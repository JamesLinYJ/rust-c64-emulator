// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 memory address decoder
//
//   File:       devices/drive1541/memory.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::devices::iec::IecBus;

use super::disk_via::{Drive1541DiskVia, Drive1541DiskViaError};
use super::iec_via::{Drive1541IecVia, Drive1541IecViaError};
use super::mechanism::Drive1541Mechanism;

pub const DRIVE_1541_RAM_SIZE: usize = 0x0800;
pub const DRIVE_1541_ROM_SIZE: usize = 0x4000;
pub const DRIVE_1541_STACK_PAGE_START: u16 = 0x0100;

const DECODED_REGION_MASK: u16 = 0x1fff;
const RAM_DECODE_END: u16 = 0x07ff;
const IEC_VIA_START: u16 = 0x1800;
const IEC_VIA_END: u16 = 0x1bff;
const DISK_VIA_START: u16 = 0x1c00;
const DISK_VIA_END: u16 = 0x1fff;
const ROM_MIRROR_START: u16 = 0x8000;
const ROM_OFFSET_MASK: usize = DRIVE_1541_ROM_SIZE - 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Drive1541MemoryError {
    InvalidRomLength(usize),
    DiskVia(Drive1541DiskViaError),
}

impl fmt::Display for Drive1541MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRomLength(length) => write!(
                formatter,
                "Commodore 1541 ROM must contain {DRIVE_1541_ROM_SIZE} bytes; received {length}"
            ),
            Self::DiskVia(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Drive1541MemoryError {}

impl From<Drive1541DiskViaError> for Drive1541MemoryError {
    fn from(error: Drive1541DiskViaError) -> Self {
        Self::DiskVia(error)
    }
}

/// The 1541 long-board decoder, including its unselected open-bus latch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Drive1541Memory {
    ram: [u8; DRIVE_1541_RAM_SIZE],
    rom: [u8; DRIVE_1541_ROM_SIZE],
    iec_via: Drive1541IecVia,
    disk_via: Drive1541DiskVia,
    data_bus_value: u8,
}

impl Drive1541Memory {
    /// Build a decoder around an exact 16 KiB DOS ROM and two attached VIAs.
    ///
    /// # Errors
    ///
    /// Rejects ROM images whose length is not exactly 16 KiB.
    pub fn new(
        rom: &[u8],
        iec_via: Drive1541IecVia,
        disk_via: Drive1541DiskVia,
    ) -> Result<Self, Drive1541MemoryError> {
        if rom.len() != DRIVE_1541_ROM_SIZE {
            return Err(Drive1541MemoryError::InvalidRomLength(rom.len()));
        }
        let mut rom_image = [0; DRIVE_1541_ROM_SIZE];
        rom_image.copy_from_slice(rom);
        Ok(Self::new_with_rom_image(&rom_image, iec_via, disk_via))
    }

    pub(super) const fn new_with_rom_image(
        rom: &[u8; DRIVE_1541_ROM_SIZE],
        iec_via: Drive1541IecVia,
        disk_via: Drive1541DiskVia,
    ) -> Self {
        Self {
            ram: [0; DRIVE_1541_RAM_SIZE],
            rom: *rom,
            iec_via,
            disk_via,
            data_bus_value: u8::MAX,
        }
    }

    pub const fn ram(&self) -> &[u8; DRIVE_1541_RAM_SIZE] {
        &self.ram
    }

    pub const fn ram_mut(&mut self) -> &mut [u8; DRIVE_1541_RAM_SIZE] {
        &mut self.ram
    }

    pub const fn rom(&self) -> &[u8; DRIVE_1541_ROM_SIZE] {
        &self.rom
    }

    pub const fn iec_via(&self) -> &Drive1541IecVia {
        &self.iec_via
    }

    pub const fn disk_via(&self) -> &Drive1541DiskVia {
        &self.disk_via
    }

    pub const fn last_data_bus_value(&self) -> u8 {
        self.data_bus_value
    }

    pub(super) fn read_reset_vector(&mut self) -> u16 {
        let low = self.rom[0x3ffc];
        let high = self.rom[0x3ffd];
        self.data_bus_value = high;
        u16::from_le_bytes([low, high])
    }

    /// Clock VIA1 followed by VIA2 after the mechanism phase for this cycle.
    ///
    /// # Errors
    ///
    /// Propagates disk-side VIA board errors.
    pub fn clock_peripherals(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
    ) -> Result<bool, Drive1541MemoryError> {
        let iec_interrupt = self.iec_via.clock_cycle(iec_bus);
        let disk_interrupt = self.disk_via.clock_via_cycle(mechanism)?;
        Ok(iec_interrupt || disk_interrupt)
    }

