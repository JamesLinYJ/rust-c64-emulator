// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - G64 fixed-operation differential adapter
//
//   File:       g64_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::media::g64::{G64DiskImage, G64SpeedMap, G64SpeedZone};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
    image_bytes: Vec<u8>,
    operations: Vec<Operation>,
    probes: Vec<Probe>,
}

#[derive(Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum Operation {
    SetHalfTrack {
        data: Vec<u8>,
        half_track: u8,
        speed_map: Option<SpeedMapInput>,
    },
    WriteHalfTrackByte {
        byte_index: usize,
        half_track: u8,
        speed_zone: Option<u8>,
        value: u8,
    },
    SetWriteProtected {
        write_protected: bool,
    },
}

#[derive(Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum SpeedMapInput {
    Constant { zone: u8 },
    Variable { packed_zones: Vec<u8> },
}

impl SpeedMapInput {
    fn into_speed_map(self) -> Result<G64SpeedMap, Box<dyn Error>> {
        Ok(match self {
            Self::Constant { zone } => G64SpeedMap::Constant(G64SpeedZone::try_from(zone)?),
            Self::Variable { packed_zones } => G64SpeedMap::Variable(packed_zones),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Probe {
    byte_indices: Vec<usize>,
    half_track: u8,
}

#[derive(Serialize)]
struct DifferentialOutput {
    snapshots: Vec<ImageSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageSnapshot {
    half_track_count: u8,
    last_half_track: u8,
    maximum_track_length: usize,
    probes: Vec<ProbeSnapshot>,
    serialized_bytes: Vec<u8>,
    write_protected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeSnapshot {
    bytes: Option<Vec<u8>>,
    half_track: u8,
    speed_zones: Vec<u8>,
}

fn snapshot(image: &G64DiskImage, probes: &[Probe]) -> Result<ImageSnapshot, Box<dyn Error>> {
    let probes = probes
        .iter()
        .map(|probe| {
            let bytes = image
                .half_track(probe.half_track)?
                .map(|track| track.bytes().to_vec());
            let speed_zones = probe
                .byte_indices
                .iter()
                .copied()
                .map(|byte_index| {
                    Ok(image
                        .speed_zone_at_byte(probe.half_track, byte_index)?
                        .get())
                })
                .collect::<Result<Vec<_>, c64_core::media::g64::G64ImageError>>()?;
            Ok(ProbeSnapshot {
                bytes,
                half_track: probe.half_track,
                speed_zones,
            })
        })
        .collect::<Result<Vec<_>, c64_core::media::g64::G64ImageError>>()?;
    Ok(ImageSnapshot {
        half_track_count: image.half_track_count(),
        last_half_track: image.last_half_track(),
        maximum_track_length: image.maximum_track_length(),
        probes,
        serialized_bytes: image.to_bytes()?,
        write_protected: image.write_protected(),
    })
}

fn apply_operation(image: &mut G64DiskImage, operation: Operation) -> Result<(), Box<dyn Error>> {
    match operation {
        Operation::SetHalfTrack {
            data,
            half_track,
            speed_map,
        } => image.set_half_track(
            half_track,
            &data,
            speed_map.map(SpeedMapInput::into_speed_map).transpose()?,
        )?,
        Operation::WriteHalfTrackByte {
            byte_index,
            half_track,
            speed_zone,
            value,
        } => image.write_half_track_byte(
            half_track,
            byte_index,
            value,
            speed_zone.map(G64SpeedZone::try_from).transpose()?,
        )?,
        Operation::SetWriteProtected { write_protected } => {
            image.set_write_protected(write_protected);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input_bytes = String::new();
    io::stdin().read_to_string(&mut input_bytes)?;
    let input: DifferentialInput = serde_json::from_str(&input_bytes)?;
    let mut image = G64DiskImage::parse(&input.image_bytes, false)?;
    let mut snapshots = Vec::with_capacity(input.operations.len() + 1);
    snapshots.push(snapshot(&image, &input.probes)?);
    for operation in input.operations {
        apply_operation(&mut image, operation)?;
        snapshots.push(snapshot(&image, &input.probes)?);
    }
    serde_json::to_writer(io::stdout().lock(), &DifferentialOutput { snapshots })?;
    Ok(())
}
