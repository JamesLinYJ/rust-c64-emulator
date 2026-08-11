// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - G64 raw half-track image
//
//   File:       media/g64.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const G64_FIRST_HALF_TRACK: u8 = 2;
pub const G64_MAXIMUM_HALF_TRACK_COUNT: u8 = 84;

const G64_HEADER_SIZE: usize = 12;
const G64_MAXIMUM_TRACK_LENGTH_OFFSET: usize = 10;
const G64_SIGNATURE: &[u8; 8] = b"GCR-1541";
const G64_SPEED_ENTRY_SIZE: usize = 4;
const G64_SUPPORTED_VERSION: u8 = 0;
const G64_TRACK_COUNT_OFFSET: usize = 9;
const G64_TRACK_ENTRY_SIZE: usize = 4;
const G64_TRACK_OFFSET_TABLE_OFFSET: usize = 12;
const G64_VERSION_OFFSET: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum G64SpeedZone {
    #[default]
    Zone0 = 0,
    Zone1 = 1,
    Zone2 = 2,
    Zone3 = 3,
}

impl G64SpeedZone {
    pub const ALL: [Self; 4] = [Self::Zone0, Self::Zone1, Self::Zone2, Self::Zone3];

    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for G64SpeedZone {
    type Error = G64ImageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Zone0),
            1 => Ok(Self::Zone1),
            2 => Ok(Self::Zone2),
            3 => Ok(Self::Zone3),
            _ => Err(G64ImageError::InvalidSpeedZone(value)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum G64SpeedMap {
    Constant(G64SpeedZone),
    Variable(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct G64HalfTrack {
    bytes: Vec<u8>,
    half_track: u8,
    speed_map: G64SpeedMap,
}

impl G64HalfTrack {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn half_track(&self) -> u8 {
        self.half_track
    }

    pub const fn speed_map(&self) -> &G64SpeedMap {
        &self.speed_map
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum G64ImageError {
    HeaderTooShort {
        actual: usize,
    },
    InvalidSignature,
    UnsupportedVersion(u8),
    InvalidHalfTrackCount(u8),
    ZeroMaximumTrackLength,
    TruncatedTables {
        required: usize,
        actual: usize,
    },
    IncompleteInteger {
        offset: usize,
        width: usize,
    },
    TrackOffsetInsideTables {
        half_track: u8,
        offset: usize,
    },
    InvalidTrackLength {
        half_track: u8,
        length: usize,
        maximum: usize,
    },
    TrackDataOutOfBounds {
        half_track: u8,
        end: usize,
        actual: usize,
    },
    SpeedMapOutOfBounds {
        half_track: u8,
        start: usize,
        end: usize,
        actual: usize,
    },
    InvalidHalfTrack(u8),
    InvalidByteIndex {
        index: usize,
        maximum: usize,
    },
    InvalidTrackDataLength {
        length: usize,
        maximum: usize,
    },
    InvalidVariableSpeedMapLength {
        length: usize,
        expected: usize,
    },
    InvalidSpeedZone(u8),
    HalfTrackNotRepresented {
        half_track: u8,
        last_half_track: u8,
    },
    MissingHalfTrack(u8),
    SerializedImageTooLarge,
    WriteProtected,
}

impl fmt::Display for G64ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTooShort { actual } => write!(
                formatter,
                "G64 image requires a {G64_HEADER_SIZE}-byte header; received {actual}"
            ),
            Self::InvalidSignature => {
                write!(formatter, "G64 image does not begin with GCR-1541")
            }
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "unsupported G64 version {version}; expected {G64_SUPPORTED_VERSION}"
            ),
            Self::InvalidHalfTrackCount(count) => write!(
                formatter,
                "G64 half-track count {count} is outside 1..={G64_MAXIMUM_HALF_TRACK_COUNT}"
            ),
            Self::ZeroMaximumTrackLength => {
                write!(
                    formatter,
                    "G64 maximum track length must be greater than zero"
                )
            }
            Self::TruncatedTables { required, actual } => write!(
                formatter,
                "G64 header tables require {required} bytes; received {actual}"
            ),
            Self::IncompleteInteger { offset, width } => write!(
                formatter,
                "G64 {width}-byte integer at offset {offset} is incomplete"
            ),
            Self::TrackOffsetInsideTables { half_track, offset } => write!(
                formatter,
                "G64 half-track {half_track} offset {offset} points inside the header tables"
            ),
            Self::InvalidTrackLength {
                half_track,
                length,
                maximum,
            } => write!(
                formatter,
                "G64 half-track {half_track} length {length} is outside 1..={maximum}"
            ),
            Self::TrackDataOutOfBounds {
                half_track,
                end,
                actual,
            } => write!(
                formatter,
                "G64 half-track {half_track} ends at {end} beyond image length {actual}"
            ),
            Self::SpeedMapOutOfBounds {
                half_track,
                start,
                end,
                actual,
            } => write!(
                formatter,
                "G64 half-track {half_track} speed map {start}..{end} exceeds image length {actual}"
            ),
            Self::InvalidHalfTrack(half_track) => write!(
                formatter,
                "G64 half-track {half_track} is outside {G64_FIRST_HALF_TRACK}..={}",
                maximum_half_track()
            ),
            Self::InvalidByteIndex { index, maximum } => write!(
                formatter,
                "G64 speed-map byte index {index} is outside 0..{maximum}"
            ),
            Self::InvalidTrackDataLength { length, maximum } => write!(
                formatter,
                "G64 half-track data length {length} is outside 1..={maximum}"
            ),
            Self::InvalidVariableSpeedMapLength { length, expected } => write!(
                formatter,
                "G64 variable speed map contains {length} bytes instead of {expected}"
            ),
            Self::InvalidSpeedZone(zone) => {
                write!(formatter, "G64 speed zone {zone} is outside 0..=3")
            }
            Self::HalfTrackNotRepresented {
                half_track,
                last_half_track,
            } => write!(
                formatter,
                "G64 half-track {half_track} is beyond represented half-track {last_half_track}"
            ),
            Self::MissingHalfTrack(half_track) => {
                write!(
                    formatter,
                    "G64 half-track {half_track} has not been recorded"
                )
            }
            Self::SerializedImageTooLarge => {
                write!(
                    formatter,
                    "G64 serialization exceeds the 32-bit offset range"
                )
            }
            Self::WriteProtected => write!(formatter, "G64 image is write-protected"),
        }
    }
}

impl std::error::Error for G64ImageError {}

/// Owns the raw GCR byte stream and speed metadata for every represented
/// physical half-track. File offsets are validated before any allocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct G64DiskImage {
    half_track_count: u8,
    maximum_track_length: usize,
    write_protected: bool,
    half_tracks: Vec<Option<G64HalfTrack>>,
    default_speed_maps: Vec<G64SpeedMap>,
}

impl G64DiskImage {
    /// Parse a version-zero G64 image.
    ///
    /// # Errors
    ///
    /// Rejects malformed headers, offsets, raw-track lengths and speed maps.
    pub fn parse(input: &[u8], write_protected: bool) -> Result<Self, G64ImageError> {
        require_header(input)?;
        let half_track_count = input[G64_TRACK_COUNT_OFFSET];
        if !(1..=G64_MAXIMUM_HALF_TRACK_COUNT).contains(&half_track_count) {
            return Err(G64ImageError::InvalidHalfTrackCount(half_track_count));
        }
        let maximum_track_length = usize::from(read_u16(input, G64_MAXIMUM_TRACK_LENGTH_OFFSET)?);
        if maximum_track_length == 0 {
            return Err(G64ImageError::ZeroMaximumTrackLength);
        }
        let table_end = table_end_offset(half_track_count);
        if input.len() < table_end {
            return Err(G64ImageError::TruncatedTables {
                required: table_end,
                actual: input.len(),
            });
        }

        let count = usize::from(half_track_count);
        let mut half_tracks = vec![None; count];
        let mut default_speed_maps = Vec::with_capacity(count);
        for (index, (half_track, half_track_slot)) in (G64_FIRST_HALF_TRACK..)
            .zip(half_tracks.iter_mut())
            .enumerate()
        {
            let track_offset = usize::try_from(read_u32(
                input,
                G64_TRACK_OFFSET_TABLE_OFFSET + index * G64_TRACK_ENTRY_SIZE,
            )?)
            .map_err(|_| G64ImageError::SerializedImageTooLarge)?;
            let speed_entry = usize::try_from(read_u32(
                input,
                speed_table_offset(half_track_count) + index * G64_SPEED_ENTRY_SIZE,
            )?)
            .map_err(|_| G64ImageError::SerializedImageTooLarge)?;
            let speed_map = parse_speed_map(
                input,
                speed_entry,
                half_track,
                maximum_track_length,
                table_end,
            )?;
            default_speed_maps.push(speed_map.clone());

            if track_offset == 0 {
                continue;
            }
            if track_offset < table_end {
                return Err(G64ImageError::TrackOffsetInsideTables {
                    half_track,
                    offset: track_offset,
                });
            }
            let track_length = usize::from(read_u16(input, track_offset)?);
            if track_length == 0 || track_length > maximum_track_length {
                return Err(G64ImageError::InvalidTrackLength {
                    half_track,
                    length: track_length,
                    maximum: maximum_track_length,
                });
            }
            let data_start = track_offset
                .checked_add(2)
                .ok_or(G64ImageError::SerializedImageTooLarge)?;
            let data_end = data_start
                .checked_add(track_length)
                .ok_or(G64ImageError::SerializedImageTooLarge)?;
            if data_end > input.len() {
                return Err(G64ImageError::TrackDataOutOfBounds {
                    half_track,
                    end: data_end,
                    actual: input.len(),
                });
            }
            *half_track_slot = Some(G64HalfTrack {
                bytes: input[data_start..data_end].to_vec(),
                half_track,
                speed_map,
            });
        }

        Ok(Self {
            half_track_count,
            maximum_track_length,
            write_protected,
            half_tracks,
            default_speed_maps,
        })
    }

