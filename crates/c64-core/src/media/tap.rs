// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - TAP cassette image
//
//   File:       media/tap.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const TAP_HEADER_SIZE: usize = 20;

const TAP_C64_SYSTEM: u8 = 0;
const TAP_DATA_LENGTH_OFFSET: usize = 16;
const TAP_MAGIC: &[u8; 12] = b"C64-TAPE-RAW";
const TAP_MAXIMUM_PRECISE_PULSE_CYCLES: u32 = 0x00ff_ffff;
const TAP_MAXIMUM_SHORT_PULSE_CYCLES: u32 = 0xff * TAP_SHORT_PULSE_CYCLE_QUANTUM;
const TAP_SHORT_PULSE_CYCLE_QUANTUM: u32 = 8;
const TAP_SYSTEM_OFFSET: usize = 13;
const TAP_VERSION_OFFSET: usize = 12;
const TAP_VIDEO_STANDARD_OFFSET: usize = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum TapVersion {
    Legacy = 0,
    Precise = 1,
}

impl TryFrom<u8> for TapVersion {
    type Error = TapImageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Legacy),
            1 => Ok(Self::Precise),
            _ => Err(TapImageError::UnsupportedVersion(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum TapVideoStandard {
    #[default]
    Pal = 0,
    Ntsc = 1,
    NtscOld = 2,
    PalN = 3,
}

impl TapVideoStandard {
    pub const fn source_clock_hz(self) -> u32 {
        match self {
            Self::Pal => 985_248,
            Self::Ntsc | Self::NtscOld => 1_022_730,
            Self::PalN => 1_023_440,
        }
    }
}

impl TryFrom<u8> for TapVideoStandard {
    type Error = TapImageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Pal),
            1 => Ok(Self::Ntsc),
            2 => Ok(Self::NtscOld),
            3 => Ok(Self::PalN),
            _ => Err(TapImageError::InvalidVideoStandard(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct TapPulse {
    pub data_offset: usize,
    pub encoded_length: u8,
    pub source_cycles: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum TapImageError {
    HeaderTooShort { actual: usize },
    InvalidSignature,
    UnsupportedVersion(u8),
    UnsupportedSystem(u8),
    InvalidVideoStandard(u8),
    DeclaredDataLengthMismatch { declared: usize, actual: usize },
    InvalidLegacyOverflowPulseDuration(u32),
    LegacyOverflowPulseDurationRequired { data_offset: usize },
    TruncatedPrecisePulse { data_offset: usize },
    ZeroPrecisePulse { data_offset: usize },
    InvalidRecordedPulseDuration(u32),
    InvalidPulseIndex { index: usize, pulse_count: usize },
    DataLengthOverflow,
    TotalDurationOverflow,
    SerializedImageTooLarge,
}

impl fmt::Display for TapImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTooShort { actual } => write!(
                formatter,
                "TAP image requires a {TAP_HEADER_SIZE}-byte header; received {actual}"
            ),
            Self::InvalidSignature => {
                formatter.write_str("TAP image does not begin with C64-TAPE-RAW")
            }
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "unsupported C64 TAP version {version}; expected version 0 or 1"
            ),
            Self::UnsupportedSystem(system) => {
                write!(formatter, "TAP system {system} is not a Commodore 64 image")
            }
            Self::InvalidVideoStandard(standard) => {
                write!(formatter, "TAP video standard {standard} is not defined")
            }
            Self::DeclaredDataLengthMismatch { declared, actual } => write!(
                formatter,
                "TAP header declares {declared} data bytes, but the file contains {actual}"
            ),
            Self::InvalidLegacyOverflowPulseDuration(duration) => write!(
                formatter,
                "legacy TAP v0 overflow pulse length {duration} must be greater than zero"
            ),
            Self::LegacyOverflowPulseDurationRequired { data_offset } => write!(
                formatter,
                "TAP v0 zero marker at data offset {data_offset} requires an explicit overflow pulse duration"
            ),
            Self::TruncatedPrecisePulse { data_offset } => write!(
                formatter,
                "TAP v1 extended pulse at data offset {data_offset} is truncated"
            ),
            Self::ZeroPrecisePulse { data_offset } => write!(
                formatter,
                "TAP v1 extended pulse at data offset {data_offset} has zero length"
            ),
            Self::InvalidRecordedPulseDuration(duration) => write!(
                formatter,
                "recorded TAP pulse duration {duration} is outside 1..={TAP_MAXIMUM_PRECISE_PULSE_CYCLES}"
            ),
            Self::InvalidPulseIndex { index, pulse_count } => write!(
                formatter,
                "TAP pulse index {index} is outside 0..={pulse_count}"
            ),
            Self::DataLengthOverflow => {
                formatter.write_str("recorded TAP data exceeds the 32-bit file-format limit")
            }
            Self::TotalDurationOverflow => {
                formatter.write_str("TAP total pulse duration exceeds the 64-bit cycle range")
            }
            Self::SerializedImageTooLarge => {
                formatter.write_str("TAP serialization exceeds the host address range")
            }
        }
    }
}

impl std::error::Error for TapImageError {}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct TapImage {
    bytes: Vec<u8>,
    pulses: Vec<TapPulse>,
    source_clock_hz: u32,
    total_source_cycles: u64,
    version: TapVersion,
    video_standard: TapVideoStandard,
}

impl TapImage {
    /// Parse a C64 TAP v0 or v1 image.
    ///
    /// TAP v0 zero markers lose their original duration. Callers must provide
    /// an explicit replacement duration when such records are present.
    ///
    /// # Errors
    ///
    /// Rejects malformed headers, ambiguous v0 overflow records, truncated v1
    /// records and durations whose sum does not fit in 64 bits.
    pub fn parse(
        input: &[u8],
        legacy_v0_overflow_pulse_cycles: Option<u32>,
    ) -> Result<Self, TapImageError> {
        if input.len() < TAP_HEADER_SIZE {
            return Err(TapImageError::HeaderTooShort {
                actual: input.len(),
            });
        }
        if &input[..TAP_MAGIC.len()] != TAP_MAGIC {
            return Err(TapImageError::InvalidSignature);
        }
        let version = TapVersion::try_from(input[TAP_VERSION_OFFSET])?;
        let system = input[TAP_SYSTEM_OFFSET];
        if system != TAP_C64_SYSTEM {
            return Err(TapImageError::UnsupportedSystem(system));
        }
        let video_standard = TapVideoStandard::try_from(input[TAP_VIDEO_STANDARD_OFFSET])?;
        if legacy_v0_overflow_pulse_cycles == Some(0) {
            return Err(TapImageError::InvalidLegacyOverflowPulseDuration(0));
        }

        let declared = usize::try_from(u32::from_le_bytes(
            input[TAP_DATA_LENGTH_OFFSET..TAP_HEADER_SIZE]
                .try_into()
                .map_err(|_| TapImageError::HeaderTooShort {
                    actual: input.len(),
                })?,
        ))
        .map_err(|_| TapImageError::SerializedImageTooLarge)?;
        let actual = input.len() - TAP_HEADER_SIZE;
        if declared != actual {
            return Err(TapImageError::DeclaredDataLengthMismatch { declared, actual });
        }

        let mut pulses = Vec::new();
        let mut total_source_cycles = 0_u64;
        let mut offset = TAP_HEADER_SIZE;
        while offset < input.len() {
            let data_offset = offset - TAP_HEADER_SIZE;
            let marker = input[offset];
            let (encoded_length, source_cycles) = if marker != 0 {
                (1, u32::from(marker) * TAP_SHORT_PULSE_CYCLE_QUANTUM)
            } else if version == TapVersion::Legacy {
                (
                    1,
                    legacy_v0_overflow_pulse_cycles.ok_or(
                        TapImageError::LegacyOverflowPulseDurationRequired { data_offset },
                    )?,
                )
            } else {
                let end = offset
                    .checked_add(4)
                    .ok_or(TapImageError::SerializedImageTooLarge)?;
                if end > input.len() {
                    return Err(TapImageError::TruncatedPrecisePulse { data_offset });
                }
                let source_cycles = u32::from(input[offset + 1])
                    | (u32::from(input[offset + 2]) << 8)
                    | (u32::from(input[offset + 3]) << 16);
                if source_cycles == 0 {
                    return Err(TapImageError::ZeroPrecisePulse { data_offset });
                }
                (4, source_cycles)
            };
            total_source_cycles = total_source_cycles
                .checked_add(u64::from(source_cycles))
                .ok_or(TapImageError::TotalDurationOverflow)?;
            pulses.push(TapPulse {
                data_offset,
                encoded_length,
                source_cycles,
            });
            offset += usize::from(encoded_length);
        }

        Ok(Self {
            bytes: input.to_vec(),
            pulses,
            source_clock_hz: video_standard.source_clock_hz(),
            total_source_cycles,
            version,
            video_standard,
        })
    }

    pub fn pulses(&self) -> &[TapPulse] {
        &self.pulses
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    pub const fn source_clock_hz(&self) -> u32 {
        self.source_clock_hz
    }

    pub const fn total_source_cycles(&self) -> u64 {
        self.total_source_cycles
    }

    pub const fn version(&self) -> TapVersion {
        self.version
    }

    pub const fn video_standard(&self) -> TapVideoStandard {
        self.video_standard
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct WritableTapImage {
    data_length: u32,
    pulses: Vec<TapPulse>,
    total_source_cycles: u64,
    video_standard: TapVideoStandard,
}

impl Default for WritableTapImage {
    fn default() -> Self {
        Self::new(TapVideoStandard::Pal)
    }
}

impl WritableTapImage {
    pub const fn new(video_standard: TapVideoStandard) -> Self {
        Self {
            data_length: 0,
            pulses: Vec::new(),
            total_source_cycles: 0,
            video_standard,
        }
    }

    pub fn pulses(&self) -> &[TapPulse] {
        &self.pulses
    }

    pub const fn source_clock_hz(&self) -> u32 {
        self.video_standard.source_clock_hz()
    }

    pub const fn total_source_cycles(&self) -> u64 {
        self.total_source_cycles
    }

    pub const fn video_standard(&self) -> TapVideoStandard {
        self.video_standard
    }

    /// Append one pulse using the shortest lossless TAP v1 encoding.
    ///
    /// # Errors
    ///
    /// Rejects zero, durations beyond the 24-bit v1 representation and images
    /// that exceed their 32-bit data-length field.
    pub fn append_pulse(&mut self, source_cycles: u32) -> Result<(), TapImageError> {
        if source_cycles == 0 || source_cycles > TAP_MAXIMUM_PRECISE_PULSE_CYCLES {
            return Err(TapImageError::InvalidRecordedPulseDuration(source_cycles));
        }
        let encoded_length = if source_cycles <= TAP_MAXIMUM_SHORT_PULSE_CYCLES
            && source_cycles.is_multiple_of(TAP_SHORT_PULSE_CYCLE_QUANTUM)
        {
            1
        } else {
            4
        };
        let next_data_length = self
            .data_length
            .checked_add(u32::from(encoded_length))
            .ok_or(TapImageError::DataLengthOverflow)?;
        let next_total_source_cycles = self
            .total_source_cycles
            .checked_add(u64::from(source_cycles))
            .ok_or(TapImageError::TotalDurationOverflow)?;
        let data_offset = usize::try_from(self.data_length)
            .map_err(|_| TapImageError::SerializedImageTooLarge)?;

        self.pulses.push(TapPulse {
            data_offset,
            encoded_length,
            source_cycles,
        });
        self.data_length = next_data_length;
        self.total_source_cycles = next_total_source_cycles;
        Ok(())
    }

    /// Discard every pulse at or after `pulse_index`.
    ///
    /// # Errors
    ///
    /// Rejects indices beyond the current end of the image.
    pub fn truncate_at_pulse(&mut self, pulse_index: usize) -> Result<(), TapImageError> {
        if pulse_index > self.pulses.len() {
            return Err(TapImageError::InvalidPulseIndex {
                index: pulse_index,
                pulse_count: self.pulses.len(),
            });
        }
        if pulse_index == self.pulses.len() {
            return Ok(());
        }

        self.total_source_cycles = self.pulses[..pulse_index]
            .iter()
            .map(|pulse| u64::from(pulse.source_cycles))
            .sum();
        self.data_length = if let Some(previous_index) = pulse_index.checked_sub(1) {
            let pulse = &self.pulses[previous_index];
            u32::try_from(pulse.data_offset + usize::from(pulse.encoded_length))
                .map_err(|_| TapImageError::SerializedImageTooLarge)?
        } else {
            0
        };
        self.pulses.truncate(pulse_index);
        Ok(())
    }

    /// Serialize a canonical C64 TAP v1 image.
    ///
    /// # Errors
    ///
    /// Returns an error if the host cannot represent the image allocation.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TapImageError> {
        let data_length = usize::try_from(self.data_length)
            .map_err(|_| TapImageError::SerializedImageTooLarge)?;
        let total_length = TAP_HEADER_SIZE
            .checked_add(data_length)
            .ok_or(TapImageError::SerializedImageTooLarge)?;
        let mut bytes = vec![0; total_length];
        bytes[..TAP_MAGIC.len()].copy_from_slice(TAP_MAGIC);
        bytes[TAP_VERSION_OFFSET] = TapVersion::Precise as u8;
        bytes[TAP_SYSTEM_OFFSET] = TAP_C64_SYSTEM;
        bytes[TAP_VIDEO_STANDARD_OFFSET] = self.video_standard as u8;
        bytes[TAP_DATA_LENGTH_OFFSET..TAP_HEADER_SIZE]
            .copy_from_slice(&self.data_length.to_le_bytes());

        let mut offset = TAP_HEADER_SIZE;
        for pulse in &self.pulses {
            if pulse.encoded_length == 1 {
                bytes[offset] = u8::try_from(pulse.source_cycles / TAP_SHORT_PULSE_CYCLE_QUANTUM)
                    .map_err(|_| {
                    TapImageError::InvalidRecordedPulseDuration(pulse.source_cycles)
                })?;
                offset += 1;
            } else {
                let source_cycles = pulse.source_cycles.to_le_bytes();
                bytes[offset] = 0;
                bytes[offset + 1..offset + 4].copy_from_slice(&source_cycles[..3]);
                offset += 4;
            }
        }
        Ok(bytes)
    }
}
