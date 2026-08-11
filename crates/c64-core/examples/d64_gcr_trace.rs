// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - D64 GCR 固定数据差分适配器
//
//   文件:       d64_gcr_trace.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::d64_gcr::{
    build_d64_gcr_track, decode_commodore_gcr, decode_d64_gcr_track, encode_commodore_gcr,
    encode_d64_sector_to_gcr,
};
use c64_core::media::d64::{D64DiskImage, D64ErrorCode};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SectorCase {
    data: Vec<u8>,
    error_code: u8,
    id1: u8,
    id2: u8,
    sector: u8,
    track: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
    disk_bytes: Vec<u8>,
    groups: Vec<Vec<u8>>,
    sectors: Vec<SectorCase>,
    tracks: Vec<u8>,
}

#[derive(Serialize)]
struct GroupOutput {
    decoded: Vec<u8>,
    encoded: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecodedSectorOutput {
    data: Vec<u8>,
    header_bit_offset: usize,
    id1: u8,
    id2: u8,
    sector: u8,
    track: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecodeIssueOutput {
    bit_offset: usize,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackOutput {
    bytes: Vec<u8>,
    decoded_sectors: Vec<DecodedSectorOutput>,
    issues: Vec<DecodeIssueOutput>,
    speed_zone: u8,
    track: u8,
    transfer_bits_per_second: u32,
}

#[derive(Serialize)]
struct DifferentialOutput {
    groups: Vec<GroupOutput>,
    sectors: Vec<Vec<u8>>,
    tracks: Vec<TrackOutput>,
}

fn encode_groups(groups: Vec<Vec<u8>>) -> Result<Vec<GroupOutput>, Box<dyn Error>> {
    groups
        .into_iter()
        .map(|source| {
            let encoded = encode_commodore_gcr(&source)?;
            let decoded = decode_commodore_gcr(&encoded)?;
            Ok(GroupOutput { decoded, encoded })
        })
        .collect()
}

fn encode_sectors(sectors: Vec<SectorCase>) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
    sectors
        .into_iter()
        .map(|sector_case| {
            Ok(encode_d64_sector_to_gcr(
                &sector_case.data,
                c64_core::media::d64::D64DiskId {
                    id1: sector_case.id1,
                    id2: sector_case.id2,
                },
                sector_case.track,
                sector_case.sector,
                D64ErrorCode::try_from(sector_case.error_code)?,
            )?)
        })
        .collect()
}

fn encode_tracks(
    image: &D64DiskImage,
    track_numbers: Vec<u8>,
) -> Result<Vec<TrackOutput>, Box<dyn Error>> {
    track_numbers
        .into_iter()
        .map(|track_number| {
            let track = build_d64_gcr_track(image, track_number)?;
            let decoded = decode_d64_gcr_track(&track.bytes)?;
            Ok(TrackOutput {
                bytes: track.bytes,
                decoded_sectors: decoded
                    .sectors
                    .into_iter()
                    .map(|sector| DecodedSectorOutput {
                        data: sector.data,
                        header_bit_offset: sector.header_bit_offset,
                        id1: sector.id1,
                        id2: sector.id2,
                        sector: sector.sector,
                        track: sector.track,
                    })
                    .collect(),
                issues: decoded
                    .issues
                    .into_iter()
                    .map(|issue| DecodeIssueOutput {
                        bit_offset: issue.bit_offset,
                        reason: issue.reason,
                    })
                    .collect(),
                speed_zone: track.speed_zone.get(),
                track: track.track,
                transfer_bits_per_second: track.transfer_bits_per_second,
            })
        })
        .collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input_bytes = String::new();
    io::stdin().read_to_string(&mut input_bytes)?;
    let input: DifferentialInput = serde_json::from_str(&input_bytes)?;
    let image = D64DiskImage::parse(&input.disk_bytes, false)?;
    let output = DifferentialOutput {
        groups: encode_groups(input.groups)?,
        sectors: encode_sectors(input.sectors)?,
        tracks: encode_tracks(&image, input.tracks)?,
    };
    serde_json::to_writer(io::stdout().lock(), &output)?;
    Ok(())
}
