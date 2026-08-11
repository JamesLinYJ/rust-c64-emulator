// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - D64 sector image
//
//   File:       media/d64.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const D64_MINIMUM_TRACK_COUNT: u8 = 35;
pub const D64_MAXIMUM_TRACK_COUNT: u8 = 42;
pub const D64_SECTOR_SIZE: usize = 0x0100;
const DIRECTORY_HEADER_TRACK: u8 = 18;
const DIRECTORY_HEADER_SECTOR: u8 = 0;
const DIRECTORY_DISK_ID_1_OFFSET: usize = 0xa2;
const DIRECTORY_DISK_ID_2_OFFSET: usize = 0xa3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum D64ErrorCode {
    #[default]
    Ok = 1,
    HeaderNotFound = 2,
    SyncNotFound = 3,
    DataBlockNotFound = 4,
    DataChecksum = 5,
    Verify = 7,
    WriteProtected = 8,
    HeaderChecksum = 9,
    DataBlockLength = 10,
    DiskIdMismatch = 11,
    FormatSpeed = 12,
    Drive = 15,
    Decode = 16,
}

impl TryFrom<u8> for D64ErrorCode {
    type Error = D64ImageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Ok),
            2 => Ok(Self::HeaderNotFound),
            3 => Ok(Self::SyncNotFound),
            4 => Ok(Self::DataBlockNotFound),
            5 => Ok(Self::DataChecksum),
            7 => Ok(Self::Verify),
            8 => Ok(Self::WriteProtected),
            9 => Ok(Self::HeaderChecksum),
            10 => Ok(Self::DataBlockLength),
            11 => Ok(Self::DiskIdMismatch),
            12 => Ok(Self::FormatSpeed),
            15 => Ok(Self::Drive),
            16 => Ok(Self::Decode),
            _ => Err(D64ImageError::InvalidErrorCode(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct D64DiskId {
    pub id1: u8,
    pub id2: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum D64ImageError {
    InvalidImageSize(usize),
    InvalidTrack(u8),
    InvalidSector { track: u8, sector: u8 },
    InvalidSectorLength(usize),
    InvalidErrorCode(u8),
    WriteProtected,
}

impl fmt::Display for D64ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidImageSize(size) => write!(
                formatter,
                "unsupported D64 size {size}; expected a 35..=42 track image with optional error bytes"
            ),
            Self::InvalidTrack(track) => write!(formatter, "D64 track {track} is invalid"),
            Self::InvalidSector { track, sector } => {
                write!(formatter, "D64 track/sector {track}/{sector} is invalid")
            }
            Self::InvalidSectorLength(length) => {
                write!(
                    formatter,
                    "D64 sector contains {length} bytes instead of 256"
                )
            }
            Self::InvalidErrorCode(code) => write!(formatter, "D64 error code {code} is invalid"),
            Self::WriteProtected => write!(formatter, "D64 image is write-protected"),
        }
    }
}

impl std::error::Error for D64ImageError {}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct D64DiskImage {
    sector_data: Vec<u8>,
    error_info: Vec<D64ErrorCode>,
    track_count: u8,
    has_error_info: bool,
    write_protected: bool,
}

impl D64DiskImage {
    /// Parse a 35..=42 track D64, optionally followed by one error byte per sector.
    ///
    /// # Errors
    ///
    /// Rejects unsupported geometry and undefined error-info values.
    pub fn parse(input: &[u8], write_protected: bool) -> Result<Self, D64ImageError> {
        let (track_count, sector_count, data_length, has_error_info) =
            identify_geometry(input.len())?;
        let error_info = if has_error_info {
            input[data_length..]
                .iter()
                .copied()
                .map(D64ErrorCode::try_from)
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![D64ErrorCode::Ok; sector_count]
        };
        Ok(Self {
            sector_data: input[..data_length].to_vec(),
            error_info,
            track_count,
            has_error_info,
            write_protected,
        })
    }

    pub const fn track_count(&self) -> u8 {
        self.track_count
    }

