// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 integer host clock synchronizer
//
//   File:       devices/drive1541/clock.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::architecture::VideoStandard;
use crate::devices::iec::IecBus;

use super::machine::{Drive1541Machine, Drive1541MachineError};

pub const DRIVE_1541_PROCESSOR_CLOCK_HZ: u64 = 1_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Drive1541ClockError {
    ZeroHostClock,
    PhaseOverflow,
    TargetOverflow,
    DriveAheadOfTarget {
        elapsed_cycles: u64,
        target_cycles: u64,
    },
    Machine(Drive1541MachineError),
}

impl fmt::Display for Drive1541ClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroHostClock => formatter.write_str("host processor clock must be positive"),
            Self::PhaseOverflow => {
                formatter.write_str("1541 host clock phase arithmetic overflowed")
            }
            Self::TargetOverflow => formatter.write_str("1541 target cycle counter overflowed"),
            Self::DriveAheadOfTarget {
                elapsed_cycles,
                target_cycles,
            } => write!(
                formatter,
                "1541 elapsed clock {elapsed_cycles} is ahead of target {target_cycles}"
            ),
            Self::Machine(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Drive1541ClockError {}

impl From<Drive1541MachineError> for Drive1541ClockError {
    fn from(error: Drive1541MachineError) -> Self {
        Self::Machine(error)
    }
}

/// Converts host system cycles into exact 1 MHz 1541 CPU cycles.
///
/// The rational phase is represented entirely by integers. A generated drive
/// cycle is executed immediately, so IEC reads and writes cannot observe a
/// future host cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Drive1541ClockSynchronizer {
    host_clock_hz: u64,
    host_clock_remainder: u64,
    target_drive_cycles: u64,
}

impl Drive1541ClockSynchronizer {
    /// Construct a synchronizer for an explicit host clock rate.
    ///
    /// # Errors
    ///
    /// Rejects a zero host clock rate.
    pub const fn try_new(host_clock_hz: u64) -> Result<Self, Drive1541ClockError> {
        if host_clock_hz == 0 {
            return Err(Drive1541ClockError::ZeroHostClock);
        }
        Ok(Self {
            host_clock_hz,
            host_clock_remainder: 0,
            target_drive_cycles: 0,
        })
    }

    pub const fn for_video_standard(video_standard: VideoStandard) -> Self {
        Self {
            host_clock_hz: video_standard.system_clock_hz() as u64,
            host_clock_remainder: 0,
            target_drive_cycles: 0,
        }
    }

    pub const fn host_clock_hz(self) -> u64 {
        self.host_clock_hz
    }

    pub const fn target_cycles(self) -> u64 {
        self.target_drive_cycles
    }

    pub const fn phase_remainder(self) -> u64 {
        self.host_clock_remainder
    }

    pub fn lead_cycles(self, machine: &Drive1541Machine) -> i128 {
        i128::from(machine.elapsed_cycles()) - i128::from(self.target_drive_cycles)
    }

    /// Advance one host cycle without multiplication or division in the hot
    /// path. Every generated drive cycle is executed before this call returns.
    ///
    /// Returns the number of drive cycles executed.
    ///
    /// # Errors
    ///
    /// Returns a checked-arithmetic, clock-alignment or drive-machine error.
    pub fn advance_host_cycle(
        &mut self,
        machine: &mut Drive1541Machine,
        iec_bus: &mut IecBus,
    ) -> Result<u64, Drive1541ClockError> {
        self.host_clock_remainder = self
            .host_clock_remainder
            .checked_add(DRIVE_1541_PROCESSOR_CLOCK_HZ)
            .ok_or(Drive1541ClockError::PhaseOverflow)?;
        let mut generated_cycles = 0;
        while self.host_clock_remainder >= self.host_clock_hz {
            self.host_clock_remainder -= self.host_clock_hz;
            generated_cycles += 1;
        }
        self.advance_drive_target(machine, iec_bus, generated_cycles)
    }

    /// Advance an exact host interval using one checked rational conversion.
    ///
    /// Returns the number of drive cycles executed.
    ///
    /// # Errors
    ///
    /// Returns a checked-arithmetic, clock-alignment or drive-machine error.
    pub fn advance_host_cycles(
        &mut self,
        machine: &mut Drive1541Machine,
        iec_bus: &mut IecBus,
        cycles: u64,
    ) -> Result<u64, Drive1541ClockError> {
        let added_numerator = cycles
            .checked_mul(DRIVE_1541_PROCESSOR_CLOCK_HZ)
            .ok_or(Drive1541ClockError::PhaseOverflow)?;
        let accumulated_numerator = self
            .host_clock_remainder
            .checked_add(added_numerator)
            .ok_or(Drive1541ClockError::PhaseOverflow)?;
        let generated_cycles = accumulated_numerator / self.host_clock_hz;
        self.host_clock_remainder = accumulated_numerator % self.host_clock_hz;
        self.advance_drive_target(machine, iec_bus, generated_cycles)
    }

    pub fn reset_clock(&mut self, machine: &mut Drive1541Machine) {
        self.host_clock_remainder = 0;
        self.target_drive_cycles = 0;
        machine.reset_timing();
    }

    fn advance_drive_target(
        &mut self,
        machine: &mut Drive1541Machine,
        iec_bus: &mut IecBus,
        generated_cycles: u64,
    ) -> Result<u64, Drive1541ClockError> {
        self.target_drive_cycles = self
            .target_drive_cycles
            .checked_add(generated_cycles)
            .ok_or(Drive1541ClockError::TargetOverflow)?;
        let elapsed_cycles = machine.elapsed_cycles();
        let cycles_to_run = self.target_drive_cycles.checked_sub(elapsed_cycles).ok_or(
            Drive1541ClockError::DriveAheadOfTarget {
                elapsed_cycles,
                target_cycles: self.target_drive_cycles,
            },
        )?;
        machine.clock_cycles(iec_bus, cycles_to_run)?;
        Ok(cycles_to_run)
    }
}

impl Default for Drive1541ClockSynchronizer {
    fn default() -> Self {
        Self::for_video_standard(VideoStandard::Pal)
    }
}
