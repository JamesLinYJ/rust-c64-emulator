// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 disk-side VIA differential adapter
//
//   File:       drive1541_disk_via_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::disk_via::Drive1541DiskVia;
use c64_core::devices::drive1541::mechanism::{Drive1541DiskImage, Drive1541Mechanism};
use c64_core::media::d64::D64DiskImage;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
    disk_bytes: Vec<u8>,
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum Operation {
    Clock { cycles: u32 },
    MechanismTick { cycles: u64 },
    MountD64 { write_protected: bool },
    Read { address: u16 },
    Reset,
    Write { address: u16, value: u8 },
}

#[derive(Serialize)]
struct DifferentialOutput {
    snapshots: Vec<DiskViaSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiskViaSnapshot {
    device_number: u8,
    last_read: Option<u8>,
    mechanism: MechanismSnapshot,
    via: ViaSnapshot,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MechanismSnapshot {
    byte_ready: ByteReadySnapshot,
    control: ControlSnapshot,
    gcr: GcrSnapshot,
    media: MediaSnapshot,
    position: PositionSnapshot,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ByteReadySnapshot {
    asserted: bool,
    edge_sequence: u64,
    enabled: bool,
    transition_sequence: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ControlSnapshot {
    led_on: bool,
    motor_on: bool,
    reading: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GcrSnapshot {
    data_byte: u8,
    sync_found: bool,
    write_data_byte: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaSnapshot {
    dirty_half_tracks: Vec<u8>,
    disk_present: bool,
    write_protect_sensor_active: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PositionSnapshot {
    angular_bit_offset: usize,
    current_half_track: u8,
    selected_speed_zone: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ViaSnapshot {
    interrupt_pending: bool,
    port_a_output_pins: u8,
    port_b_data_direction: u8,
    port_b_output_latch: u8,
    port_b_output_pins: u8,
}

fn snapshot(
    via: &Drive1541DiskVia,
    mechanism: &Drive1541Mechanism,
    last_read: Option<u8>,
) -> DiskViaSnapshot {
    DiskViaSnapshot {
        device_number: via.device_number(),
        last_read,
        mechanism: MechanismSnapshot {
            byte_ready: ByteReadySnapshot {
                asserted: mechanism.byte_ready_asserted(),
                edge_sequence: mechanism.byte_ready_edge_sequence(),
                enabled: mechanism.byte_ready_enabled(),
                transition_sequence: mechanism.byte_ready_transition_sequence(),
            },
            control: ControlSnapshot {
                led_on: mechanism.led_on(),
                motor_on: mechanism.motor_on(),
                reading: mechanism.reading(),
            },
            gcr: GcrSnapshot {
                data_byte: mechanism.data_byte(),
                sync_found: mechanism.sync_found(),
                write_data_byte: mechanism.write_data_byte(),
            },
            media: MediaSnapshot {
                dirty_half_tracks: mechanism.dirty_half_tracks(),
                disk_present: mechanism.disk_present(),
                write_protect_sensor_active: mechanism.write_protect_sensor_active(),
            },
            position: PositionSnapshot {
                angular_bit_offset: mechanism.angular_bit_offset(),
                current_half_track: mechanism.current_half_track(),
                selected_speed_zone: mechanism.selected_speed_zone().get(),
            },
        },
        via: ViaSnapshot {
            interrupt_pending: via.interrupt_pending(),
            port_a_output_pins: via.via().port_a_output_pins(),
            port_b_data_direction: via.via().port_b_data_direction(),
            port_b_output_latch: via.via().port_b_output_latch(),
            port_b_output_pins: via.via().port_b_output_pins(),
        },
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input_bytes = String::new();
    io::stdin().read_to_string(&mut input_bytes)?;
    let input: DifferentialInput = serde_json::from_str(&input_bytes)?;
    let mut mechanism = Drive1541Mechanism::new();
    let mut via = Drive1541DiskVia::new(8, &mut mechanism)?;
    let mut snapshots = Vec::with_capacity(input.operations.len() + 1);
    snapshots.push(snapshot(&via, &mechanism, None));

    for operation in &input.operations {
        let last_read = match operation {
            Operation::Clock { cycles } => {
                for _ in 0..*cycles {
                    via.clock_cycle(&mut mechanism)?;
                }
                None
            }
            Operation::MechanismTick { cycles } => {
                mechanism.tick(*cycles)?;
                None
            }
            Operation::MountD64 { write_protected } => {
                mechanism.mount_disk(Drive1541DiskImage::D64(D64DiskImage::parse(
                    &input.disk_bytes,
                    *write_protected,
                )?))?;
                None
            }
            Operation::Read { address } => Some(via.read(&mut mechanism, *address)?),
            Operation::Reset => {
                via.reset(&mut mechanism);
                None
            }
            Operation::Write { address, value } => {
                via.write(&mut mechanism, *address, *value)?;
                None
            }
        };
        snapshots.push(snapshot(&via, &mechanism, last_read));
    }

    serde_json::to_writer(io::stdout().lock(), &DifferentialOutput { snapshots })?;
    Ok(())
}