    pub fn sector_count(&self) -> usize {
        self.error_info.len()
    }

    pub const fn has_error_info(&self) -> bool {
        self.has_error_info
    }

    pub const fn write_protected(&self) -> bool {
        self.write_protected
    }

    pub fn set_write_protected(&mut self, write_protected: bool) {
        self.write_protected = write_protected;
    }

    /// Read the BAM sector's two-byte disk identifier.
    ///
    /// # Errors
    ///
    /// Returns an error only if the parsed image's internal geometry is invalid.
    pub fn disk_id(&self) -> Result<D64DiskId, D64ImageError> {
        let header = self.read_sector(DIRECTORY_HEADER_TRACK, DIRECTORY_HEADER_SECTOR)?;
        Ok(D64DiskId {
            id1: header[DIRECTORY_DISK_ID_1_OFFSET],
            id2: header[DIRECTORY_DISK_ID_2_OFFSET],
        })
    }

    /// Return the number of sectors on a represented track.
    ///
    /// # Errors
    ///
    /// Rejects tracks outside this image.
    pub fn sectors_on_track(&self, track: u8) -> Result<u8, D64ImageError> {
        self.require_track(track)?;
        d64_sectors_on_track(track)
    }

    /// Borrow one immutable 256-byte sector.
    ///
    /// # Errors
    ///
    /// Rejects an out-of-range track or sector.
    pub fn read_sector(&self, track: u8, sector: u8) -> Result<&[u8], D64ImageError> {
        let index = self.sector_index(track, sector)?;
        let start = index * D64_SECTOR_SIZE;
        Ok(&self.sector_data[start..start + D64_SECTOR_SIZE])
    }

    /// Replace one sector and clear its DOS error byte.
    ///
    /// # Errors
    ///
    /// Rejects writes to protected images, invalid addresses or non-256-byte data.
    pub fn write_sector(
        &mut self,
        track: u8,
        sector: u8,
        data: &[u8],
    ) -> Result<(), D64ImageError> {
        if self.write_protected {
            return Err(D64ImageError::WriteProtected);
        }
        if data.len() != D64_SECTOR_SIZE {
            return Err(D64ImageError::InvalidSectorLength(data.len()));
        }
        let index = self.sector_index(track, sector)?;
        let start = index * D64_SECTOR_SIZE;
        self.sector_data[start..start + D64_SECTOR_SIZE].copy_from_slice(data);
        self.error_info[index] = D64ErrorCode::Ok;
        Ok(())
    }

    /// Read a sector's optional DOS error code.
    ///
    /// # Errors
    ///
    /// Rejects an out-of-range track or sector.
    pub fn error_code(&self, track: u8, sector: u8) -> Result<D64ErrorCode, D64ImageError> {
        Ok(self.error_info[self.sector_index(track, sector)?])
    }

    /// Change a sector's DOS error code.
    ///
    /// # Errors
    ///
    /// Rejects writes to protected images or an out-of-range address.
    pub fn set_error_code(
        &mut self,
        track: u8,
        sector: u8,
        error_code: D64ErrorCode,
    ) -> Result<(), D64ImageError> {
        if self.write_protected {
            return Err(D64ImageError::WriteProtected);
        }
        let index = self.sector_index(track, sector)?;
        self.error_info[index] = error_code;
        Ok(())
    }

    pub fn to_bytes(&self, include_error_info: bool) -> Vec<u8> {
        let extra = if include_error_info {
            self.error_info.len()
        } else {
            0
        };
        let mut result = Vec::with_capacity(self.sector_data.len() + extra);
        result.extend_from_slice(&self.sector_data);
        if include_error_info {
            result.extend(self.error_info.iter().map(|code| *code as u8));
        }
        result
    }

    fn require_track(&self, track: u8) -> Result<(), D64ImageError> {
        if track == 0 || track > self.track_count {
            return Err(D64ImageError::InvalidTrack(track));
        }
        Ok(())
    }