    pub const fn first_half_track(&self) -> u8 {
        G64_FIRST_HALF_TRACK
    }

    pub const fn half_track_count(&self) -> u8 {
        self.half_track_count
    }

    pub fn last_half_track(&self) -> u8 {
        G64_FIRST_HALF_TRACK + self.half_track_count() - 1
    }

    pub const fn maximum_track_length(&self) -> usize {
        self.maximum_track_length
    }

    pub const fn write_protected(&self) -> bool {
        self.write_protected
    }

    pub fn set_write_protected(&mut self, write_protected: bool) {
        self.write_protected = write_protected;
    }

    /// Borrow a represented half-track without copying its raw bytes.
    ///
    /// # Errors
    ///
    /// Rejects physical half-tracks outside the 1541 G64 range.
    pub fn half_track(&self, half_track: u8) -> Result<Option<&G64HalfTrack>, G64ImageError> {
        let index = physical_half_track_index(half_track)?;
        Ok(self.half_tracks.get(index).and_then(Option::as_ref))
    }

    /// Check whether raw flux data exists for a physical half-track.
    ///
    /// # Errors
    ///
    /// Rejects physical half-tracks outside the 1541 G64 range.
    pub fn has_half_track(&self, half_track: u8) -> Result<bool, G64ImageError> {
        Ok(self.half_track(half_track)?.is_some())
    }

