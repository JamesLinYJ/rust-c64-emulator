// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - EasyFlash cartridge
//
//   File:       devices/cartridge/easyflash.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use crate::address_space::CartridgeLines;
use crate::media::crt::{CRT_CHIP_TYPE_FLASH, CrtChipPacket, CrtImage};
use crate::pla::CartridgeMode;

use super::CartridgeError;
use super::amd29f040b::{AMD_29F040B_CAPACITY_BYTES, Amd29F040BFlash};

pub const EASY_FLASH_BANK_COUNT: usize = 64;
pub const EASY_FLASH_BANK_SIZE_BYTES: usize = 0x2000;
pub const EASY_FLASH_IO2_RAM_SIZE_BYTES: usize = 0x0100;

const BANK_MASK: u8 = 0x3f;
const MODE_REGISTER_MASK: u8 = 0x87;
const MODE_LED_BIT: u8 = 1 << 7;
const REGISTER_SELECT_BIT: u16 = 1 << 1;
const ROM_ADDRESS_MASK: usize = EASY_FLASH_BANK_SIZE_BYTES - 1;
const MODE_TABLE: [CartridgeMode; 16] = [
    CartridgeMode::Ultimax,
    CartridgeMode::Ultimax,
    CartridgeMode::Game16K,
    CartridgeMode::Game16K,
    CartridgeMode::Detached,
    CartridgeMode::Ultimax,
    CartridgeMode::Game8K,
    CartridgeMode::Game16K,
    CartridgeMode::Detached,
    CartridgeMode::Ultimax,
    CartridgeMode::Game8K,
    CartridgeMode::Game16K,
    CartridgeMode::Detached,
    CartridgeMode::Ultimax,
    CartridgeMode::Game8K,
    CartridgeMode::Game16K,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EasyFlashCartridge {
    flash_low: Amd29F040BFlash,
    flash_high: Amd29F040BFlash,
    io2_ram: Box<[u8]>,
    jumper_installed: bool,
    mode_register: u8,
    selected_bank: u8,
}

impl EasyFlashCartridge {
    /// Construct a board from two physical flash images and optional SRAM.
    ///
    /// # Errors
    ///
    /// Rejects flash images or SRAM with non-physical lengths.
    pub fn new(
        flash_low: &[u8],
        flash_high: &[u8],
        io2_ram: Option<&[u8]>,
        jumper_installed: bool,
    ) -> Result<Self, CartridgeError> {
        let io2_ram = if let Some(initial) = io2_ram {
            if initial.len() != EASY_FLASH_IO2_RAM_SIZE_BYTES {
                return Err(CartridgeError::InvalidEasyFlashIo2RamSize(initial.len()));
            }
            initial.to_vec().into_boxed_slice()
        } else {
            vec![0xff; EASY_FLASH_IO2_RAM_SIZE_BYTES].into_boxed_slice()
        };
        Ok(Self {
            flash_low: Amd29F040BFlash::new(flash_low)?,
            flash_high: Amd29F040BFlash::new(flash_high)?,
            io2_ram,
            jumper_installed,
            mode_register: 0,
            selected_bank: 0,
        })
    }

    pub const fn selected_bank(&self) -> u8 {
        self.selected_bank
    }

    pub const fn mode_register(&self) -> u8 {
        self.mode_register
    }

    pub const fn jumper_installed(&self) -> bool {
        self.jumper_installed
    }

    pub const fn led_on(&self) -> bool {
        self.mode_register & MODE_LED_BIT != 0
    }

    pub fn mode(&self) -> CartridgeMode {
        let jumper_index = if self.jumper_installed { 8 } else { 0 };
        MODE_TABLE[jumper_index | usize::from(self.mode_register & 0x07)]
    }

    pub fn lines(&self) -> CartridgeLines {
        match self.mode() {
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

    pub const fn flash_low(&self) -> &Amd29F040BFlash {
        &self.flash_low
    }

    pub const fn flash_high(&self) -> &Amd29F040BFlash {
        &self.flash_high
    }

    pub const fn flash_low_mut(&mut self) -> &mut Amd29F040BFlash {
        &mut self.flash_low
    }

    pub const fn flash_high_mut(&mut self) -> &mut Amd29F040BFlash {
        &mut self.flash_high
    }

    pub fn read_rom_low(&mut self, address: u16) -> u8 {
        let flash_address = self.flash_address(address);
        self.flash_low.read_valid(flash_address)
    }

    pub fn read_rom_high(&mut self, address: u16) -> u8 {
        let flash_address = self.flash_address(address);
        self.flash_high.read_valid(flash_address)
    }

    pub fn read_io2(&self, address: u16) -> u8 {
        self.io2_ram[usize::from(address & 0x00ff)]
    }

    pub fn write_io1(&mut self, address: u16, value: u8) {
        if address & REGISTER_SELECT_BIT == 0 {
            self.selected_bank = value & BANK_MASK;
        } else {
            self.mode_register = value & MODE_REGISTER_MASK;
        }
    }

    pub fn write_io2(&mut self, address: u16, value: u8) {
        self.io2_ram[usize::from(address & 0x00ff)] = value;
    }

    pub fn write_rom_low(&mut self, address: u16, value: u8) {
        let flash_address = self.flash_address(address);
        self.flash_low.write_valid(flash_address, value);
    }

    pub fn write_rom_high(&mut self, address: u16, value: u8) {
        let flash_address = self.flash_address(address);
        self.flash_high.write_valid(flash_address, value);
    }

    pub fn clock_cycles(&mut self, cycles: u32) {
        self.flash_low.clock_cycles(cycles);
        self.flash_high.clock_cycles(cycles);
    }

    /// Reset the board latches while retaining SRAM, flash data and in-flight
    /// flash commands, which are not connected to the C64 reset pin.
    pub fn reset(&mut self) {
        self.selected_bank = 0;
        self.mode_register = 0;
    }

    pub(super) fn from_crt_image(
        image: &CrtImage,
        jumper_installed: bool,
    ) -> Result<Self, CartridgeError> {
        let mut flash_low = vec![0xff; AMD_29F040B_CAPACITY_BYTES];
        let mut flash_high = vec![0xff; AMD_29F040B_CAPACITY_BYTES];
        let mut occupied_low = vec![false; AMD_29F040B_CAPACITY_BYTES];
        let mut occupied_high = vec![false; AMD_29F040B_CAPACITY_BYTES];

        for chip in &image.chips {
            if chip.chip_type != CRT_CHIP_TYPE_FLASH {
                return Err(CartridgeError::InvalidEasyFlashChipType {
                    bank: chip.bank,
                    chip_type: chip.chip_type,
                });
            }
            if usize::from(chip.bank) >= EASY_FLASH_BANK_COUNT {
                return Err(CartridgeError::InvalidEasyFlashBank(chip.bank));
            }
            let bank_offset = usize::from(chip.bank) * EASY_FLASH_BANK_SIZE_BYTES;
            match (chip.image_size, chip.load_address) {
                (EASY_FLASH_BANK_SIZE_BYTES, 0x8000) => {
                    copy_crt_chip(chip, &mut flash_low, &mut occupied_low, bank_offset, 0)?;
                }
                (EASY_FLASH_BANK_SIZE_BYTES, 0xa000 | 0xe000) => {
                    copy_crt_chip(chip, &mut flash_high, &mut occupied_high, bank_offset, 0)?;
                }
                (size, 0x8000) if size == EASY_FLASH_BANK_SIZE_BYTES * 2 => {
                    copy_crt_chip(chip, &mut flash_low, &mut occupied_low, bank_offset, 0)?;
                    copy_crt_chip(
                        chip,
                        &mut flash_high,
                        &mut occupied_high,
                        bank_offset,
                        EASY_FLASH_BANK_SIZE_BYTES,
                    )?;
                }
                _ => {
                    return Err(CartridgeError::InvalidEasyFlashChipLayout {
                        bank: chip.bank,
                        load_address: chip.load_address,
                        image_size: chip.image_size,
                    });
                }
            }
        }
        Self::new(&flash_low, &flash_high, None, jumper_installed)
    }

    fn flash_address(&self, address: u16) -> usize {
        usize::from(self.selected_bank) * EASY_FLASH_BANK_SIZE_BYTES
            + (usize::from(address) & ROM_ADDRESS_MASK)
    }
}

fn copy_crt_chip(
    chip: &CrtChipPacket,
    target: &mut [u8],
    occupied: &mut [bool],
    target_offset: usize,
    source_offset: usize,
) -> Result<(), CartridgeError> {
    for index in 0..EASY_FLASH_BANK_SIZE_BYTES {
        let destination = target_offset + index;
        if occupied[destination] {
            return Err(CartridgeError::OverlappingEasyFlashData {
                bank: chip.bank,
                flash_offset: destination,
            });
        }
        target[destination] = chip.data[source_offset + index];
        occupied[destination] = true;
    }
    Ok(())
}
