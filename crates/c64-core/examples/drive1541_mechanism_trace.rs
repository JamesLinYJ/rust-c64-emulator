// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 mechanism differential adapter
//
//   File:       drive1541_mechanism_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::gcr::Drive1541SpeedZone;
use c64_core::devices::drive1541::mechanism::{
    Drive1541ControlState, Drive1541DiskImage, Drive1541Mechanism, Drive1541StepperPhase,
};
use c64_core::media::d64::D64DiskImage;
use c64_core::media::g64::G64DiskImage;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
    media: Vec<MediaInput>,
    operations: Vec<Operation>,
    raw_track_probes: Vec<RawTrackProbe>,
}

#[derive(Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum MediaInput {
    D64 {
        bytes: Vec<u8>,
        write_protected: bool,
    },
    G64 {
        bytes: Vec<u8>,
        write_protected: bool,
    },
}

impl MediaInput {
    fn parse(&self) -> Result<Drive1541DiskImage, Box<dyn Error>> {
        Ok(match self {
            Self::D64 {
                bytes,
                write_protected,
            } => Drive1541DiskImage::D64(D64DiskImage::parse(bytes, *write_protected)?),
            Self::G64 {
                bytes,
                write_protected,
            } => Drive1541DiskImage::G64(G64DiskImage::parse(bytes, *write_protected)?),
        })
    }
}