    /// Read the speed zone assigned to one raw-byte position.
    ///
    /// # Errors
    ///
    /// Rejects invalid physical half-tracks, byte positions and truncated maps.
    pub fn speed_zone_at_byte(
        &self,
        half_track: u8,
        byte_index: usize,
    ) -> Result<G64SpeedZone, G64ImageError> {
        let index = physical_half_track_index(half_track)?;
        if byte_index >= self.maximum_track_length {
            return Err(G64ImageError::InvalidByteIndex {
                index: byte_index,
                maximum: self.maximum_track_length.saturating_sub(1),
            });
        }
        let speed_map = self
            .half_tracks
            .get(index)
            .and_then(Option::as_ref)
            .map(G64HalfTrack::speed_map)
            .or_else(|| self.default_speed_maps.get(index));
        match speed_map {
            Some(speed_map) => speed_zone_from_map(speed_map, byte_index),
            None => Ok(default_speed_zone(half_track)),
        }
    }

    /// Replace or create one raw half-track and its speed metadata.
    ///
    /// # Errors
    ///
    /// Rejects protected images, invalid geometry and malformed speed maps.
    pub fn set_half_track(
        &mut self,
        half_track: u8,
        data: &[u8],
        speed_map: Option<G64SpeedMap>,
    ) -> Result<(), G64ImageError> {
        self.require_writable()?;
        let index = physical_half_track_index(half_track)?;
        if data.is_empty() || data.len() > self.maximum_track_length {
            return Err(G64ImageError::InvalidTrackDataLength {
                length: data.len(),
                maximum: self.maximum_track_length,
            });
        }
        let selected_speed_map = speed_map.unwrap_or_else(|| {
            self.default_speed_maps
                .get(index)
                .cloned()
                .unwrap_or_else(|| G64SpeedMap::Constant(default_speed_zone(half_track)))
        });
        self.validate_speed_map(&selected_speed_map)?;
        self.ensure_half_track_index(half_track);
        self.default_speed_maps[index] = selected_speed_map.clone();
        self.half_tracks[index] = Some(G64HalfTrack {
            bytes: data.to_vec(),
            half_track,
            speed_map: selected_speed_map,
        });
        Ok(())
    }

