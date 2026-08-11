// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 memory decoder differential adapter
//
//   File:       drive1541_memory_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::disk_via::Drive1541DiskVia;
use c64_core::devices::drive1541::iec_via::Drive1541IecVia;
use c64_core::devices::drive1541::mechanism::Drive1541Mechanism;
use c64_core::devices::drive1541::memory::Drive1541Memory;
use c64_core::devices::iec::IecBus;
use serde::{Deserialize, Serialize};

const CONTROL_MOTOR_ON: u8 = 1 << 0;
const CONTROL_LED_ON: u8 = 1 << 1;
const CONTROL_READING: u8 = 1 << 2;
const CONTROL_BYTE_READY_ENABLED: u8 = 1 << 3;
const CONTROL_SYNC_FOUND: u8 = 1 << 4;
const CONTROL_WRITE_PROTECT_SENSOR: u8 = 1 << 5;
const INTERRUPT_IEC_VIA: u8 = 1 << 0;
const INTERRUPT_DISK_VIA: u8 = 1 << 1;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
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
    Read { address: u16 },
    ReadStack { stack_pointer: u8 },
    ReadWord { address: u16 },
    Reset,
    Write { address: u16, value: u8 },
    WriteStack { stack_pointer: u8, value: u8 },
    WriteWord { address: u16, value: u16 },
}

#[derive(Serialize)]
struct DifferentialOutput {
    snapshots: Vec<MemorySnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemorySnapshot {
    disk_port_a_output_pins: u8,
    disk_port_b_data_direction: u8,
    disk_port_b_output_latch: u8,
    iec_bus_low_mask: u8,
    iec_port_b_data_direction: u8,
    iec_port_b_output_latch: u8,
    interrupt_pending_mask: u8,
    last_data_bus_value: u8,
    last_read: Option<u16>,
    mechanism_control_mask: u8,
    mechanism_half_track: u8,
    mechanism_speed_zone: u8,
    mechanism_write_data_byte: u8,
    ram: Vec<u8>,
}

fn snapshot(
    memory: &Drive1541Memory,
    mechanism: &Drive1541Mechanism,
    iec_bus: &IecBus,
    last_read: Option<u16>,
) -> MemorySnapshot {
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
    } | if mechanism.write_protect_sensor_active() {
        CONTROL_WRITE_PROTECT_SENSOR
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
    MemorySnapshot {
        disk_port_a_output_pins: memory.disk_via().via().port_a_output_pins(),
        disk_port_b_data_direction: memory.disk_via().via().port_b_data_direction(),
        disk_port_b_output_latch: memory.disk_via().via().port_b_output_latch(),
        iec_bus_low_mask: iec_bus.state().low_mask(),
        iec_port_b_data_direction: memory.iec_via().via().port_b_data_direction(),
        iec_port_b_output_latch: memory.iec_via().via().port_b_output_latch(),
        interrupt_pending_mask,
        last_data_bus_value: memory.last_data_bus_value(),
        last_read,
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
    let mut memory = Drive1541Memory::new(&input.rom, iec_via, disk_via)?;
    let mut snapshots = Vec::with_capacity(input.operations.len() + 1);
    snapshots.push(snapshot(&memory, &mechanism, &iec_bus, None));

    for operation in &input.operations {
        let last_read = match operation {
            Operation::Read { address } => Some(u16::from(memory.read(
                &mut iec_bus,
                &mut mechanism,
                *address,
            )?)),
            Operation::ReadStack { stack_pointer } => Some(u16::from(memory.read_stack(
                &mut iec_bus,
                &mut mechanism,
                *stack_pointer,
            )?)),
            Operation::ReadWord { address } => {
                Some(memory.read_word(&mut iec_bus, &mut mechanism, *address)?)
            }
            Operation::Reset => {
                memory.reset_hardware(&mut iec_bus, &mut mechanism);
                None
            }
            Operation::Write { address, value } => {
                memory.write(&mut iec_bus, &mut mechanism, *address, *value)?;
                None
            }
            Operation::WriteStack {
                stack_pointer,
                value,
            } => {
                memory.write_stack(&mut iec_bus, &mut mechanism, *stack_pointer, *value)?;
                None
            }
            Operation::WriteWord { address, value } => {
                memory.write_word(&mut iec_bus, &mut mechanism, *address, *value)?;
                None
            }
        };
        snapshots.push(snapshot(&memory, &mechanism, &iec_bus, last_read));
    }

    serde_json::to_writer(io::stdout().lock(), &DifferentialOutput { snapshots })?;
    Ok(())
}
