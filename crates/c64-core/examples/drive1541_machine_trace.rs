// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 machine differential adapter
//
//   File:       drive1541_machine_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::disk_via::Drive1541DiskVia;
use c64_core::devices::drive1541::gcr::Drive1541SpeedZone;
use c64_core::devices::drive1541::iec_via::Drive1541IecVia;
use c64_core::devices::drive1541::machine::Drive1541Machine;
use c64_core::devices::drive1541::mechanism::{Drive1541DiskImage, Drive1541Mechanism};
use c64_core::devices::drive1541::memory::Drive1541Memory;
use c64_core::devices::iec::IecBus;
use c64_core::media::d64::D64DiskImage;
use serde::{Deserialize, Serialize};

const CONTROL_MOTOR_ON: u8 = 1 << 0;
const CONTROL_LED_ON: u8 = 1 << 1;
const CONTROL_READING: u8 = 1 << 2;
const CONTROL_BYTE_READY_ENABLED: u8 = 1 << 3;
const CONTROL_SYNC_FOUND: u8 = 1 << 4;
const CONTROL_BYTE_READY_ASSERTED: u8 = 1 << 5;
const INTERRUPT_IEC_VIA: u8 = 1 << 0;
const INTERRUPT_DISK_VIA: u8 = 1 << 1;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
    disk_bytes: Vec<u8>,
    operations: Vec<Operation>,
    rom: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum Operation {
    AdvanceHardware {
        cycles: u64,
    },
    ClockCycles {
        cycles: u64,
    },
    ExecuteInstruction,
    MechanismTick {
        cycles: u64,
    },
    MountD64,
    ReadMemory {
        address: u16,
    },
    ResetCpu,
    ResetTiming,
    SetMechanism {
        motor_on: bool,
        reading: bool,
        speed_zone: u8,
        write_data_byte: u8,
    },
    WriteMemory {
        address: u16,
        value: u8,
    },
}