    /// Change one raw byte and optionally record the physical speed zone used.
    ///
    /// # Errors
    ///
    /// Rejects protected images, absent tracks and invalid byte positions.
    pub fn write_half_track_byte(
        &mut self,
        half_track: u8,
        byte_index: usize,
        value: u8,
        speed_zone: Option<G64SpeedZone>,
    ) -> Result<(), G64ImageError> {
        self.require_writable()?;
        let index = self.represented_half_track_index(half_track)?;
        let stored = self.half_tracks[index]
            .as_mut()
            .ok_or(G64ImageError::MissingHalfTrack(half_track))?;
        let maximum_byte_index = stored.bytes.len().saturating_sub(1);
        let byte = stored
            .bytes
            .get_mut(byte_index)
            .ok_or(G64ImageError::InvalidByteIndex {
                index: byte_index,
                maximum: maximum_byte_index,
            })?;
        *byte = value;
        if let Some(zone) = speed_zone {
            self.set_stored_speed_zone_at_byte(half_track, index, byte_index, zone)?;
        }
        Ok(())
    }

    /// Serialize a canonical G64 with fixed-size raw-track blocks.
    ///
    /// # Errors
    ///
    /// Returns an error if the resulting image cannot use 32-bit file offsets.
    pub fn to_bytes(&self) -> Result<Vec<u8>, G64ImageError> {
        let half_track_count = self.half_track_count();
        let variable_speed_length = variable_speed_map_length(self.maximum_track_length);
        let track_block_length = self
            .maximum_track_length
            .checked_add(2)
            .ok_or(G64ImageError::SerializedImageTooLarge)?;
        let mut output_length = table_end_offset(half_track_count);
        for index in 0..self.half_tracks.len() {
            if self.half_tracks[index].is_some() {
                output_length = checked_serialized_add(output_length, track_block_length)?;
            }
            if matches!(self.default_speed_maps[index], G64SpeedMap::Variable(_)) {
                output_length = checked_serialized_add(output_length, variable_speed_length)?;
            }
        }

        let mut output = vec![0; output_length];
        output[..G64_SIGNATURE.len()].copy_from_slice(G64_SIGNATURE);
        output[G64_VERSION_OFFSET] = G64_SUPPORTED_VERSION;
        output[G64_TRACK_COUNT_OFFSET] = half_track_count;
        write_u16(
            &mut output,
            G64_MAXIMUM_TRACK_LENGTH_OFFSET,
            u16::try_from(self.maximum_track_length)
                .map_err(|_| G64ImageError::SerializedImageTooLarge)?,
        )?;

        let mut next_block_offset = table_end_offset(half_track_count);
        for index in 0..self.half_tracks.len() {
            if let Some(track) = &self.half_tracks[index] {
                write_u32(
                    &mut output,
                    G64_TRACK_OFFSET_TABLE_OFFSET + index * G64_TRACK_ENTRY_SIZE,
                    u32::try_from(next_block_offset)
                        .map_err(|_| G64ImageError::SerializedImageTooLarge)?,
                )?;
                write_u16(
                    &mut output,
                    next_block_offset,
                    u16::try_from(track.bytes.len())
                        .map_err(|_| G64ImageError::SerializedImageTooLarge)?,
                )?;
                let data_start = next_block_offset + 2;
                output[data_start..data_start + track.bytes.len()].copy_from_slice(&track.bytes);
                next_block_offset += track_block_length;
            }

            let speed_offset = speed_table_offset(half_track_count) + index * G64_SPEED_ENTRY_SIZE;
            match &self.default_speed_maps[index] {
                G64SpeedMap::Constant(zone) => {
                    write_u32(&mut output, speed_offset, u32::from(zone.get()))?;
                }
                G64SpeedMap::Variable(packed_zones) => {
                    write_u32(
                        &mut output,
                        speed_offset,
                        u32::try_from(next_block_offset)
                            .map_err(|_| G64ImageError::SerializedImageTooLarge)?,
                    )?;
                    output[next_block_offset..next_block_offset + variable_speed_length]
                        .copy_from_slice(packed_zones);
                    next_block_offset += variable_speed_length;
                }
            }
        }
        Ok(output)
    }

