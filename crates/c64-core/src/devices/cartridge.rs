// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - expansion-port cartridge boards
//
//   File:       devices/cartridge.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::address_space::{CartridgeLines, CartridgeRegion};
use crate::media::crt::{
    CRT_CHIP_TYPE_ROM, CRT_HARDWARE_TYPE_EASY_FLASH, CRT_HARDWARE_TYPE_MAGIC_DESK,
    CRT_HARDWARE_TYPE_OCEAN, CRT_HARDWARE_TYPE_STANDARD, CrtChipPacket, CrtImage, CrtImageError,
};
use crate::pla::{CartridgeMode, cartridge_mode};

mod amd29f040b;
mod easyflash;

pub use amd29f040b::{
    AMD_29F040B_BYTE_PROGRAM_CYCLES, AMD_29F040B_CAPACITY_BYTES, AMD_29F040B_CHIP_ERASE_CYCLES,
    AMD_29F040B_SECTOR_ERASE_CYCLES, AMD_29F040B_SECTOR_ERASE_WINDOW_CYCLES,
    AMD_29F040B_SECTOR_SIZE_BYTES, AMD_29F040B_STATUS_TOGGLE_BIT, AMD_29F040B_UNLOCK_ADDRESS_1,
    AMD_29F040B_UNLOCK_ADDRESS_2, Amd29F040BError, Amd29F040BFlash, Amd29F040BState,
};
pub use easyflash::{
    EASY_FLASH_BANK_COUNT, EASY_FLASH_BANK_SIZE_BYTES, EASY_FLASH_IO2_RAM_SIZE_BYTES,
    EasyFlashCartridge,
};

pub const CARTRIDGE_ROM_BANK_SIZE: usize = 0x2000;
pub const OCEAN_CARTRIDGE_BANK_COUNTS: &[usize] = &[4, 16, 32, 64];
pub const MAGIC_DESK_CARTRIDGE_BANK_COUNTS: &[usize] = &[4, 8, 16, 32, 64, 128];

const CARTRIDGE_ROM_ADDRESS_MASK: usize = CARTRIDGE_ROM_BANK_SIZE - 1;
const MAGIC_DESK_DISABLE_BIT: u8 = 1 << 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum CartridgeKind {
    Standard = CRT_HARDWARE_TYPE_STANDARD,
    Ocean = CRT_HARDWARE_TYPE_OCEAN,
    MagicDesk = CRT_HARDWARE_TYPE_MAGIC_DESK,
    EasyFlash = crate::media::crt::CRT_HARDWARE_TYPE_EASY_FLASH,
}

