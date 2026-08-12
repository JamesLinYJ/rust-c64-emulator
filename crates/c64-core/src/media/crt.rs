// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - CRT cartridge image
//
//   File:       media/crt.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const CRT_HEADER_SIZE: usize = 0x40;
pub const CRT_CHIP_TYPE_ROM: u16 = 0;
pub const CRT_CHIP_TYPE_RAM: u16 = 1;
pub const CRT_CHIP_TYPE_FLASH: u16 = 2;
pub const CRT_HARDWARE_TYPE_STANDARD: u16 = 0;
pub const CRT_HARDWARE_TYPE_OCEAN: u16 = 5;
pub const CRT_HARDWARE_TYPE_MAGIC_DESK: u16 = 19;
pub const CRT_HARDWARE_TYPE_EASY_FLASH: u16 = 32;

const CRT_HEADER_SIGNATURE: &[u8; 16] = b"C64 CARTRIDGE   ";
const CRT_CHIP_HEADER_SIZE: usize = 0x10;
const CRT_CHIP_SIGNATURE: &[u8; 4] = b"CHIP";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrtHeader {
    pub header_length: usize,
    pub version: u16,
    pub hardware_type: u16,
    pub exrom_line_high: bool,
    pub game_line_high: bool,
    pub hardware_subtype: u8,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrtChipPacket {
    pub packet_length: usize,
    pub chip_type: u16,
    pub bank: u16,
    pub load_address: u16,
    pub image_size: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrtImage {
    pub header: CrtHeader,
    pub chips: Vec<CrtChipPacket>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrtImageError {
    HeaderTooShort {
        actual: usize,
    },
    InvalidHeaderSignature,
    HeaderLengthTooSmall {
        declared: usize,
    },
    LengthExceedsHostRange,
    TruncatedRange {
        context: &'static str,
        offset: usize,
        length: usize,
        actual: usize,
    },
    InvalidDigitalLine {
        name: &'static str,
        value: u8,
    },
    InvalidChipSignature {
        offset: usize,
    },
    ChipPacketTooShort {
        offset: usize,
        packet_length: usize,
    },
    ChipImageExceedsPacket {
        offset: usize,
        image_size: usize,
        packet_length: usize,
    },
    ChipRangeCrossesAddressSpace {
        load_address: u16,
        image_size: usize,
    },
}

impl fmt::Display for CrtImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTooShort { actual } => write!(
                formatter,
                "CRT image requires at least {CRT_HEADER_SIZE} header bytes; received {actual}"
            ),
            Self::InvalidHeaderSignature => {
                formatter.write_str("CRT image has an invalid C64 CARTRIDGE header signature")
            }
            Self::HeaderLengthTooSmall { declared } => write!(
                formatter,
                "CRT header length must be at least {CRT_HEADER_SIZE} bytes; received {declared}"
            ),
            Self::LengthExceedsHostRange => {
                formatter.write_str("CRT length exceeds the host address range")
            }
            Self::TruncatedRange {
                context,
                offset,
                length,
                actual,
            } => write!(
                formatter,
                "{context} needs {length} bytes at offset {offset}, but the CRT contains {actual} bytes"
            ),
            Self::InvalidDigitalLine { name, value } => {
                write!(
                    formatter,
                    "CRT {name} line must be encoded as 0 or 1; received {value}"
                )
            }
            Self::InvalidChipSignature { offset } => {
                write!(
                    formatter,
                    "CRT CHIP packet at offset {offset} has an invalid signature"
                )
            }
            Self::ChipPacketTooShort {
                offset,
                packet_length,
            } => write!(
                formatter,
                "CRT CHIP packet at offset {offset} is only {packet_length} bytes"
            ),
            Self::ChipImageExceedsPacket {
                offset,
                image_size,
                packet_length,
            } => write!(
                formatter,
                "CRT CHIP image at offset {offset} contains {image_size} bytes outside its {packet_length}-byte packet"
            ),
            Self::ChipRangeCrossesAddressSpace {
                load_address,
                image_size,
            } => write!(
                formatter,
                "CRT CHIP range starting at ${load_address:04x} with {image_size} bytes crosses the 16-bit address space"
            ),
        }
    }
}

impl std::error::Error for CrtImageError {}

