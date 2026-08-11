// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 complete drive ownership
//
//   File:       devices/drive1541/drive.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::architecture::VideoStandard;
use crate::devices::iec::IecBus;

use super::clock::{Drive1541ClockError, Drive1541ClockSynchronizer};
use super::disk_via::{Drive1541DiskVia, Drive1541DiskViaError};
use super::iec_via::{Drive1541IecVia, Drive1541IecViaError};
use super::machine::{Drive1541Machine, Drive1541MachineError};
use super::mechanism::{Drive1541DiskImage, Drive1541Mechanism, Drive1541MechanismError};
use super::memory::{DRIVE_1541_ROM_SIZE, Drive1541Memory, Drive1541MemoryError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Commodore1541DriveError {
    Clock(Drive1541ClockError),
    DiskVia(Drive1541DiskViaError),
    IecVia(Drive1541IecViaError),
    Machine(Drive1541MachineError),
    Mechanism(Drive1541MechanismError),
    Memory(Drive1541MemoryError),
}

impl fmt::Display for Commodore1541DriveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clock(error) => error.fmt(formatter),
            Self::DiskVia(error) => error.fmt(formatter),
            Self::IecVia(error) => error.fmt(formatter),
            Self::Machine(error) => error.fmt(formatter),
            Self::Mechanism(error) => error.fmt(formatter),
            Self::Memory(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Commodore1541DriveError {}

impl From<Drive1541ClockError> for Commodore1541DriveError {
    fn from(error: Drive1541ClockError) -> Self {
        Self::Clock(error)
    }
}

impl From<Drive1541DiskViaError> for Commodore1541DriveError {
    fn from(error: Drive1541DiskViaError) -> Self {
        Self::DiskVia(error)
    }
}

impl From<Drive1541IecViaError> for Commodore1541DriveError {
    fn from(error: Drive1541IecViaError) -> Self {
        Self::IecVia(error)
    }
}

impl From<Drive1541MachineError> for Commodore1541DriveError {
    fn from(error: Drive1541MachineError) -> Self {
        Self::Machine(error)
    }
}

impl From<Drive1541MechanismError> for Commodore1541DriveError {
    fn from(error: Drive1541MechanismError) -> Self {
        Self::Mechanism(error)
    }
}

impl From<Drive1541MemoryError> for Commodore1541DriveError {
    fn from(error: Drive1541MemoryError) -> Self {
        Self::Memory(error)
    }
}

/// Owns one independent 1541 and its integer adapter to the C64 clock domain.
/// The shared IEC bus remains borrowed only for individual transactions.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Commodore1541Drive {
    device_number: u8,
    machine: Drive1541Machine,
    clock: Drive1541ClockSynchronizer,
    observed_reset_assertion_sequence: u64,
}

impl Commodore1541Drive {
    /// Attach one drive to a shared IEC bus using an explicit 16 KiB DOS ROM.
    ///
    /// # Errors
    ///
    /// Rejects an invalid device number or ROM length, a full IEC bus, and
    /// errors raised while constructing the board-level devices.
    pub fn new(
        device_number: u8,
        rom: &[u8],
        video_standard: VideoStandard,
        iec_bus: &mut IecBus,
    ) -> Result<Self, Commodore1541DriveError> {
        if rom.len() != DRIVE_1541_ROM_SIZE {
            return Err(Drive1541MemoryError::InvalidRomLength(rom.len()).into());
        }
        let mut rom_image = [0; DRIVE_1541_ROM_SIZE];
        rom_image.copy_from_slice(rom);

        // Construct every fallible component that does not touch the shared
        // bus before VIA1 claims its IEC port.
        let mut mechanism = Drive1541Mechanism::new();
        let disk_via = Drive1541DiskVia::new(device_number, &mut mechanism)?;
        let iec_via = Drive1541IecVia::new(device_number, iec_bus)?;
        let memory = Drive1541Memory::new_with_rom_image(&rom_image, iec_via, disk_via);
        let machine = Drive1541Machine::new(memory, mechanism);

        Ok(Self {
            device_number,
            machine,
            clock: Drive1541ClockSynchronizer::for_video_standard(video_standard),
            observed_reset_assertion_sequence: iec_bus.reset_assertion_sequence(),
        })
    }

    pub const fn device_number(&self) -> u8 {
        self.device_number
    }

    pub const fn machine(&self) -> &Drive1541Machine {
        &self.machine
    }

    pub const fn machine_mut(&mut self) -> &mut Drive1541Machine {
        &mut self.machine
    }

    pub const fn clock(&self) -> &Drive1541ClockSynchronizer {
        &self.clock
    }

    /// Mount media without replacing an existing disk implicitly.
    ///
    /// # Errors
    ///
    /// Propagates mechanism validation errors.
    pub fn mount_disk(&mut self, image: Drive1541DiskImage) -> Result<(), Commodore1541DriveError> {
        self.machine.mechanism_mut().mount_disk(image)?;
        Ok(())
    }

    /// Eject the mounted image without discarding uncommitted D64 writes.
    ///
    /// # Errors
    ///
    /// Propagates empty-drive and uncommitted-write errors.
    pub fn eject_disk(&mut self) -> Result<Drive1541DiskImage, Commodore1541DriveError> {
        Ok(self.machine.mechanism_mut().eject_disk()?)
    }

    /// Advance one host system cycle and execute every generated drive cycle
    /// before returning.
    ///
    /// # Errors
    ///
    /// Propagates pending RESET, clock conversion and machine errors.
    pub fn clock_host_cycle(
        &mut self,
        iec_bus: &mut IecBus,
    ) -> Result<u64, Commodore1541DriveError> {
        self.synchronize_iec_reset(iec_bus)?;
        Ok(self.clock.advance_host_cycle(&mut self.machine, iec_bus)?)
    }

    /// Reset electronics and CPU timing while retaining RAM, media and head
    /// position.
    ///
    /// # Errors
    ///
    /// Propagates errors from the seven physical CPU reset cycles.
    pub fn reset(&mut self, iec_bus: &mut IecBus) -> Result<(), Commodore1541DriveError> {
        self.machine.reset_hardware(iec_bus)?;
        self.clock.reset_clock(&mut self.machine);
        self.observed_reset_assertion_sequence = iec_bus.reset_assertion_sequence();
        Ok(())
    }

    /// Consume the drive and release its IEC port.
    ///
    /// # Errors
    ///
    /// Returns an IEC error if the internally owned port was already detached.
    pub fn disconnect(self, iec_bus: &mut IecBus) -> Result<(), Commodore1541DriveError> {
        self.machine.disconnect(iec_bus)?;
        Ok(())
    }

    pub(crate) fn reconfigure_host_clock(&mut self, video_standard: VideoStandard) {
        self.clock = Drive1541ClockSynchronizer::for_video_standard(video_standard);
        self.machine.reset_timing();
    }

    pub(crate) fn synchronize_iec_reset(
        &mut self,
        iec_bus: &mut IecBus,
    ) -> Result<bool, Commodore1541DriveError> {
        if self.observed_reset_assertion_sequence == iec_bus.reset_assertion_sequence() {
            return Ok(false);
        }
        self.reset(iec_bus)?;
        Ok(true)
    }
}
