// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore D64 GCR codec
//
//   File:       devices/drive1541/d64_gcr.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::media::d64::{
    D64DiskId, D64DiskImage, D64ErrorCode, D64ImageError, d64_sectors_on_track,
};

use super::gcr::Drive1541SpeedZone;

const GCR_NIBBLE_TO_CODE: [u8; 16] = [
    0x0a, 0x0b, 0x12, 0x13, 0x0e, 0x0f, 0x16, 0x17, 0x09, 0x19, 0x1a, 0x1b, 0x0d, 0x1d, 0x1e, 0x15,
];
const GCR_CODE_TO_NIBBLE: [i8; 32] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, 8, 0, 1, -1, 12, 4, 5, -1, -1, 2, 3, -1, 15, 6, 7, -1, 9,
    10, 11, -1, 13, 14, -1,
];
pub const D64_GCR_RAW_TRACK_SIZE: [usize; 4] = [6_250, 6_666, 7_142, 7_692];
pub const D64_GCR_TRANSFER_BITS_PER_SECOND: [u32; 4] = [250_000, 266_667, 285_714, 307_692];
const DATA_GAP_BY_SPEED_ZONE: [usize; 4] = [9, 12, 17, 8];
const DECODED_GROUP_SIZE: usize = 4;
const ENCODED_GROUP_SIZE: usize = 5;
const ENCODED_HEADER_AND_DATA_SIZE: usize = 335;
const DATA_BLOCK_DECODED_SIZE: usize = 260;
const HEADER_GAP_SIZE: usize = 9;
const SYNC_LENGTH: usize = 5;
const SYNC_DETECT_BITS: usize = 10;
const FILL_BYTE: u8 = 0x55;
const SYNC_BYTE: u8 = 0xff;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum D64GcrError {
    Image(D64ImageError),
    InvalidDecodedLength(usize),
    InvalidEncodedLength(usize),
    InvalidGcrCode(u8),
    InvalidTrack(u8),
    UnrepresentableErrorCode(D64ErrorCode),
    EmptyTrack,
    TrackCapacity { track: u8, capacity: usize },
    InternalAssembly,
}

impl fmt::Display for D64GcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => error.fmt(formatter),
            Self::InvalidDecodedLength(length) => write!(
                formatter,
                "Commodore GCR decoded input length {length} is not divisible by four"
            ),
            Self::InvalidEncodedLength(length) => write!(
                formatter,
                "Commodore GCR encoded input length {length} is not divisible by five"
            ),
            Self::InvalidGcrCode(code) => write!(formatter, "invalid GCR code {code:02x}"),
            Self::InvalidTrack(track) => write!(formatter, "D64 GCR track {track} is invalid"),
            Self::UnrepresentableErrorCode(code) => {
                write!(
                    formatter,
                    "D64 error code {code:?} has no passive GCR representation"
                )
            }
            Self::EmptyTrack => write!(formatter, "cannot decode an empty GCR track"),
            Self::TrackCapacity { track, capacity } => write!(
                formatter,
                "D64 track {track} exceeds its {capacity}-byte speed-zone capacity"
            ),
            Self::InternalAssembly => {
                write!(formatter, "GCR assembly ended at an invalid boundary")
            }
        }
    }
}

impl std::error::Error for D64GcrError {}