impl CrtImage {
    /// Parse one C64 CRT container without interpreting its mapper hardware.
    ///
    /// # Errors
    ///
    /// Rejects malformed signatures, non-digital GAME/EXROM values, truncated
    /// packets and CHIP address ranges outside the C64 address space.
    pub fn parse(input: &[u8]) -> Result<Self, CrtImageError> {
        if input.len() < CRT_HEADER_SIZE {
            return Err(CrtImageError::HeaderTooShort {
                actual: input.len(),
            });
        }
        if &input[..CRT_HEADER_SIGNATURE.len()] != CRT_HEADER_SIGNATURE {
            return Err(CrtImageError::InvalidHeaderSignature);
        }

        let header_length = usize::try_from(read_u32(input, 0x10))
            .map_err(|_| CrtImageError::LengthExceedsHostRange)?;
        if header_length < CRT_HEADER_SIZE {
            return Err(CrtImageError::HeaderLengthTooSmall {
                declared: header_length,
            });
        }
        require_range(input, 0, header_length, "declared CRT header")?;

        let header = CrtHeader {
            header_length,
            version: read_u16(input, 0x14),
            hardware_type: read_u16(input, 0x16),
            exrom_line_high: read_digital_line(input[0x18], "EXROM")?,
            game_line_high: read_digital_line(input[0x19], "GAME")?,
            hardware_subtype: input[0x1a],
            name: read_fixed_name(&input[0x20..0x40]),
        };

        let mut chips = Vec::new();
        let mut packet_offset = header_length;
        while packet_offset < input.len() {
            require_range(
                input,
                packet_offset,
                CRT_CHIP_HEADER_SIZE,
                "CRT CHIP header",
            )?;
            if &input[packet_offset..packet_offset + CRT_CHIP_SIGNATURE.len()] != CRT_CHIP_SIGNATURE
            {
                return Err(CrtImageError::InvalidChipSignature {
                    offset: packet_offset,
                });
            }

            let packet_length = usize::try_from(read_u32(input, packet_offset + 4))
                .map_err(|_| CrtImageError::LengthExceedsHostRange)?;
            if packet_length < CRT_CHIP_HEADER_SIZE {
                return Err(CrtImageError::ChipPacketTooShort {
                    offset: packet_offset,
                    packet_length,
                });
            }
            require_range(
                input,
                packet_offset,
                packet_length,
                "declared CRT CHIP packet",
            )?;

            let image_size = usize::from(read_u16(input, packet_offset + 0x0e));
            if image_size > packet_length - CRT_CHIP_HEADER_SIZE {
                return Err(CrtImageError::ChipImageExceedsPacket {
                    offset: packet_offset,
                    image_size,
                    packet_length,
                });
            }
            let load_address = read_u16(input, packet_offset + 0x0c);
            if usize::from(load_address) + image_size > 0x1_0000 {
                return Err(CrtImageError::ChipRangeCrossesAddressSpace {
                    load_address,
                    image_size,
                });
            }
            let data_offset = packet_offset + CRT_CHIP_HEADER_SIZE;
            chips.push(CrtChipPacket {
                packet_length,
                chip_type: read_u16(input, packet_offset + 8),
                bank: read_u16(input, packet_offset + 0x0a),
                load_address,
                image_size,
                data: input[data_offset..data_offset + image_size].to_vec(),
            });
            packet_offset = packet_offset
                .checked_add(packet_length)
                .ok_or(CrtImageError::LengthExceedsHostRange)?;
        }

        Ok(Self { header, chips })
    }
}

fn read_digital_line(value: u8, name: &'static str) -> Result<bool, CrtImageError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CrtImageError::InvalidDigitalLine { name, value }),
    }
}

fn read_fixed_name(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(bytes.len());
    bytes[..end]
        .iter()
        .map(|value| char::from(*value))
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn require_range(
    input: &[u8],
    offset: usize,
    length: usize,
    context: &'static str,
) -> Result<(), CrtImageError> {
    let end = offset
        .checked_add(length)
        .ok_or(CrtImageError::LengthExceedsHostRange)?;
    if end > input.len() {
        return Err(CrtImageError::TruncatedRange {
            context,
            offset,
            length,
            actual: input.len(),
        });
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