impl CartridgeKind {
    pub const fn code(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CartridgeError {
    Crt(CrtImageError),
    Flash(Amd29F040BError),
    UnsupportedHardwareType(u16),
    MissingChipPackets {
        device: &'static str,
    },
    DetachedStandardCartridge,
    InvalidChipType {
        device: &'static str,
        bank: u16,
        chip_type: u16,
    },
    StandardChipMustUseBankZero {
        bank: u16,
        load_address: u16,
    },
    EmptyChipPacket {
        bank: u16,
        load_address: u16,
    },
    ChipByteOutsideMode {
        address: u32,
    },
    OverlappingChipData {
        signal: &'static str,
        address: u16,
    },
    MissingChipData {
        signal: &'static str,
        address: u16,
    },
    UnsupportedBankCount {
        device: &'static str,
        count: usize,
    },
    InvalidBankSize {
        device: &'static str,
        bank: u16,
        actual: usize,
    },
    InvalidBankLoadAddress {
        device: &'static str,
        bank: u16,
        load_address: u16,
    },
    DuplicateBank {
        device: &'static str,
        bank: u16,
    },
    MissingBank {
        device: &'static str,
        bank: usize,
    },
    InvalidEasyFlashIo2RamSize(usize),
    InvalidEasyFlashChipType {
        bank: u16,
        chip_type: u16,
    },
    InvalidEasyFlashBank(u16),
    InvalidEasyFlashChipLayout {
        bank: u16,
        load_address: u16,
        image_size: usize,
    },
    OverlappingEasyFlashData {
        bank: u16,
        flash_offset: usize,
    },
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crt(error) => error.fmt(formatter),
            Self::Flash(error) => error.fmt(formatter),
            Self::UnsupportedHardwareType(hardware_type) => write!(
                formatter,
                "CRT hardware type {hardware_type} is not implemented by the Rust cartridge factory"
            ),
            Self::MissingChipPackets { device } => {
                write!(formatter, "{device} CRT image contains no CHIP packets")
            }
            Self::DetachedStandardCartridge => formatter.write_str(
                "standard CRT header leaves both GAME and EXROM high, so no cartridge is selected",
            ),
            Self::InvalidChipType {
                device,
                bank,
                chip_type,
            } => write!(
                formatter,
                "{device} requires ROM CHIP packets; bank {bank} has type {chip_type}"
            ),
            Self::StandardChipMustUseBankZero { bank, load_address } => write!(
                formatter,
                "standard ROM cartridge requires bank 0; received bank {bank} at ${load_address:04x}"
            ),
            Self::EmptyChipPacket { bank, load_address } => write!(
                formatter,
                "CRT CHIP packet for bank {bank} at ${load_address:04x} is empty"
            ),
            Self::ChipByteOutsideMode { address } => write!(
                formatter,
                "CRT CHIP byte at ${address:04x} is outside the selected cartridge mode"
            ),
            Self::OverlappingChipData { signal, address } => write!(
                formatter,
                "CRT CHIP packets overlap in {signal} at ${address:04x}"
            ),
            Self::MissingChipData { signal, address } => write!(
                formatter,
                "CRT image does not drive {signal} address ${address:04x}"
            ),
            Self::UnsupportedBankCount { device, count } => {
                write!(formatter, "{device} has unsupported ROM bank count {count}")
            }
            Self::InvalidBankSize {
                device,
                bank,
                actual,
            } => write!(
                formatter,
                "{device} ROM bank {bank} must contain {CARTRIDGE_ROM_BANK_SIZE} bytes; received {actual}"
            ),
            Self::InvalidBankLoadAddress {
                device,
                bank,
                load_address,
            } => write!(
                formatter,
                "{device} ROM bank {bank} has invalid load address ${load_address:04x}"
            ),
            Self::DuplicateBank { device, bank } => {
                write!(formatter, "{device} CRT contains duplicate ROM bank {bank}")
            }
            Self::MissingBank { device, bank } => {
                write!(formatter, "{device} CRT is missing ROM bank {bank}")
            }
            Self::InvalidEasyFlashIo2RamSize(actual) => write!(
                formatter,
                "EasyFlash IO2 RAM must contain {EASY_FLASH_IO2_RAM_SIZE_BYTES} bytes; received {actual}"
            ),
            Self::InvalidEasyFlashChipType { bank, chip_type } => write!(
                formatter,
                "EasyFlash CRT requires Flash CHIP packets; bank {bank} has type {chip_type}"
            ),
            Self::InvalidEasyFlashBank(bank) => write!(
                formatter,
                "EasyFlash CRT bank must be from 0 through {}; received {bank}",
                EASY_FLASH_BANK_COUNT - 1
            ),
            Self::InvalidEasyFlashChipLayout {
                bank,
                load_address,
                image_size,
            } => write!(
                formatter,
                "EasyFlash CRT bank {bank} has invalid {image_size}-byte CHIP at ${load_address:04x}"
            ),
            Self::OverlappingEasyFlashData { bank, flash_offset } => write!(
                formatter,
                "EasyFlash CRT contains overlapping CHIP data in bank {bank} at flash offset ${flash_offset:x}"
            ),
        }
    }
}

impl std::error::Error for CartridgeError {}

impl From<CrtImageError> for CartridgeError {
    fn from(error: CrtImageError) -> Self {
        Self::Crt(error)
    }
}

