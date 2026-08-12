// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 chipset integration differential adapter
//
//   File:       drive1541_chipset_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::gcr::Drive1541SpeedZone;
use c64_core::devices::drive1541::mechanism::{
    Drive1541ControlState, Drive1541DiskImage, Drive1541StepperPhase,
};
use c64_core::devices::vic::VicMemoryBus;
use c64_core::media::d64::D64DiskImage;
use c64_core::{C64BusDevices, C64Chipset, VideoStandard};
use serde::{Deserialize, Serialize};

const MECHANISM_DISK_PRESENT: u8 = 1 << 0;
const MECHANISM_LED_ON: u8 = 1 << 1;
const MECHANISM_MOTOR_ON: u8 = 1 << 2;
const MECHANISM_READING: u8 = 1 << 3;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
    device_number: u8,
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
    ClockSystemCycles {
        cycles: u64,
    },
    MountD64,
    ResetBoard,
    SetMechanism {
        led_on: bool,
        motor_on: bool,
        speed_zone: u8,
        stepper_phase: u8,
    },
    WriteCia2 {
        address: u16,
        value: u8,
    },
}

#[derive(Serialize)]
struct DifferentialOutput {
    snapshots: Vec<ChipsetSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChipsetSnapshot {
    cia2_port_a: u8,
    clock: ClockSnapshot,
    cpu: CpuSnapshot,
    device_number: u8,
    elapsed_cycles: u64,
    iec_bus_low_mask: u8,
    last_data_bus_value: u8,
    last_result: Option<u64>,
    mechanism: MechanismSnapshot,
    ram: Vec<u8>,
    reset_assertion_sequence: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClockSnapshot {
    lead_cycles: i128,
    phase_remainder: u64,
    target_cycles: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CpuSnapshot {
    accumulator: u8,
    at_instruction_boundary: bool,
    index_x: u8,
    index_y: u8,
    jammed: bool,
    program_counter: u16,
    stack_pointer: u8,
    status: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MechanismSnapshot {
    angular_bit_offset: usize,
    control_mask: u8,
    half_track: u8,
    speed_zone: u8,
}

struct ZeroVicMemory;

impl VicMemoryBus for ZeroVicMemory {
    fn cpu_data_bus_value(&self) -> u8 {
        u8::MAX
    }

    fn read_vic_byte(&mut self, _address_in_bank: u16) -> u8 {
        0
    }

    fn read_vic_color(&mut self, _index: u16) -> u8 {
        0
    }
}

fn snapshot(chipset: &mut C64Chipset, last_result: Option<u64>) -> ChipsetSnapshot {
    let cia2_port_a = chipset.read_io(0xdd00, u8::MAX);
    let iec_bus_low_mask = chipset.iec_bus().state().low_mask();
    let reset_assertion_sequence = chipset.iec_bus().reset_assertion_sequence();
    let drive = chipset
        .drive1541()
        .expect("the differential drive must stay attached");
    let machine = drive.machine();
    let mechanism = machine.mechanism();
    let cpu = machine.cpu();
    let cpu_state = cpu.state();
    ChipsetSnapshot {
        cia2_port_a,
        clock: ClockSnapshot {
            lead_cycles: drive.clock().lead_cycles(machine),
            phase_remainder: drive.clock().phase_remainder(),
            target_cycles: drive.clock().target_cycles(),
        },
        cpu: CpuSnapshot {
            accumulator: cpu_state.accumulator,
            at_instruction_boundary: cpu.is_at_instruction_boundary(),
            index_x: cpu_state.index_x,
            index_y: cpu_state.index_y,
            jammed: cpu.is_jammed(),
            program_counter: cpu_state.program_counter,
            stack_pointer: cpu_state.stack_pointer,
            status: cpu_state.status,
        },
        device_number: drive.device_number(),
        elapsed_cycles: machine.elapsed_cycles(),
        iec_bus_low_mask,
        last_data_bus_value: machine.memory().last_data_bus_value(),
        last_result,
        mechanism: MechanismSnapshot {
            angular_bit_offset: mechanism.angular_bit_offset(),
            control_mask: if mechanism.disk_present() {
                MECHANISM_DISK_PRESENT
            } else {
                0
            } | if mechanism.led_on() {
                MECHANISM_LED_ON
            } else {
                0
            } | if mechanism.motor_on() {
                MECHANISM_MOTOR_ON
            } else {
                0
            } | if mechanism.reading() {
                MECHANISM_READING
            } else {
                0
            },
            half_track: mechanism.current_half_track(),
            speed_zone: mechanism.selected_speed_zone().get(),
        },
        ram: machine.memory().ram().to_vec(),
        reset_assertion_sequence,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input_bytes = String::new();
    io::stdin().read_to_string(&mut input_bytes)?;
    let input: DifferentialInput = serde_json::from_str(&input_bytes)?;
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    chipset.attach_drive1541(input.device_number, &input.rom)?;
    let mut vic_memory = ZeroVicMemory;
    let mut snapshots = Vec::with_capacity(input.operations.len() + 1);
    snapshots.push(snapshot(&mut chipset, None));

    for operation in &input.operations {
        let elapsed_before = chipset
            .drive1541()
            .expect("the differential drive must stay attached")
            .machine()
            .elapsed_cycles();
        let last_result = match operation {
            Operation::ClockSystemCycles { cycles } => {
                for _ in 0..*cycles {
                    chipset.clock_system_cycle(&mut vic_memory)?;
                }
                let elapsed_after = chipset
                    .drive1541()
                    .expect("the differential drive must stay attached")
                    .machine()
                    .elapsed_cycles();
                Some(elapsed_after - elapsed_before)
            }
            Operation::MountD64 => {
                let image = D64DiskImage::parse(&input.disk_bytes, false)?;
                chipset
                    .drive1541_mut()
                    .expect("the differential drive must stay attached")
                    .mount_disk(Drive1541DiskImage::D64(image))?;
                None
            }
            Operation::ResetBoard => {
                chipset.reset()?;
                None
            }
            Operation::SetMechanism {
                led_on,
                motor_on,
                speed_zone,
                stepper_phase,
            } => {
                chipset
                    .drive1541_mut()
                    .expect("the differential drive must stay attached")
                    .machine_mut()
                    .mechanism_mut()
                    .apply_control_state(Drive1541ControlState {
                        led_on: *led_on,
                        motor_on: *motor_on,
                        speed_zone: Drive1541SpeedZone::try_from(*speed_zone)?,
                        stepper_phase: Drive1541StepperPhase::try_from(*stepper_phase)?,
                    })?;
                None
            }
            Operation::WriteCia2 { address, value } => {
                chipset.write_io(*address, *value);
                None
            }
        };
        snapshots.push(snapshot(&mut chipset, last_result));
    }

    serde_json::to_writer(io::stdout().lock(), &DifferentialOutput { snapshots })?;
    Ok(())
}