    fn sector_index(&self, track: u8, sector: u8) -> Result<usize, D64ImageError> {
        self.require_track(track)?;
        let sectors = d64_sectors_on_track(track)?;
        if sector >= sectors {
            return Err(D64ImageError::InvalidSector { track, sector });
        }
        Ok(d64_sector_count_through_track(track - 1)? + usize::from(sector))
    }
}

/// Return the standard number of sectors for a physical 1541 track.
///
/// # Errors
///
/// Rejects tracks outside 1..=42.
pub fn d64_sectors_on_track(track: u8) -> Result<u8, D64ImageError> {
    match track {
        1..=17 => Ok(21),
        18..=24 => Ok(19),
        25..=30 => Ok(18),
        31..=42 => Ok(17),
        _ => Err(D64ImageError::InvalidTrack(track)),
    }
}

/// Count all sectors from track one through the supplied inclusive count.
///
/// # Errors
///
/// Rejects counts above 42.
pub fn d64_sector_count_through_track(track_count: u8) -> Result<usize, D64ImageError> {
    if track_count > D64_MAXIMUM_TRACK_COUNT {
        return Err(D64ImageError::InvalidTrack(track_count));
    }
    let mut sectors = 0;
    for track in 1..=track_count {
        sectors += usize::from(d64_sectors_on_track(track)?);
    }
    Ok(sectors)
}

fn identify_geometry(length: usize) -> Result<(u8, usize, usize, bool), D64ImageError> {
    for track_count in D64_MINIMUM_TRACK_COUNT..=D64_MAXIMUM_TRACK_COUNT {
        let sector_count = d64_sector_count_through_track(track_count)?;
        let data_length = sector_count * D64_SECTOR_SIZE;
        if length == data_length {
            return Ok((track_count, sector_count, data_length, false));
        }
        if length == data_length + sector_count {
            return Ok((track_count, sector_count, data_length, true));
        }
    }
    Err(D64ImageError::InvalidImageSize(length))
}

#[cfg(test)]
mod tests {
    use super::{
        D64_SECTOR_SIZE, D64DiskImage, D64ErrorCode, D64ImageError, d64_sector_count_through_track,
    };

    #[test]
    fn parses_round_trips_and_writes_standard_35_track_images() {
        let sector_count = d64_sector_count_through_track(35).unwrap();
        let mut bytes = vec![0; sector_count * D64_SECTOR_SIZE];
        let bam_offset = d64_sector_count_through_track(17).unwrap() * D64_SECTOR_SIZE;
        bytes[bam_offset + 0xa2] = 0x12;
        bytes[bam_offset + 0xa3] = 0x34;
        let mut image = D64DiskImage::parse(&bytes, false).unwrap();
        assert_eq!(image.disk_id().unwrap().id1, 0x12);
        image.write_sector(1, 0, &[0x5a; D64_SECTOR_SIZE]).unwrap();
        assert_eq!(image.read_sector(1, 0).unwrap()[0], 0x5a);
        assert_eq!(image.to_bytes(false).len(), bytes.len());
    }

    #[test]
    fn validates_error_tables_and_write_protection() {
        let sector_count = d64_sector_count_through_track(35).unwrap();
        let mut bytes = vec![0; sector_count * D64_SECTOR_SIZE + sector_count];
        bytes[sector_count * D64_SECTOR_SIZE..].fill(D64ErrorCode::Ok as u8);
        bytes[sector_count * D64_SECTOR_SIZE] = 6;
        assert_eq!(
            D64DiskImage::parse(&bytes, false),
            Err(D64ImageError::InvalidErrorCode(6))
        );

        bytes[sector_count * D64_SECTOR_SIZE] = D64ErrorCode::Ok as u8;
        let mut image = D64DiskImage::parse(&bytes, true).unwrap();
        assert_eq!(
            image.write_sector(1, 0, &[0; D64_SECTOR_SIZE]),
            Err(D64ImageError::WriteProtected)
        );
    }
}