#[derive(Serialize)]
struct DifferentialOutput {
    snapshots: Vec<MachineSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MachineSnapshot {
    byte_ready_edge_sequence: u64,
    byte_ready_transition_sequence: u64,
    cpu: CpuSnapshot,
    elapsed_cycles: u64,
    iec_bus_low_mask: u8,
    interrupt_pending_mask: u8,
    last_data_bus_value: u8,
    last_result: Option<u64>,
    mechanism_angular_bit_offset: usize,
    mechanism_control_mask: u8,
    mechanism_half_track: u8,
    mechanism_speed_zone: u8,
    mechanism_write_data_byte: u8,
    ram: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CpuSnapshot {
    at_instruction_boundary: bool,
    jammed: bool,
    registers: CpuRegistersSnapshot,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CpuRegistersSnapshot {
    accumulator: u8,
    index_x: u8,
    index_y: u8,
    program_counter: u16,
    stack_pointer: u8,
    status: u8,
}

fn snapshot(
    machine: &Drive1541Machine,
    iec_bus: &IecBus,
    last_result: Option<u64>,
) -> MachineSnapshot {
    let mechanism = machine.mechanism();
    let memory = machine.memory();
    let mechanism_control_mask = if mechanism.motor_on() {
        CONTROL_MOTOR_ON
    } else {
        0
    } | if mechanism.led_on() {
        CONTROL_LED_ON
    } else {
        0
    } | if mechanism.reading() {
        CONTROL_READING
    } else {
        0
    } | if mechanism.byte_ready_enabled() {
        CONTROL_BYTE_READY_ENABLED
    } else {
        0
    } | if mechanism.sync_found() {
        CONTROL_SYNC_FOUND
    } else {
        0
    } | if mechanism.byte_ready_asserted() {
        CONTROL_BYTE_READY_ASSERTED
    } else {
        0
    };
    let interrupt_pending_mask = if memory.iec_via().interrupt_pending() {
        INTERRUPT_IEC_VIA
    } else {
        0
    } | if memory.disk_via().interrupt_pending() {
        INTERRUPT_DISK_VIA
    } else {
        0
    };
    let cpu_state = machine.cpu().state();
    MachineSnapshot {
        byte_ready_edge_sequence: mechanism.byte_ready_edge_sequence(),
        byte_ready_transition_sequence: mechanism.byte_ready_transition_sequence(),
        cpu: CpuSnapshot {
            at_instruction_boundary: machine.cpu().is_at_instruction_boundary(),
            jammed: machine.cpu().is_jammed(),
            registers: CpuRegistersSnapshot {
                accumulator: cpu_state.accumulator,
                index_x: cpu_state.index_x,
                index_y: cpu_state.index_y,
                program_counter: cpu_state.program_counter,
                stack_pointer: cpu_state.stack_pointer,
                status: cpu_state.status,
            },
        },
        elapsed_cycles: machine.elapsed_cycles(),
        iec_bus_low_mask: iec_bus.state().low_mask(),
        interrupt_pending_mask,
        last_data_bus_value: memory.last_data_bus_value(),
        last_result,
        mechanism_angular_bit_offset: mechanism.angular_bit_offset(),
        mechanism_control_mask,
        mechanism_half_track: mechanism.current_half_track(),
        mechanism_speed_zone: mechanism.selected_speed_zone().get(),
        mechanism_write_data_byte: mechanism.write_data_byte(),
        ram: memory.ram().to_vec(),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input_bytes = String::new();
    io::stdin().read_to_string(&mut input_bytes)?;
    let input: DifferentialInput = serde_json::from_str(&input_bytes)?;
    let mut iec_bus = IecBus::new();
    let mut mechanism = Drive1541Mechanism::new();
    let iec_via = Drive1541IecVia::new(8, &mut iec_bus)?;
    let disk_via = Drive1541DiskVia::new(8, &mut mechanism)?;
    let memory = Drive1541Memory::new(&input.rom, iec_via, disk_via)?;
    let mut machine = Drive1541Machine::new(memory, mechanism);
    let mut snapshots = Vec::with_capacity(input.operations.len() + 1);
    snapshots.push(snapshot(&machine, &iec_bus, None));

    for operation in &input.operations {
        let last_result = match operation {
            Operation::AdvanceHardware { cycles } => {
                Some(machine.advance_hardware(&mut iec_bus, *cycles)?)
            }
            Operation::ClockCycles { cycles } => Some(machine.clock_cycles(&mut iec_bus, *cycles)?),
            Operation::ExecuteInstruction => Some(machine.execute_instruction(&mut iec_bus)?),
            Operation::MechanismTick { cycles } => {
                machine.mechanism_mut().tick(*cycles)?;
                None
            }
            Operation::MountD64 => {
                machine.mechanism_mut().mount_disk(Drive1541DiskImage::D64(
                    D64DiskImage::parse(&input.disk_bytes, false)?,
                ))?;
                None
            }
            Operation::ReadMemory { address } => {
                Some(u64::from(machine.read_memory(&mut iec_bus, *address)?))
            }
            Operation::ResetCpu => Some(machine.reset_cpu(&mut iec_bus)?),
            Operation::ResetTiming => {
                machine.reset_timing();
                None
            }
            Operation::SetMechanism {
                motor_on,
                reading,
                speed_zone,
                write_data_byte,
            } => {
                let mechanism = machine.mechanism_mut();
                mechanism.set_speed_zone(Drive1541SpeedZone::try_from(*speed_zone)?);
                mechanism.set_write_data_byte(*write_data_byte);
                mechanism.set_read_mode(*reading);
                mechanism.set_motor_on(*motor_on);
                None
            }
            Operation::WriteMemory { address, value } => {
                machine.write_memory(&mut iec_bus, *address, *value)?;
                None
            }
        };
        snapshots.push(snapshot(&machine, &iec_bus, last_result));
    }

    serde_json::to_writer(io::stdout().lock(), &DifferentialOutput { snapshots })?;
    Ok(())
}