    /// Read one decoded CPU byte cycle.
    ///
    /// # Errors
    ///
    /// Propagates disk-side VIA board errors.
    pub fn read(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
    ) -> Result<u8, Drive1541MemoryError> {
        let value = self.read_decoded(iec_bus, mechanism, address)?;
        self.data_bus_value = value;
        Ok(value)
    }

    /// Read a little-endian word as two wrapping byte cycles.
    ///
    /// # Errors
    ///
    /// Propagates either byte access error.
    pub fn read_word(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
    ) -> Result<u16, Drive1541MemoryError> {
        let low = self.read(iec_bus, mechanism, address)?;
        let high = self.read(iec_bus, mechanism, address.wrapping_add(1))?;
        Ok(u16::from_le_bytes([low, high]))
    }

    /// Read from page one using an 8-bit stack pointer.
    ///
    /// # Errors
    ///
    /// Propagates the underlying byte access error.
    pub fn read_stack(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        stack_pointer: u8,
    ) -> Result<u8, Drive1541MemoryError> {
        self.read(
            iec_bus,
            mechanism,
            DRIVE_1541_STACK_PAGE_START | u16::from(stack_pointer),
        )
    }

    /// Write one CPU byte cycle. The CPU drives open bus and ROM writes too.
    ///
    /// # Errors
    ///
    /// Propagates disk-side VIA board errors after updating the bus latch.
    pub fn write(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
        value: u8,
    ) -> Result<(), Drive1541MemoryError> {
        self.data_bus_value = value;
        self.write_decoded(iec_bus, mechanism, address, value)
    }

    /// Write a little-endian word as two wrapping byte cycles.
    ///
    /// # Errors
    ///
    /// Propagates either byte access error.
    pub fn write_word(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
        value: u16,
    ) -> Result<(), Drive1541MemoryError> {
        let [low, high] = value.to_le_bytes();
        self.write(iec_bus, mechanism, address, low)?;
        self.write(iec_bus, mechanism, address.wrapping_add(1), high)
    }

    /// Write to page one using an 8-bit stack pointer.
    ///
    /// # Errors
    ///
    /// Propagates the underlying byte access error.
    pub fn write_stack(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        stack_pointer: u8,
        value: u8,
    ) -> Result<(), Drive1541MemoryError> {
        self.write(
            iec_bus,
            mechanism,
            DRIVE_1541_STACK_PAGE_START | u16::from(stack_pointer),
            value,
        )
    }

    /// Reset board electronics and the open-bus latch while preserving RAM.
    pub fn reset_hardware(&mut self, iec_bus: &mut IecBus, mechanism: &mut Drive1541Mechanism) {
        self.data_bus_value = u8::MAX;
        self.iec_via.reset(iec_bus);
        self.disk_via.reset(mechanism);
    }

    /// Consume the decoder and release its VIA1 port from the shared IEC bus.
    ///
    /// # Errors
    ///
    /// Returns an IEC error if the internally owned port was already detached.
    pub fn disconnect(self, iec_bus: &mut IecBus) -> Result<(), Drive1541IecViaError> {
        self.iec_via.disconnect(iec_bus)
    }

    fn read_decoded(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
    ) -> Result<u8, Drive1541MemoryError> {
        if address >= ROM_MIRROR_START {
            let offset = usize::from(address - ROM_MIRROR_START) & ROM_OFFSET_MASK;
            return Ok(self.rom[offset]);
        }

        let decoded = address & DECODED_REGION_MASK;
        if decoded <= RAM_DECODE_END {
            return Ok(self.ram[usize::from(decoded)]);
        }
        if (IEC_VIA_START..=IEC_VIA_END).contains(&decoded) {
            return Ok(self.iec_via.read(iec_bus, decoded));
        }
        if (DISK_VIA_START..=DISK_VIA_END).contains(&decoded) {
            return Ok(self.disk_via.read(mechanism, decoded)?);
        }
        Ok(self.data_bus_value)
    }

    fn write_decoded(
        &mut self,
        iec_bus: &mut IecBus,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
        value: u8,
    ) -> Result<(), Drive1541MemoryError> {
        if address >= ROM_MIRROR_START {
            return Ok(());
        }

        let decoded = address & DECODED_REGION_MASK;
        if decoded <= RAM_DECODE_END {
            self.ram[usize::from(decoded)] = value;
        } else if (IEC_VIA_START..=IEC_VIA_END).contains(&decoded) {
            self.iec_via.write(iec_bus, decoded, value);
        } else if (DISK_VIA_START..=DISK_VIA_END).contains(&decoded) {
            self.disk_via.write(mechanism, decoded, value)?;
        }
        Ok(())
    }
}
