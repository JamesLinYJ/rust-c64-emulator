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

use super::{PAL_CYCLES_PER_RASTER_LINE, SPRITE_COUNT};
use crate::devices::vic::timing::PAL_VIC_TIMING;

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
}

impl fmt::Display for VicCycleRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "PAL VIC-II cycle must be in 1..={PAL_CYCLES_PER_RASTER_LINE}; received {}",
            self.cycle
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

/// Return the immutable PAL half-cycle assignment.
///
/// # Errors
///
/// Returns `VicCycleRangeError` unless `cycle` is in the architectural 1..=63 range.
pub fn vic_bus_schedule_for_cycle(cycle: u8) -> Result<VicBusScheduleEntry, VicCycleRangeError> {
    if cycle == 0 || cycle > PAL_CYCLES_PER_RASTER_LINE {
        return Err(VicCycleRangeError { cycle });
    }
    Ok(PAL_BUS_SCHEDULE[cycle as usize - 1])
}

const fn create_pal_bus_schedule() -> [VicBusScheduleEntry; PAL_CYCLES_PER_RASTER_LINE as usize] {
    let mut schedule = [PLACEHOLDER_ENTRY; PAL_CYCLES_PER_RASTER_LINE as usize];
    let mut cycle = 1_u8;
    while cycle <= PAL_CYCLES_PER_RASTER_LINE {
        schedule[cycle as usize - 1] = VicBusScheduleEntry {
            cycle,
            phi1: default_phi1_fetch(cycle),
            phi2: if cycle >= PAL_VIC_TIMING.fetch.matrix_first_cycle
                && cycle <= PAL_VIC_TIMING.fetch.matrix_last_cycle
            {
                Some(VicPhi2Fetch::Matrix)
            } else {
                None
            },
        };
        cycle += 1;
    }

    let mut sprite_index = 0_u8;
    while sprite_index < SPRITE_COUNT {
        let pointer_cycle = wrap_cycle(
            PAL_VIC_TIMING.sprite.data_first_cycle
                + sprite_index * PAL_VIC_TIMING.sprite.start_cycle_spacing,
        );
        let remaining_cycle = wrap_cycle(pointer_cycle + 1);
        schedule[pointer_cycle as usize - 1].phi1 = VicPhi1Fetch::SpritePointer { sprite_index };
        schedule[pointer_cycle as usize - 1].phi2 = Some(VicPhi2Fetch::SpriteData {
            sprite_index,
            byte_index: 0,
        });
        schedule[remaining_cycle as usize - 1].phi1 = VicPhi1Fetch::SpriteData {
            sprite_index,
            byte_index: 1,
        };
        schedule[remaining_cycle as usize - 1].phi2 = Some(VicPhi2Fetch::SpriteData {
            sprite_index,
            byte_index: 2,
        });
        sprite_index += 1;
    }
    schedule
}

const fn default_phi1_fetch(cycle: u8) -> VicPhi1Fetch {
    if cycle >= PAL_VIC_TIMING.fetch.refresh_first_cycle
        && cycle <= PAL_VIC_TIMING.fetch.refresh_last_cycle
    {
        VicPhi1Fetch::Refresh
    } else if cycle >= PAL_VIC_TIMING.fetch.graphics_first_cycle
        && cycle <= PAL_VIC_TIMING.fetch.graphics_last_cycle
    {
        VicPhi1Fetch::Graphics
    } else {
        // The only non-sprite, non-refresh, non-graphics cycles are 56 and 57.
        VicPhi1Fetch::Idle
    }
}

const fn wrap_cycle(cycle: u8) -> u8 {
    (cycle - 1) % PAL_CYCLES_PER_RASTER_LINE + 1
}

#[cfg(test)]
mod tests {
    use super::{VicPhi1Fetch, VicPhi2Fetch, vic_bus_schedule_for_cycle};

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
}
