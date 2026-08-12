// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE 1541 MOS 6522 reference replay adapter
//
//   File:       vice_1541_via.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io;

use c64_core::cpu::{Cpu6510, CpuBus};
use c64_core::devices::via::{self, Mos6522, Mos6522ControlLine};
use serde::{Deserialize, Serialize};

#[path = "support/json_line.rs"]
mod json_line;

const DRIVE_RAM_SIZE: usize = 0x0800;
const DRIVE_RAM_DECODE_END: u16 = 0x07ff;
const DRIVE_ROM_SIZE: usize = 0x4000;
const DRIVE_ROM_MIRROR_START: u16 = 0x8000;
const DRIVE_ROM_PRIMARY_START: u16 = 0xc000;
const DRIVE_DECODED_REGION_MASK: u16 = 0x1fff;
const DRIVE_IEC_VIA_START: u16 = 0x1800;
const DRIVE_IEC_VIA_END: u16 = 0x1bff;
const SAMPLE_BUFFER_ADDRESS: usize = 0x0700;
const REFERENCE_SAMPLE_COUNT: usize = 0x0100;
const MAXIMUM_REPLAY_INSTRUCTIONS: usize = 2_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplayRequest {
    program: Vec<u8>,
    stop_address: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayResult {
    samples: Vec<u8>,
}

struct ReferenceDriveBus {
    ram: [u8; DRIVE_RAM_SIZE],
    rom: [u8; DRIVE_ROM_SIZE],
    iec_via: Mos6522,
    data_bus: u8,
    clock_peripherals: bool,
}

impl ReferenceDriveBus {
    fn new(program: &[u8]) -> Result<Self, &'static str> {
        let program_end = program.len();
        if program_end > DRIVE_ROM_SIZE {
            return Err("1541 VIA replay program does not fit in the ROM image");
        }

        let mut rom = [0; DRIVE_ROM_SIZE];
        rom[..program_end].copy_from_slice(program);
        rom[0x3ffc] = DRIVE_ROM_PRIMARY_START.to_le_bytes()[0];
        rom[0x3ffd] = DRIVE_ROM_PRIMARY_START.to_le_bytes()[1];
        Ok(Self {
            ram: [0; DRIVE_RAM_SIZE],
            rom,
            iec_via: Mos6522::new(),
            data_bus: u8::MAX,
            clock_peripherals: false,
        })
    }

    fn initialize_reference_via(&mut self) {
        // The VICE viavarious receiver holds IEC DATA and CLOCK low while ATN is
        // released. The 1541 inverter network therefore presents $1f to VIA1 PB.
        self.iec_via.set_port_b_external_inputs(0x1f);
        self.iec_via
            .signal_control_line(Mos6522ControlLine::Ca1, false);
        self.iec_via.write(
            u16::from(via::register::AUXILIARY_CONTROL),
            via::auxiliary_control::TIMER_2_COUNT_PORT_B_6,
        );
        self.iec_via
            .write(u16::from(via::register::PERIPHERAL_CONTROL), 0x00);
        self.iec_via
            .write(u16::from(via::register::SHIFT_REGISTER), 0xa5);
        self.iec_via.write(
            u16::from(via::register::INTERRUPT_ENABLE),
            via::interrupt::SOURCE_MASK,
        );
        self.iec_via.write(
            u16::from(via::register::INTERRUPT_FLAGS),
            via::interrupt::SOURCE_MASK,
        );
        self.iec_via
            .write(u16::from(via::register::TIMER_1_COUNTER_LOW), 0x00);
        self.iec_via
            .write(u16::from(via::register::TIMER_1_COUNTER_HIGH), 0x00);
        self.iec_via
            .write(u16::from(via::register::TIMER_1_LATCH_LOW), 0x00);
        self.iec_via
            .write(u16::from(via::register::TIMER_1_LATCH_HIGH), 0x00);
        self.iec_via
            .write(u16::from(via::register::TIMER_2_COUNTER_LOW), 0x00);
        self.iec_via
            .write(u16::from(via::register::TIMER_2_COUNTER_HIGH), 0x00);

        // Real hardware has already allowed the zero-valued T1 to time out once
        // before the captured fragment begins.
        self.iec_via.clock_cycles(2);
        self.clock_peripherals = true;
    }

    fn samples(&self) -> Vec<u8> {
        self.ram[SAMPLE_BUFFER_ADDRESS..SAMPLE_BUFFER_ADDRESS + REFERENCE_SAMPLE_COUNT].to_vec()
    }

    fn begin_cpu_bus_cycle(&mut self) {
        if self.clock_peripherals {
            self.iec_via.clock_cycle();
        }
    }

    fn read_decoded(&mut self, address: u16) -> u8 {
        if address >= DRIVE_ROM_MIRROR_START {
            let offset = usize::from(address - DRIVE_ROM_MIRROR_START) & (DRIVE_ROM_SIZE - 1);
            return self.rom[offset];
        }

        let decoded = address & DRIVE_DECODED_REGION_MASK;
        if decoded <= DRIVE_RAM_DECODE_END {
            return self.ram[usize::from(decoded)];
        }
        if (DRIVE_IEC_VIA_START..=DRIVE_IEC_VIA_END).contains(&decoded) {
            return self.iec_via.read(decoded);
        }
        self.data_bus
    }

    fn write_decoded(&mut self, address: u16, value: u8) {
        if address >= DRIVE_ROM_MIRROR_START {
            return;
        }

        let decoded = address & DRIVE_DECODED_REGION_MASK;
        if decoded <= DRIVE_RAM_DECODE_END {
            self.ram[usize::from(decoded)] = value;
        } else if (DRIVE_IEC_VIA_START..=DRIVE_IEC_VIA_END).contains(&decoded) {
            self.iec_via.write(decoded, value);
        }
    }
}

impl CpuBus for ReferenceDriveBus {
    fn read(&mut self, address: u16) -> u8 {
        self.begin_cpu_bus_cycle();
        let value = self.read_decoded(address);
        self.data_bus = value;
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        self.begin_cpu_bus_cycle();
        self.data_bus = value;
        self.write_decoded(address, value);
    }
}

fn replay(request: &ReplayRequest) -> Result<ReplayResult, Box<dyn Error>> {
    let mut bus = ReferenceDriveBus::new(&request.program)?;
    let mut cpu = Cpu6510::new();
    cpu.reset(&mut bus);
    bus.initialize_reference_via();

    let mut executed_instructions = 0;
    while cpu.state().program_counter != request.stop_address
        && executed_instructions < MAXIMUM_REPLAY_INSTRUCTIONS
    {
        loop {
            if cpu.clock_cycle(&mut bus)? {
                break;
            }
        }
        executed_instructions += 1;
    }
    if cpu.state().program_counter != request.stop_address {
        return Err(format!(
            "1541 VIA replay did not stop at ${:04x} within {} instructions",
            request.stop_address, MAXIMUM_REPLAY_INSTRUCTIONS
        )
        .into());
    }

    Ok(ReplayResult {
        samples: bus.samples(),
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let requests: Vec<ReplayRequest> = json_line::read_stdin_json_line()?;
    let results = requests.iter().map(replay).collect::<Result<Vec<_>, _>>()?;
    serde_json::to_writer(io::stdout().lock(), &results)?;
    Ok(())
}
