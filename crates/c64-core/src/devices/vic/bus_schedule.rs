// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VIC-II half-cycle bus schedule
//
//   File:       vic/bus_schedule.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use super::{NTSC_CYCLES_PER_RASTER_LINE, PAL_CYCLES_PER_RASTER_LINE, SPRITE_COUNT};
use crate::devices::vic::timing::{NTSC_VIC_TIMING, PAL_VIC_TIMING, VicTiming};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VicPhi1Fetch {
    Graphics,
    Idle,
    Refresh,
    SpriteData { sprite_index: u8, byte_index: u8 },
    SpritePointer { sprite_index: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VicPhi2Fetch {
    Matrix,
    SpriteData { sprite_index: u8, byte_index: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicBusScheduleEntry {
    pub cycle: u8,
    pub phi1: VicPhi1Fetch,
    pub phi2: Option<VicPhi2Fetch>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicCycleRangeError {
    pub cycle: u8,
    pub cycles_per_raster_line: u8,
}

impl fmt::Display for VicCycleRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "VIC-II cycle must be in 1..={}; received {}",
            self.cycles_per_raster_line, self.cycle
        )
    }
}

impl std::error::Error for VicCycleRangeError {}

const PLACEHOLDER_ENTRY: VicBusScheduleEntry = VicBusScheduleEntry {
    cycle: 1,
    phi1: VicPhi1Fetch::Refresh,
    phi2: None,
};

pub const PAL_BUS_SCHEDULE: [VicBusScheduleEntry; PAL_CYCLES_PER_RASTER_LINE as usize] =
    create_pal_bus_schedule();
pub const NTSC_BUS_SCHEDULE: [VicBusScheduleEntry; NTSC_CYCLES_PER_RASTER_LINE as usize] =
    create_ntsc_bus_schedule();

/// Return the immutable PAL half-cycle assignment.
///
/// # Errors
///
/// Returns `VicCycleRangeError` unless `cycle` is in the architectural 1..=63 range.
pub fn vic_bus_schedule_for_cycle(cycle: u8) -> Result<VicBusScheduleEntry, VicCycleRangeError> {
    vic_bus_schedule_for_timing(PAL_VIC_TIMING, cycle)
}

/// Return the immutable half-cycle assignment for one supported VIC-II timing model.
///
/// # Errors
///
/// Returns `VicCycleRangeError` unless `cycle` is inside the configured raster line.
pub fn vic_bus_schedule_for_timing(
    timing: VicTiming,
    cycle: u8,
) -> Result<VicBusScheduleEntry, VicCycleRangeError> {
    if cycle == 0 || cycle > timing.cycles_per_raster_line {
        return Err(VicCycleRangeError {
            cycle,
            cycles_per_raster_line: timing.cycles_per_raster_line,
        });
    }
    let index = usize::from(cycle - 1);
    if timing.is_ntsc() {
        Ok(NTSC_BUS_SCHEDULE[index])
    } else {
        Ok(PAL_BUS_SCHEDULE[index])
    }
}

const fn create_pal_bus_schedule() -> [VicBusScheduleEntry; PAL_CYCLES_PER_RASTER_LINE as usize] {
    let mut schedule = [PLACEHOLDER_ENTRY; PAL_CYCLES_PER_RASTER_LINE as usize];
    let mut cycle = 1_u8;
    while cycle <= PAL_CYCLES_PER_RASTER_LINE {
        schedule[cycle as usize - 1] = create_bus_schedule_entry(PAL_VIC_TIMING, cycle);
        cycle += 1;
    }
    schedule
}

const fn create_ntsc_bus_schedule() -> [VicBusScheduleEntry; NTSC_CYCLES_PER_RASTER_LINE as usize] {
    let mut schedule = [PLACEHOLDER_ENTRY; NTSC_CYCLES_PER_RASTER_LINE as usize];
    let mut cycle = 1_u8;
    while cycle <= NTSC_CYCLES_PER_RASTER_LINE {
        schedule[cycle as usize - 1] = create_bus_schedule_entry(NTSC_VIC_TIMING, cycle);
        cycle += 1;
    }
    schedule
}

const fn create_bus_schedule_entry(timing: VicTiming, cycle: u8) -> VicBusScheduleEntry {
    let mut entry = VicBusScheduleEntry {
        cycle,
        phi1: default_phi1_fetch(timing, cycle),
        phi2: if cycle >= timing.fetch.matrix_first_cycle && cycle <= timing.fetch.matrix_last_cycle
        {
            Some(VicPhi2Fetch::Matrix)
        } else {
            None
        },
    };
    let mut sprite_index = 0_u8;
    while sprite_index < SPRITE_COUNT {
        let pointer_cycle = wrap_cycle(
            timing.sprite.data_first_cycle + sprite_index * timing.sprite.start_cycle_spacing,
            timing.cycles_per_raster_line,
        );
        let remaining_cycle = wrap_cycle(pointer_cycle + 1, timing.cycles_per_raster_line);
        if cycle == pointer_cycle {
            entry.phi1 = VicPhi1Fetch::SpritePointer { sprite_index };
            entry.phi2 = Some(VicPhi2Fetch::SpriteData {
                sprite_index,
                byte_index: 0,
            });
        } else if cycle == remaining_cycle {
            entry.phi1 = VicPhi1Fetch::SpriteData {
                sprite_index,
                byte_index: 1,
            };
            entry.phi2 = Some(VicPhi2Fetch::SpriteData {
                sprite_index,
                byte_index: 2,
            });
        }
        sprite_index += 1;
    }
    entry
}

const fn default_phi1_fetch(timing: VicTiming, cycle: u8) -> VicPhi1Fetch {
    if cycle >= timing.fetch.refresh_first_cycle && cycle <= timing.fetch.refresh_last_cycle {
        VicPhi1Fetch::Refresh
    } else if cycle >= timing.fetch.graphics_first_cycle
        && cycle <= timing.fetch.graphics_last_cycle
    {
        VicPhi1Fetch::Graphics
    } else {
        VicPhi1Fetch::Idle
    }
}

const fn wrap_cycle(cycle: u8, cycles_per_raster_line: u8) -> u8 {
    (cycle - 1) % cycles_per_raster_line + 1
}

#[cfg(test)]
mod tests {
    use super::{NTSC_BUS_SCHEDULE, VicPhi1Fetch, VicPhi2Fetch, vic_bus_schedule_for_cycle};

    #[test]
    fn pal_schedule_assigns_every_phi1_and_expected_phi2_window() {
        for cycle in 1..=63 {
            let entry = vic_bus_schedule_for_cycle(cycle).unwrap();
            assert_eq!(entry.cycle, cycle);
            if (15..=54).contains(&cycle) {
                assert_eq!(entry.phi2, Some(VicPhi2Fetch::Matrix));
            }
        }
        assert_eq!(
            vic_bus_schedule_for_cycle(58).unwrap().phi1,
            VicPhi1Fetch::SpritePointer { sprite_index: 0 }
        );
        assert_eq!(
            vic_bus_schedule_for_cycle(1).unwrap().phi1,
            VicPhi1Fetch::SpritePointer { sprite_index: 3 }
        );
        assert_eq!(
            vic_bus_schedule_for_cycle(2).unwrap().phi1,
            VicPhi1Fetch::SpriteData {
                sprite_index: 3,
                byte_index: 1
            }
        );
        assert!(vic_bus_schedule_for_cycle(0).is_err());
        assert!(vic_bus_schedule_for_cycle(64).is_err());
    }

    #[test]
    fn ntsc_schedule_matches_vice_6567r8_extra_idle_and_sprite_cycles() {
        assert_eq!(NTSC_BUS_SCHEDULE[9].phi1, VicPhi1Fetch::Idle);
        assert_eq!(NTSC_BUS_SCHEDULE[57].phi1, VicPhi1Fetch::Idle);
        assert_eq!(
            NTSC_BUS_SCHEDULE[58].phi1,
            VicPhi1Fetch::SpritePointer { sprite_index: 0 }
        );
        assert_eq!(
            NTSC_BUS_SCHEDULE[64].phi1,
            VicPhi1Fetch::SpritePointer { sprite_index: 3 }
        );
    }
}