#[derive(Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum Operation {
    AcknowledgeByteReady,
    ApplyControl {
        led_on: bool,
        motor_on: bool,
        speed_zone: u8,
        stepper_phase: u8,
    },
    CommitD64,
    Eject,
    Mount {
        media_index: usize,
    },
    ResetElectronics,
    SetByteReadyEnabled {
        enabled: bool,
    },
    SetLedOn {
        led_on: bool,
    },
    SetMotorOn {
        motor_on: bool,
    },
    SetReadMode {
        reading: bool,
    },
    SetSpeedZone {
        speed_zone: u8,
    },
    SetWriteDataByte {
        value: u8,
    },
    Tick {
        cycles: u64,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTrackProbe {
    byte_count: usize,
    half_track: u8,
}

#[derive(Serialize)]
struct DifferentialOutput {
    snapshots: Vec<MechanismSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MechanismSnapshot {
    angular_bit_offset: usize,
    byte_ready: ByteReadySnapshot,
    control: ControlSnapshot,
    current_half_track: u8,
    data_byte: u8,
    media: MediaSnapshot,
    selected_speed_zone: u8,
    sync_found: bool,
    write_data_byte: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ByteReadySnapshot {
    byte_ready_asserted: bool,
    byte_ready_edge_sequence: u64,
    byte_ready_enabled: bool,
    byte_ready_transition_sequence: u64,
    last_byte_ready_edge_data: u8,
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
struct MediaSnapshot {
    dirty_half_tracks: Vec<u8>,
    disk_present: bool,
    kind: Option<&'static str>,
    raw_tracks: Vec<RawTrackSnapshot>,
    write_protect_sensor_active: bool,
    write_protected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RawTrackSnapshot {
    bytes: Option<Vec<u8>>,
    half_track: u8,
}

fn snapshot(
    mechanism: &mut Drive1541Mechanism,
    probes: &[RawTrackProbe],
) -> Result<MechanismSnapshot, Box<dyn Error>> {
    let media_kind = match mechanism.mounted_disk() {
        Some(Drive1541DiskImage::D64(_)) => Some("d64"),
        Some(Drive1541DiskImage::G64(_)) => Some("g64"),
        None => None,
    };
    let raw_tracks = probes
        .iter()
        .map(|probe| {
            let bytes = if mechanism.disk_present() {
                Some(
                    mechanism
                        .read_raw_half_track(probe.half_track)?
                        .into_iter()
                        .take(probe.byte_count)
                        .collect(),
                )
            } else {
                None
            };
            Ok(RawTrackSnapshot {
                bytes,
                half_track: probe.half_track,
            })
        })
        .collect::<Result<Vec<_>, c64_core::devices::drive1541::mechanism::Drive1541MechanismError>>()?;
    Ok(MechanismSnapshot {
        angular_bit_offset: mechanism.angular_bit_offset(),
        byte_ready: ByteReadySnapshot {
            byte_ready_asserted: mechanism.byte_ready_asserted(),
            byte_ready_edge_sequence: mechanism.byte_ready_edge_sequence(),
            byte_ready_enabled: mechanism.byte_ready_enabled(),
            byte_ready_transition_sequence: mechanism.byte_ready_transition_sequence(),
            last_byte_ready_edge_data: mechanism.last_byte_ready_edge_data(),
        },
        control: ControlSnapshot {
            led_on: mechanism.led_on(),
            motor_on: mechanism.motor_on(),
            reading: mechanism.reading(),
        },
        current_half_track: mechanism.current_half_track(),
        data_byte: mechanism.data_byte(),
        media: MediaSnapshot {
            dirty_half_tracks: mechanism.dirty_half_tracks(),
            disk_present: mechanism.disk_present(),
            kind: media_kind,
            raw_tracks,
            write_protect_sensor_active: mechanism.write_protect_sensor_active(),
            write_protected: mechanism.write_protected(),
        },
        selected_speed_zone: mechanism.selected_speed_zone().get(),
        sync_found: mechanism.sync_found(),
        write_data_byte: mechanism.write_data_byte(),
    })
}

fn apply_operation(
    mechanism: &mut Drive1541Mechanism,
    media: &[MediaInput],
    operation: &Operation,
) -> Result<(), Box<dyn Error>> {
    match operation {
        Operation::AcknowledgeByteReady => mechanism.acknowledge_byte_ready(),
        Operation::ApplyControl {
            led_on,
            motor_on,
            speed_zone,
            stepper_phase,
        } => mechanism.apply_control_state(Drive1541ControlState {
            led_on: *led_on,
            motor_on: *motor_on,
            speed_zone: Drive1541SpeedZone::try_from(*speed_zone)?,
            stepper_phase: Drive1541StepperPhase::try_from(*stepper_phase)?,
        })?,
        Operation::CommitD64 => {
            mechanism.commit_raw_track_writes_to_d64()?;
        }
        Operation::Eject => {
            mechanism.eject_disk()?;
        }
        Operation::Mount { media_index } => {
            let input = media.get(*media_index).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid mechanism media index")
            })?;
            mechanism.mount_disk(input.parse()?)?;
        }
        Operation::ResetElectronics => mechanism.reset_electronics(),
        Operation::SetByteReadyEnabled { enabled } => {
            mechanism.set_byte_ready_enabled(*enabled);
        }
        Operation::SetLedOn { led_on } => mechanism.set_led_on(*led_on),
        Operation::SetMotorOn { motor_on } => mechanism.set_motor_on(*motor_on),
        Operation::SetReadMode { reading } => mechanism.set_read_mode(*reading),
        Operation::SetSpeedZone { speed_zone } => {
            mechanism.set_speed_zone(Drive1541SpeedZone::try_from(*speed_zone)?);
        }
        Operation::SetWriteDataByte { value } => mechanism.set_write_data_byte(*value),
        Operation::Tick { cycles } => mechanism.tick(*cycles)?,
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input_bytes = String::new();
    io::stdin().read_to_string(&mut input_bytes)?;
    let input: DifferentialInput = serde_json::from_str(&input_bytes)?;
    let mut mechanism = Drive1541Mechanism::new();
    let mut snapshots = Vec::with_capacity(input.operations.len() + 1);
    snapshots.push(snapshot(&mut mechanism, &input.raw_track_probes)?);
    for operation in &input.operations {
        apply_operation(&mut mechanism, &input.media, operation)?;
        snapshots.push(snapshot(&mut mechanism, &input.raw_track_probes)?);
    }
    serde_json::to_writer(io::stdout().lock(), &DifferentialOutput { snapshots })?;
    Ok(())
}