impl From<D64ImageError> for D64GcrError {
    fn from(error: D64ImageError) -> Self {
        Self::Image(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct D64GcrTrack {
    pub bytes: Vec<u8>,
    pub speed_zone: Drive1541SpeedZone,
    pub track: u8,
    pub transfer_bits_per_second: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedD64GcrSector {
    pub data: Vec<u8>,
    pub header_bit_offset: usize,
    pub id1: u8,
    pub id2: u8,
    pub sector: u8,
    pub track: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct D64GcrDecodeIssue {
    pub bit_offset: usize,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct D64GcrTrackDecodeResult {
    pub issues: Vec<D64GcrDecodeIssue>,
    pub sectors: Vec<DecodedD64GcrSector>,
}

/// Encode groups of four bytes into five-byte Commodore GCR groups.
///
/// # Errors
///
/// Rejects input whose length is not divisible by four.
pub fn encode_commodore_gcr(source: &[u8]) -> Result<Vec<u8>, D64GcrError> {
    if !source.len().is_multiple_of(DECODED_GROUP_SIZE) {
        return Err(D64GcrError::InvalidDecodedLength(source.len()));
    }
    let mut encoded = vec![0; source.len() / DECODED_GROUP_SIZE * ENCODED_GROUP_SIZE];
    let mut output_index = 0;
    let mut bit_buffer = 0_u32;
    let mut buffered_bits = 0_u8;
    for &value in source {
        for code in [
            GCR_NIBBLE_TO_CODE[usize::from(value >> 4)],
            GCR_NIBBLE_TO_CODE[usize::from(value & 0x0f)],
        ] {
            bit_buffer = (bit_buffer << 5) | u32::from(code);
            buffered_bits += 5;
            if buffered_bits >= 8 {
                buffered_bits -= 8;
                encoded[output_index] = (bit_buffer >> buffered_bits).to_le_bytes()[0];
                output_index += 1;
                bit_buffer &= if buffered_bits == 0 {
                    0
                } else {
                    (1_u32 << buffered_bits) - 1
                };
            }
        }
    }
    if buffered_bits != 0 || output_index != encoded.len() {
        return Err(D64GcrError::InternalAssembly);
    }
    Ok(encoded)
}

/// Decode complete five-byte Commodore GCR groups.
///
/// # Errors
///
/// Rejects non-grouped input or any undefined five-bit symbol.
pub fn decode_commodore_gcr(source: &[u8]) -> Result<Vec<u8>, D64GcrError> {
    if !source.len().is_multiple_of(ENCODED_GROUP_SIZE) {
        return Err(D64GcrError::InvalidEncodedLength(source.len()));
    }
    let mut decoded = vec![0; source.len() / ENCODED_GROUP_SIZE * DECODED_GROUP_SIZE];
    let mut output_index = 0;
    let mut high_nibble = None;
    let mut bit_buffer = 0_u32;
    let mut buffered_bits = 0_u8;
    for &value in source {
        bit_buffer = (bit_buffer << 8) | u32::from(value);
        buffered_bits += 8;
        while buffered_bits >= 5 {
            buffered_bits -= 5;
            let code = ((bit_buffer >> buffered_bits) & 0x1f).to_le_bytes()[0];
            bit_buffer &= if buffered_bits == 0 {
                0
            } else {
                (1_u32 << buffered_bits) - 1
            };
            let nibble = decode_nibble(code)?;
            if let Some(high) = high_nibble.take() {
                decoded[output_index] = (high << 4) | nibble;
                output_index += 1;
            } else {
                high_nibble = Some(nibble);
            }
        }
    }
    if buffered_bits != 0 || high_nibble.is_some() || output_index != decoded.len() {
        return Err(D64GcrError::InternalAssembly);
    }
    Ok(decoded)
}

/// Encode one D64 sector including sync marks, header, checksums and gaps.
///
/// # Errors
///
/// Rejects invalid track geometry or error codes that do not describe passive media.
pub fn encode_d64_sector_to_gcr(
    sector_data: &[u8],
    disk_id: D64DiskId,
    track: u8,
    sector: u8,
    error_code: D64ErrorCode,
) -> Result<Vec<u8>, D64GcrError> {
    if sector_data.len() != 0x100 {
        return Err(D64GcrError::InvalidDecodedLength(sector_data.len()));
    }
    require_representable_error_code(error_code)?;
    let speed_zone = d64_speed_zone_for_track(track)?;
    let data_gap_size = DATA_GAP_BY_SPEED_ZONE[usize::from(speed_zone.get())];
    let mut output =
        vec![
            FILL_BYTE;
            ENCODED_HEADER_AND_DATA_SIZE + HEADER_GAP_SIZE + data_gap_size + SYNC_LENGTH * 2
        ];
    let sync_byte = if error_code == D64ErrorCode::SyncNotFound {
        FILL_BYTE
    } else {
        SYNC_BYTE
    };
    let mut offset = 0;
    output[offset..offset + SYNC_LENGTH].fill(sync_byte);
    offset += SYNC_LENGTH;

    let id_mutation = u8::from(error_code == D64ErrorCode::DiskIdMismatch) * u8::MAX;
    let header_checksum_mutation = u8::from(error_code == D64ErrorCode::HeaderChecksum) * u8::MAX;
    let decoded_header = [
        if error_code == D64ErrorCode::HeaderNotFound {
            0xff
        } else {
            0x08
        },
        header_checksum_mutation ^ sector ^ track ^ disk_id.id2 ^ disk_id.id1 ^ id_mutation,
        sector,
        track,
        disk_id.id2,
        disk_id.id1 ^ id_mutation,
        0x0f,
        0x0f,
    ];
    let encoded_header = encode_commodore_gcr(&decoded_header)?;
    output[offset..offset + encoded_header.len()].copy_from_slice(&encoded_header);
    offset += encoded_header.len() + HEADER_GAP_SIZE;

    output[offset..offset + SYNC_LENGTH].fill(sync_byte);
    offset += SYNC_LENGTH;
    let mut decoded_data = vec![0; DATA_BLOCK_DECODED_SIZE];
    decoded_data[0] = if error_code == D64ErrorCode::DataBlockNotFound {
        0x00
    } else {
        0x07
    };
    decoded_data[1..257].copy_from_slice(sector_data);
    let mut checksum = u8::from(error_code == D64ErrorCode::DataChecksum) * u8::MAX;
    for &value in sector_data {
        checksum ^= value;
    }
    decoded_data[257] = checksum;
    let encoded_data = encode_commodore_gcr(&decoded_data)?;
    output[offset..offset + encoded_data.len()].copy_from_slice(&encoded_data);
    offset += encoded_data.len() + data_gap_size;
    if offset != output.len() {
        return Err(D64GcrError::InternalAssembly);
    }
    Ok(output)
}

/// Materialize one sector-based D64 track as a raw rotating GCR byte stream.
///
/// # Errors
///
/// Propagates image, geometry and unsupported passive-error failures.
pub fn build_d64_gcr_track(image: &D64DiskImage, track: u8) -> Result<D64GcrTrack, D64GcrError> {
    let sector_count = image.sectors_on_track(track)?;
    let speed_zone = d64_speed_zone_for_track(track)?;
    let zone_index = usize::from(speed_zone.get());
    let capacity = D64_GCR_RAW_TRACK_SIZE[zone_index];
    let mut bytes = vec![FILL_BYTE; capacity];
    let disk_id = image.disk_id()?;
    let mut offset = 0;
    for sector in 0..sector_count {
        let encoded = encode_d64_sector_to_gcr(
            image.read_sector(track, sector)?,
            disk_id,
            track,
            sector,
            image.error_code(track, sector)?,
        )?;
        if offset + encoded.len() > bytes.len() {
            return Err(D64GcrError::TrackCapacity { track, capacity });
        }
        bytes[offset..offset + encoded.len()].copy_from_slice(&encoded);
        offset += encoded.len();
    }
    Ok(D64GcrTrack {
        bytes,
        speed_zone,
        track,
        transfer_bits_per_second: D64_GCR_TRANSFER_BITS_PER_SECOND[zone_index],
    })
}

/// Scan a cyclic raw track and recover only checksum-valid standard sectors.
///
/// # Errors
///
/// Rejects an empty raw track. Candidate-level decode failures are returned as issues.
pub fn decode_d64_gcr_track(track_bytes: &[u8]) -> Result<D64GcrTrackDecodeResult, D64GcrError> {
    if track_bytes.is_empty() {
        return Err(D64GcrError::EmptyTrack);
    }
    let starts = find_sync_payload_starts(track_bytes);
    let mut issues = Vec::new();
    let mut sectors = Vec::new();
    for (candidate_index, &bit_offset) in starts.iter().enumerate() {
        let Some(header) = decode_header_candidate(track_bytes, bit_offset, &mut issues) else {
            continue;
        };
        let Some(&data_bit_offset) = starts.get((candidate_index + 1) % starts.len()) else {
            issues.push(D64GcrDecodeIssue {
                bit_offset,
                reason: String::from("header has no following sync-delimited data block"),
            });
            continue;
        };
        let Some(data) = decode_data_candidate(track_bytes, data_bit_offset, &mut issues) else {
            continue;
        };
        sectors.push(DecodedD64GcrSector {
            data,
            header_bit_offset: bit_offset,
            id1: header.id1,
            id2: header.id2,
            sector: header.sector,
            track: header.track,
        });
    }
    Ok(D64GcrTrackDecodeResult { issues, sectors })
}

/// Resolve the 1541 density zone for a physical track.
///
/// # Errors
///
/// Rejects tracks outside 1..=42.
pub fn d64_speed_zone_for_track(track: u8) -> Result<Drive1541SpeedZone, D64GcrError> {
    match track {
        1..=17 => Ok(Drive1541SpeedZone::Zone3),
        18..=24 => Ok(Drive1541SpeedZone::Zone2),
        25..=30 => Ok(Drive1541SpeedZone::Zone1),
        31..=42 => Ok(Drive1541SpeedZone::Zone0),
        _ => Err(D64GcrError::InvalidTrack(track)),
    }
}

fn require_representable_error_code(error_code: D64ErrorCode) -> Result<(), D64GcrError> {
    match error_code {
        D64ErrorCode::Ok
        | D64ErrorCode::HeaderNotFound
        | D64ErrorCode::SyncNotFound
        | D64ErrorCode::DataBlockNotFound
        | D64ErrorCode::DataChecksum
        | D64ErrorCode::HeaderChecksum
        | D64ErrorCode::DiskIdMismatch => Ok(()),
        D64ErrorCode::Verify
        | D64ErrorCode::WriteProtected
        | D64ErrorCode::DataBlockLength
        | D64ErrorCode::FormatSpeed
        | D64ErrorCode::Drive
        | D64ErrorCode::Decode => Err(D64GcrError::UnrepresentableErrorCode(error_code)),
    }
}

fn decode_nibble(code: u8) -> Result<u8, D64GcrError> {
    let nibble = GCR_CODE_TO_NIBBLE[usize::from(code)];
    if nibble < 0 {
        Err(D64GcrError::InvalidGcrCode(code))
    } else {
        Ok(nibble.to_le_bytes()[0])
    }
}

fn find_sync_payload_starts(track_bytes: &[u8]) -> Vec<usize> {
    let bit_length = track_bytes.len() * 8;
    let mut starts = Vec::new();
    for bit_offset in 0..bit_length {
        if read_cyclic_track_bit(track_bytes, bit_offset) != 0 {
            continue;
        }
        let preceded_by_sync = (1..=SYNC_DETECT_BITS).all(|distance| {
            let previous = (bit_offset + bit_length - distance % bit_length) % bit_length;
            read_cyclic_track_bit(track_bytes, previous) != 0
        });
        if preceded_by_sync {
            starts.push(bit_offset);
        }
    }
    starts
}

fn decode_header_candidate(
    track_bytes: &[u8],
    bit_offset: usize,
    issues: &mut Vec<D64GcrDecodeIssue>,
) -> Option<D64DiskIdAndSector> {
    let decoded =
        decode_commodore_gcr(&read_cyclic_track_bytes(track_bytes, bit_offset, 10)).ok()?;
    if decoded.first().copied() != Some(0x08) {
        return None;
    }
    let checksum = decoded[1];
    let sector = decoded[2];
    let track = decoded[3];
    let id2 = decoded[4];
    let id1 = decoded[5];
    let valid_sector = d64_sectors_on_track(track).is_ok_and(|sector_count| sector < sector_count);
    if !valid_sector {
        issues.push(D64GcrDecodeIssue {
            bit_offset,
            reason: format!("header identifies invalid track/sector {track}/{sector}"),
        });
        return None;
    }
    if checksum != sector ^ track ^ id2 ^ id1 {
        issues.push(D64GcrDecodeIssue {
            bit_offset,
            reason: format!("header checksum failed for track/sector {track}/{sector}"),
        });
        return None;
    }
    Some(D64DiskIdAndSector {
        id1,
        id2,
        sector,
        track,
    })
}

fn decode_data_candidate(
    track_bytes: &[u8],
    bit_offset: usize,
    issues: &mut Vec<D64GcrDecodeIssue>,
) -> Option<Vec<u8>> {
    let source = read_cyclic_track_bytes(track_bytes, bit_offset, 325);
    let decoded = match decode_commodore_gcr_prefix(&source, 258) {
        Ok(value) => value,
        Err(error) => {
            issues.push(D64GcrDecodeIssue {
                bit_offset,
                reason: format!("data block contains illegal GCR: {error}"),
            });
            return None;
        }
    };
    if decoded.first().copied() != Some(0x07) {
        issues.push(D64GcrDecodeIssue {
            bit_offset,
            reason: String::from("header is not followed by a standard $07 data block"),
        });
        return None;
    }
    let checksum = decoded[1..257].iter().fold(0, |value, byte| value ^ byte);
    if decoded[257] != checksum {
        issues.push(D64GcrDecodeIssue {
            bit_offset,
            reason: String::from("data block checksum does not match its payload"),
        });
        return None;
    }
    Some(decoded[1..257].to_vec())
}

fn decode_commodore_gcr_prefix(
    source: &[u8],
    decoded_byte_length: usize,
) -> Result<Vec<u8>, D64GcrError> {
    if decoded_byte_length == 0 || decoded_byte_length * 10 > source.len() * 8 {
        return Err(D64GcrError::InvalidEncodedLength(source.len()));
    }
    let mut decoded = vec![0; decoded_byte_length];
    for (index, output) in decoded.iter_mut().enumerate() {
        let high = decode_nibble(read_linear_bits(source, index * 10, 5)?)?;
        let low = decode_nibble(read_linear_bits(source, index * 10 + 5, 5)?)?;
        *output = (high << 4) | low;
    }
    Ok(decoded)
}

fn read_linear_bits(source: &[u8], bit_offset: usize, bit_count: usize) -> Result<u8, D64GcrError> {
    let mut result = 0;
    for bit in 0..bit_count {
        let absolute = bit_offset + bit;
        let Some(&value) = source.get(absolute / 8) else {
            return Err(D64GcrError::InvalidEncodedLength(source.len()));
        };
        result = (result << 1) | ((value >> (7 - (absolute & 7))) & 1);
    }
    Ok(result)
}

fn read_cyclic_track_bytes(
    track_bytes: &[u8],
    start_bit_offset: usize,
    byte_length: usize,
) -> Vec<u8> {
    let mut result = vec![0; byte_length];
    for (byte_index, output) in result.iter_mut().enumerate() {
        let mut value = 0;
        for bit in 0..8 {
            value = (value << 1)
                | read_cyclic_track_bit(track_bytes, start_bit_offset + byte_index * 8 + bit);
        }
        *output = value;
    }
    result
}

fn read_cyclic_track_bit(track_bytes: &[u8], bit_offset: usize) -> u8 {
    let normalized = bit_offset % (track_bytes.len() * 8);
    (track_bytes[normalized / 8] >> (7 - (normalized & 7))) & 1
}

#[derive(Clone, Copy)]
struct D64DiskIdAndSector {
    id1: u8,
    id2: u8,
    sector: u8,
    track: u8,
}

#[cfg(test)]
mod tests {
    use crate::media::d64::{D64_SECTOR_SIZE, D64DiskImage, d64_sector_count_through_track};

    use super::{
        build_d64_gcr_track, decode_commodore_gcr, decode_d64_gcr_track, encode_commodore_gcr,
    };

    #[test]
    fn codec_round_trips_every_byte_value() {
        let source: Vec<u8> = (0..=u8::MAX).collect();
        let encoded = encode_commodore_gcr(&source).unwrap();
        assert_eq!(decode_commodore_gcr(&encoded).unwrap(), source);
    }

    #[test]
    fn built_track_decodes_all_standard_sectors() {
        let sectors = d64_sector_count_through_track(35).unwrap();
        let mut bytes = vec![0; sectors * D64_SECTOR_SIZE];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = index.to_le_bytes()[0];
        }
        let image = D64DiskImage::parse(&bytes, false).unwrap();
        let track = build_d64_gcr_track(&image, 18).unwrap();
        let decoded = decode_d64_gcr_track(&track.bytes).unwrap();
        assert_eq!(decoded.sectors.len(), 19);
        assert!(decoded.issues.is_empty());
    }
}