    fn represented_half_track_index(&self, half_track: u8) -> Result<usize, G64ImageError> {
        let index = physical_half_track_index(half_track)?;
        if index >= self.half_tracks.len() {
            return Err(G64ImageError::HalfTrackNotRepresented {
                half_track,
                last_half_track: self.last_half_track(),
            });
        }
        Ok(index)
    }

    fn ensure_half_track_index(&mut self, half_track: u8) {
        let index = usize::from(half_track - G64_FIRST_HALF_TRACK);
        while self.half_tracks.len() <= index {
            let next_half_track = G64_FIRST_HALF_TRACK + self.half_track_count;
            self.half_tracks.push(None);
            self.default_speed_maps
                .push(G64SpeedMap::Constant(default_speed_zone(next_half_track)));
            self.half_track_count += 1;
        }
    }

    fn validate_speed_map(&self, speed_map: &G64SpeedMap) -> Result<(), G64ImageError> {
        if let G64SpeedMap::Variable(packed_zones) = speed_map {
            let expected = variable_speed_map_length(self.maximum_track_length);
            if packed_zones.len() != expected {
                return Err(G64ImageError::InvalidVariableSpeedMapLength {
                    length: packed_zones.len(),
                    expected,
                });
            }
        }
        Ok(())
    }

    fn set_stored_speed_zone_at_byte(
        &mut self,
        half_track: u8,
        half_track_index: usize,
        byte_index: usize,
        speed_zone: G64SpeedZone,
    ) -> Result<(), G64ImageError> {
        let variable_length = variable_speed_map_length(self.maximum_track_length);
        let stored = self.half_tracks[half_track_index]
            .as_mut()
            .ok_or(G64ImageError::MissingHalfTrack(half_track))?;
        if matches!(stored.speed_map, G64SpeedMap::Constant(zone) if zone == speed_zone) {
            return Ok(());
        }
        if let G64SpeedMap::Constant(zone) = stored.speed_map {
            stored.speed_map = G64SpeedMap::Variable(vec![zone.get() * 0x55; variable_length]);
        }
        let G64SpeedMap::Variable(packed_zones) = &mut stored.speed_map else {
            unreachable!("constant maps were converted above");
        };
        let packed_index = byte_index / 4;
        let shift = 6 - (byte_index % 4) * 2;
        let packed_length = packed_zones.len();
        let packed = packed_zones.get_mut(packed_index).ok_or(
            G64ImageError::InvalidVariableSpeedMapLength {
                length: packed_length,
                expected: variable_length,
            },
        )?;
        *packed = (*packed & !(0x03 << shift)) | (speed_zone.get() << shift);
        self.default_speed_maps[half_track_index] = stored.speed_map.clone();
        Ok(())
    }