impl From<Amd29F040BError> for CartridgeError {
    fn from(error: Amd29F040BError) -> Self {
        Self::Flash(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct StandardRomCartridge {
    mode: CartridgeMode,
    rom_low: Box<[u8]>,
    rom_high: Option<Box<[u8]>>,
}

impl StandardRomCartridge {
    pub const fn mode(&self) -> CartridgeMode {
        self.mode
    }

    fn lines(&self) -> CartridgeLines {
        lines_for_mode(self.mode)
    }

    fn read_low(&self, address: u16) -> u8 {
        self.rom_low[usize::from(address) & CARTRIDGE_ROM_ADDRESS_MASK]
    }

    fn read_high(&self, address: u16) -> Option<u8> {
        self.rom_high
            .as_ref()
            .map(|rom| rom[usize::from(address) & CARTRIDGE_ROM_ADDRESS_MASK])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
struct BankedCartridgeRom {
    banks: Vec<Box<[u8]>>,
}

impl BankedCartridgeRom {
    fn read(&self, bank: usize, address: u16) -> u8 {
        self.banks[bank][usize::from(address) & CARTRIDGE_ROM_ADDRESS_MASK]
    }

    fn bank_count(&self) -> usize {
        self.banks.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct OceanCartridge {
    rom: BankedCartridgeRom,
    selected_bank: usize,
}

impl OceanCartridge {
    pub const fn selected_bank(&self) -> usize {
        self.selected_bank
    }

    fn lines(&self) -> CartridgeLines {
        CartridgeLines {
            game_line_high: self.rom.bank_count() == 64,
            exrom_line_high: false,
        }
    }

    fn write_bank_register(&mut self, value: u8) {
        self.selected_bank = usize::from(value) & (self.rom.bank_count() - 1);
    }

    fn reset(&mut self) {
        self.selected_bank = 0;
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct MagicDeskCartridge {
    rom: BankedCartridgeRom,
    selected_bank: usize,
    enabled: bool,
}

impl MagicDeskCartridge {
    pub const fn selected_bank(&self) -> usize {
        self.selected_bank
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    const fn lines(&self) -> CartridgeLines {
        CartridgeLines {
            game_line_high: true,
            exrom_line_high: !self.enabled,
        }
    }

    fn write_bank_register(&mut self, value: u8) {
        self.selected_bank = usize::from(value) & (self.rom.bank_count() - 1);
        self.enabled = value & MAGIC_DESK_DISABLE_BIT == 0;
    }

    fn reset(&mut self) {
        self.write_bank_register(0);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum Cartridge {
    Standard(StandardRomCartridge),
    Ocean(OceanCartridge),
    MagicDesk(MagicDeskCartridge),
    EasyFlash(EasyFlashCartridge),
}

impl Cartridge {
    /// Parse a CRT image and construct its physical mapper atomically.
    ///
    /// # Errors
    ///
    /// Rejects malformed containers, unsupported hardware and invalid mapper
    /// geometries without changing an attached chipset.
    pub fn from_crt_bytes(
        input: &[u8],
        easy_flash_jumper_installed: bool,
    ) -> Result<Self, CartridgeError> {
        let image = CrtImage::parse(input)?;
        Self::from_crt_image(&image, easy_flash_jumper_installed)
    }

    /// Construct a mapper from an already parsed CRT image.
    ///
    /// # Errors
    ///
    /// Rejects CHIP populations that do not match the declared board.
    pub fn from_crt_image(
        image: &CrtImage,
        easy_flash_jumper_installed: bool,
    ) -> Result<Self, CartridgeError> {
        match image.header.hardware_type {
            CRT_HARDWARE_TYPE_STANDARD => create_standard_cartridge(image).map(Self::Standard),
            CRT_HARDWARE_TYPE_OCEAN => {
                create_banked_rom(image, "Ocean", OCEAN_CARTRIDGE_BANK_COUNTS).map(|rom| {
                    Self::Ocean(OceanCartridge {
                        rom,
                        selected_bank: 0,
                    })
                })
            }
            CRT_HARDWARE_TYPE_MAGIC_DESK => {
                create_banked_rom(image, "Magic Desk", MAGIC_DESK_CARTRIDGE_BANK_COUNTS).map(
                    |rom| {
                        Self::MagicDesk(MagicDeskCartridge {
                            rom,
                            selected_bank: 0,
                            enabled: true,
                        })
                    },
                )
            }
            CRT_HARDWARE_TYPE_EASY_FLASH => {
                EasyFlashCartridge::from_crt_image(image, easy_flash_jumper_installed)
                    .map(Self::EasyFlash)
            }
            hardware_type => Err(CartridgeError::UnsupportedHardwareType(hardware_type)),
        }
    }

    pub const fn kind(&self) -> CartridgeKind {
        match self {
            Self::Standard(_) => CartridgeKind::Standard,
            Self::Ocean(_) => CartridgeKind::Ocean,
            Self::MagicDesk(_) => CartridgeKind::MagicDesk,
            Self::EasyFlash(_) => CartridgeKind::EasyFlash,
        }
    }

    pub fn lines(&self) -> CartridgeLines {
        match self {
            Self::Standard(cartridge) => cartridge.lines(),
            Self::Ocean(cartridge) => cartridge.lines(),
            Self::MagicDesk(cartridge) => cartridge.lines(),
            Self::EasyFlash(cartridge) => cartridge.lines(),
        }
    }

    pub const fn irq_line_low(&self) -> bool {
        false
    }

    pub const fn nmi_line_low(&self) -> bool {
        false
    }

    pub fn read(&mut self, region: CartridgeRegion, address: u16) -> Option<u8> {
        match self {
            Self::Standard(cartridge) => match region {
                CartridgeRegion::RomLow => Some(cartridge.read_low(address)),
                CartridgeRegion::RomHigh => cartridge.read_high(address),
                CartridgeRegion::Io1 | CartridgeRegion::Io2 => None,
            },
            Self::Ocean(cartridge) => match region {
                CartridgeRegion::RomLow => {
                    Some(cartridge.rom.read(cartridge.selected_bank, address))
                }
                CartridgeRegion::RomHigh if cartridge.rom.bank_count() != 64 => {
                    Some(cartridge.rom.read(cartridge.selected_bank, address))
                }
                CartridgeRegion::RomHigh | CartridgeRegion::Io1 | CartridgeRegion::Io2 => None,
            },
            Self::MagicDesk(cartridge) => match region {
                CartridgeRegion::RomLow => {
                    Some(cartridge.rom.read(cartridge.selected_bank, address))
                }
                CartridgeRegion::RomHigh | CartridgeRegion::Io1 | CartridgeRegion::Io2 => None,
            },
            Self::EasyFlash(cartridge) => match region {
                CartridgeRegion::RomLow => Some(cartridge.read_rom_low(address)),
                CartridgeRegion::RomHigh => Some(cartridge.read_rom_high(address)),
                CartridgeRegion::Io1 => None,
                CartridgeRegion::Io2 => Some(cartridge.read_io2(address)),
            },
        }
    }

    pub fn write(&mut self, region: CartridgeRegion, address: u16, value: u8) {
        match (self, region) {
            (Self::Ocean(cartridge), CartridgeRegion::Io1) => {
                cartridge.write_bank_register(value);
            }
            (Self::MagicDesk(cartridge), CartridgeRegion::Io1) => {
                cartridge.write_bank_register(value);
            }
            (Self::EasyFlash(cartridge), CartridgeRegion::RomLow) => {
                cartridge.write_rom_low(address, value);
            }
            (Self::EasyFlash(cartridge), CartridgeRegion::RomHigh) => {
                cartridge.write_rom_high(address, value);
            }
            (Self::EasyFlash(cartridge), CartridgeRegion::Io1) => {
                cartridge.write_io1(address, value);
            }
            (Self::EasyFlash(cartridge), CartridgeRegion::Io2) => {
                cartridge.write_io2(address, value);
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Standard(_) => {}
            Self::Ocean(cartridge) => cartridge.reset(),
            Self::MagicDesk(cartridge) => cartridge.reset(),
            Self::EasyFlash(cartridge) => cartridge.reset(),
        }
    }

    pub fn clock_cycles(&mut self, cycles: u32) {
        if let Self::EasyFlash(cartridge) = self {
            cartridge.clock_cycles(cycles);
        }
    }

    pub const fn easyflash(&self) -> Option<&EasyFlashCartridge> {
        match self {
            Self::EasyFlash(cartridge) => Some(cartridge),
            Self::Standard(_) | Self::Ocean(_) | Self::MagicDesk(_) => None,
        }
    }

    pub const fn easyflash_mut(&mut self) -> Option<&mut EasyFlashCartridge> {
        match self {
            Self::EasyFlash(cartridge) => Some(cartridge),
            Self::Standard(_) | Self::Ocean(_) | Self::MagicDesk(_) => None,
        }
    }
}

fn create_standard_cartridge(image: &CrtImage) -> Result<StandardRomCartridge, CartridgeError> {
    require_chips(image, "Standard ROM")?;
    for chip in &image.chips {
        if chip.chip_type != CRT_CHIP_TYPE_ROM {
            return Err(CartridgeError::InvalidChipType {
                device: "Standard ROM",
                bank: chip.bank,
                chip_type: chip.chip_type,
            });
        }
        if chip.bank != 0 {
            return Err(CartridgeError::StandardChipMustUseBankZero {
                bank: chip.bank,
                load_address: chip.load_address,
            });
        }
        if chip.image_size == 0 {
            return Err(CartridgeError::EmptyChipPacket {
                bank: chip.bank,
                load_address: chip.load_address,
            });
        }
    }

    let mode = cartridge_mode(image.header.game_line_high, image.header.exrom_line_high);
    let ranges = match mode {
        CartridgeMode::Game8K => vec![AssemblyRange::new("ROML", 0x8000)],
        CartridgeMode::Game16K => vec![
            AssemblyRange::new("ROML", 0x8000),
            AssemblyRange::new("ROMH", 0xa000),
        ],
        CartridgeMode::Ultimax => vec![
            AssemblyRange::new("ROML", 0x8000),
            AssemblyRange::new("ROMH", 0xe000),
        ],
        CartridgeMode::Detached => return Err(CartridgeError::DetachedStandardCartridge),
    };
    let mut images = assemble_standard_ranges(&image.chips, ranges)?;
    let rom_low = images.remove(0).into_boxed_slice();
    let rom_high = images.pop().map(Vec::into_boxed_slice);
    Ok(StandardRomCartridge {
        mode,
        rom_low,
        rom_high,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssemblyRange {
    signal: &'static str,
    start: usize,
    image: Vec<u8>,
    occupied: Vec<bool>,
}

impl AssemblyRange {
    fn new(signal: &'static str, start: usize) -> Self {
        Self {
            signal,
            start,
            image: vec![0; CARTRIDGE_ROM_BANK_SIZE],
            occupied: vec![false; CARTRIDGE_ROM_BANK_SIZE],
        }
    }

    fn contains(&self, address: usize) -> bool {
        (self.start..self.start + CARTRIDGE_ROM_BANK_SIZE).contains(&address)
    }
}

fn assemble_standard_ranges(
    chips: &[CrtChipPacket],
    mut ranges: Vec<AssemblyRange>,
) -> Result<Vec<Vec<u8>>, CartridgeError> {
    for chip in chips {
        for (index, value) in chip.data.iter().copied().enumerate() {
            let address = usize::from(chip.load_address) + index;
            let range = ranges
                .iter_mut()
                .find(|range| range.contains(address))
                .ok_or(CartridgeError::ChipByteOutsideMode {
                    address: u32::try_from(address).unwrap_or(u32::MAX),
                })?;
            let destination = address - range.start;
            if range.occupied[destination] {
                return Err(CartridgeError::OverlappingChipData {
                    signal: range.signal,
                    address: u16::try_from(address).unwrap_or(u16::MAX),
                });
            }
            range.image[destination] = value;
            range.occupied[destination] = true;
        }
    }

    for range in &ranges {
        if let Some(missing) = range.occupied.iter().position(|occupied| !occupied) {
            return Err(CartridgeError::MissingChipData {
                signal: range.signal,
                address: u16::try_from(range.start + missing).unwrap_or(u16::MAX),
            });
        }
    }
    Ok(ranges.into_iter().map(|range| range.image).collect())
}

fn create_banked_rom(
    image: &CrtImage,
    device: &'static str,
    supported_bank_counts: &[usize],
) -> Result<BankedCartridgeRom, CartridgeError> {
    require_chips(image, device)?;
    let highest_bank = image
        .chips
        .iter()
        .map(|chip| usize::from(chip.bank))
        .max()
        .ok_or(CartridgeError::MissingChipPackets { device })?;
    let bank_count = highest_bank + 1;
    if !supported_bank_counts.contains(&bank_count) {
        return Err(CartridgeError::UnsupportedBankCount {
            device,
            count: bank_count,
        });
    }

    let mut banks = vec![None; bank_count];
    for chip in &image.chips {
        if chip.chip_type != CRT_CHIP_TYPE_ROM {
            return Err(CartridgeError::InvalidChipType {
                device,
                bank: chip.bank,
                chip_type: chip.chip_type,
            });
        }
        if chip.image_size != CARTRIDGE_ROM_BANK_SIZE {
            return Err(CartridgeError::InvalidBankSize {
                device,
                bank: chip.bank,
                actual: chip.image_size,
            });
        }
        if !matches!(chip.load_address, 0x8000 | 0xa000) {
            return Err(CartridgeError::InvalidBankLoadAddress {
                device,
                bank: chip.bank,
                load_address: chip.load_address,
            });
        }
        let slot = &mut banks[usize::from(chip.bank)];
        if slot.is_some() {
            return Err(CartridgeError::DuplicateBank {
                device,
                bank: chip.bank,
            });
        }
        *slot = Some(chip.data.clone().into_boxed_slice());
    }

    let banks = banks
        .into_iter()
        .enumerate()
        .map(|(bank, image)| image.ok_or(CartridgeError::MissingBank { device, bank }))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BankedCartridgeRom { banks })
}

fn require_chips(image: &CrtImage, device: &'static str) -> Result<(), CartridgeError> {
    if image.chips.is_empty() {
        return Err(CartridgeError::MissingChipPackets { device });
    }
    Ok(())
}

const fn lines_for_mode(mode: CartridgeMode) -> CartridgeLines {
    match mode {
        CartridgeMode::Detached => CartridgeLines::DISCONNECTED,
        CartridgeMode::Game8K => CartridgeLines {
            game_line_high: true,
            exrom_line_high: false,
        },
        CartridgeMode::Game16K => CartridgeLines {
            game_line_high: false,
            exrom_line_high: false,
        },
        CartridgeMode::Ultimax => CartridgeLines {
            game_line_high: false,
            exrom_line_high: true,
        },
    }
}
