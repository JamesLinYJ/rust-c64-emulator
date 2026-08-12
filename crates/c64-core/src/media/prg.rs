// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - PRG 解析与 BASIC 自动启动布局
//
//   文件:       prg.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const BASIC_PRG_LOAD_ADDRESS: u16 = 0x0801;
pub(crate) const BASIC_TEXT_START_POINTERS: [u16; 2] = [0x002b, 0x00ac];
pub(crate) const BASIC_TEXT_END_POINTERS: [u16; 4] = [0x002d, 0x002f, 0x0031, 0x00ae];
pub(crate) const BASIC_KEYBOARD_BUFFER_START: u16 = 0x0277;
pub(crate) const BASIC_KEYBOARD_BUFFER_COUNT: u16 = 0x00c6;
pub(crate) const BASIC_KEYBOARD_BUFFER_CAPACITY: u16 = 0x0289;
pub(crate) const BASIC_RUN_COMMAND: [u8; 4] = [0x52, 0x55, 0x4e, 0x0d];
pub(crate) const BASIC_RUN_COMMAND_LENGTH: u8 = 4;
const C64_ADDRESS_SPACE_SIZE: usize = 0x1_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadedPrg {
    pub load_address: u16,
    pub end_address: u16,
    pub size: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrgError {
    MissingLoadAddress,
    EmptyPayload,
    AddressRange { load_address: u16, size: usize },
    BasicLoadAddress { actual: u16 },
    BasicNotReady { text_start: u16 },
    KeyboardBufferCorrupt { used: u8, capacity: u8 },
    KeyboardBufferFull { free: u8, required: u8 },
}

impl fmt::Display for PrgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLoadAddress => {
                formatter.write_str("a PRG file must include a two-byte load address")
            }
            Self::EmptyPayload => formatter.write_str("a PRG file must contain payload bytes"),
            Self::AddressRange { load_address, size } => write!(
                formatter,
                "PRG range beginning at ${load_address:04x} with {size} bytes exceeds the 64 KiB address space"
            ),
            Self::BasicLoadAddress { actual } => write!(
                formatter,
                "BASIC RUN requires a PRG loaded at ${BASIC_PRG_LOAD_ADDRESS:04x}; this PRG loads at ${actual:04x}"
            ),
            Self::BasicNotReady { text_start } => write!(
                formatter,
                "C64 BASIC is not ready for PRG injection: text start is ${text_start:04x}"
            ),
            Self::KeyboardBufferCorrupt { used, capacity } => write!(
                formatter,
                "C64 keyboard buffer count {used} exceeds its capacity {capacity}"
            ),
            Self::KeyboardBufferFull { free, required } => write!(
                formatter,
                "C64 keyboard buffer has {free} free bytes; RUN requires {required}"
            ),
        }
    }
}

impl std::error::Error for PrgError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BasicPrgImage<'a> {
    pub load_address: u16,
    pub end_address: u16,
    pub payload: &'a [u8],
}

impl<'a> BasicPrgImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PrgError> {
        let [low, high, payload @ ..] = bytes else {
            return Err(PrgError::MissingLoadAddress);
        };
        if payload.is_empty() {
            return Err(PrgError::EmptyPayload);
        }
        let load_address = u16::from_le_bytes([*low, *high]);
        if load_address != BASIC_PRG_LOAD_ADDRESS {
            return Err(PrgError::BasicLoadAddress {
                actual: load_address,
            });
        }
        let end_address = usize::from(load_address) + payload.len();
        if end_address >= C64_ADDRESS_SPACE_SIZE {
            return Err(PrgError::AddressRange {
                load_address,
                size: payload.len(),
            });
        }
        let end_address = u16::try_from(end_address).map_err(|_| PrgError::AddressRange {
            load_address,
            size: payload.len(),
        })?;
        Ok(Self {
            load_address,
            end_address,
            payload,
        })
    }

    pub fn validate_basic_environment(mut read_ram: impl FnMut(u16) -> u8) -> Result<u8, PrgError> {
        let text_start = read_word(&mut read_ram, BASIC_TEXT_START_POINTERS[0]);
        if text_start != BASIC_PRG_LOAD_ADDRESS {
            return Err(PrgError::BasicNotReady { text_start });
        }

        let used = read_ram(BASIC_KEYBOARD_BUFFER_COUNT);
        let capacity = read_ram(BASIC_KEYBOARD_BUFFER_CAPACITY);
        if used > capacity {
            return Err(PrgError::KeyboardBufferCorrupt { used, capacity });
        }
        let required = BASIC_RUN_COMMAND_LENGTH;
        let free = capacity - used;
        if free < required {
            return Err(PrgError::KeyboardBufferFull { free, required });
        }
        Ok(used)
    }
}

fn read_word(read_ram: &mut impl FnMut(u16) -> u8, address: u16) -> u16 {
    u16::from_le_bytes([read_ram(address), read_ram(address + 1)])
}