    fn require_writable(&self) -> Result<(), G64ImageError> {
        if self.write_protected {
            return Err(G64ImageError::WriteProtected);
        }
        Ok(())
    }
}

fn require_header(input: &[u8]) -> Result<(), G64ImageError> {
    if input.len() < G64_HEADER_SIZE {
        return Err(G64ImageError::HeaderTooShort {
            actual: input.len(),
        });
    }
    if &input[..G64_SIGNATURE.len()] != G64_SIGNATURE {
        return Err(G64ImageError::InvalidSignature);
    }
    let version = input[G64_VERSION_OFFSET];
    if version != G64_SUPPORTED_VERSION {
        return Err(G64ImageError::UnsupportedVersion(version));
    }
    Ok(())
}

fn parse_speed_map(
    input: &[u8],
    entry: usize,
    half_track: u8,
    maximum_track_length: usize,
    table_end: usize,
) -> Result<G64SpeedMap, G64ImageError> {
    if entry <= 3 {
        return Ok(G64SpeedMap::Constant(G64SpeedZone::try_from(
            u8::try_from(entry).unwrap_or(u8::MAX),
        )?));
    }
    let length = variable_speed_map_length(maximum_track_length);
    let end = entry
        .checked_add(length)
        .ok_or(G64ImageError::SerializedImageTooLarge)?;
    if entry < table_end || end > input.len() {
        return Err(G64ImageError::SpeedMapOutOfBounds {
            half_track,
            start: entry,
            end,
            actual: input.len(),
        });
    }
    Ok(G64SpeedMap::Variable(input[entry..end].to_vec()))
}

fn speed_zone_from_map(
    speed_map: &G64SpeedMap,
    byte_index: usize,
) -> Result<G64SpeedZone, G64ImageError> {
    match speed_map {
        G64SpeedMap::Constant(zone) => Ok(*zone),
        G64SpeedMap::Variable(packed_zones) => {
            let packed_index = byte_index / 4;
            let packed = packed_zones.get(packed_index).copied().ok_or(
                G64ImageError::InvalidVariableSpeedMapLength {
                    length: packed_zones.len(),
                    expected: packed_index + 1,
                },
            )?;
            let shift = 6 - (byte_index % 4) * 2;
            G64SpeedZone::try_from((packed >> shift) & 0x03)
        }
    }
}

const fn maximum_half_track() -> u8 {
    G64_FIRST_HALF_TRACK + G64_MAXIMUM_HALF_TRACK_COUNT - 1
}

fn physical_half_track_index(half_track: u8) -> Result<usize, G64ImageError> {
    if !(G64_FIRST_HALF_TRACK..=maximum_half_track()).contains(&half_track) {
        return Err(G64ImageError::InvalidHalfTrack(half_track));
    }
    Ok(usize::from(half_track - G64_FIRST_HALF_TRACK))
}

const fn default_speed_zone(half_track: u8) -> G64SpeedZone {
    match half_track / 2 {
        0..=17 => G64SpeedZone::Zone3,
        18..=24 => G64SpeedZone::Zone2,
        25..=30 => G64SpeedZone::Zone1,
        _ => G64SpeedZone::Zone0,
    }
}

const fn variable_speed_map_length(maximum_track_length: usize) -> usize {
    maximum_track_length.div_ceil(4)
}

const fn speed_table_offset(half_track_count: u8) -> usize {
    G64_TRACK_OFFSET_TABLE_OFFSET + half_track_count as usize * G64_TRACK_ENTRY_SIZE
}

const fn table_end_offset(half_track_count: u8) -> usize {
    speed_table_offset(half_track_count) + half_track_count as usize * G64_SPEED_ENTRY_SIZE
}

fn checked_serialized_add(left: usize, right: usize) -> Result<usize, G64ImageError> {
    let result = left
        .checked_add(right)
        .ok_or(G64ImageError::SerializedImageTooLarge)?;
    if u32::try_from(result).is_err() {
        return Err(G64ImageError::SerializedImageTooLarge);
    }
    Ok(result)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, G64ImageError> {
    let end = offset
        .checked_add(2)
        .ok_or(G64ImageError::SerializedImageTooLarge)?;
    let source = bytes
        .get(offset..end)
        .ok_or(G64ImageError::IncompleteInteger { offset, width: 2 })?;
    Ok(u16::from_le_bytes([source[0], source[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, G64ImageError> {
    let end = offset
        .checked_add(4)
        .ok_or(G64ImageError::SerializedImageTooLarge)?;
    let source = bytes
        .get(offset..end)
        .ok_or(G64ImageError::IncompleteInteger { offset, width: 4 })?;
    Ok(u32::from_le_bytes([
        source[0], source[1], source[2], source[3],
    ]))
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) -> Result<(), G64ImageError> {
    let end = offset
        .checked_add(2)
        .ok_or(G64ImageError::SerializedImageTooLarge)?;
    let destination = bytes
        .get_mut(offset..end)
        .ok_or(G64ImageError::IncompleteInteger { offset, width: 2 })?;
    destination.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), G64ImageError> {
    let end = offset
        .checked_add(4)
        .ok_or(G64ImageError::SerializedImageTooLarge)?;
    let destination = bytes
        .get_mut(offset..end)
        .ok_or(G64ImageError::IncompleteInteger { offset, width: 4 })?;
    destination.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
