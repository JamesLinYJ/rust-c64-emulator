// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VIC-II raster, BA/AEC and sprite DMA sequencer
//
//   File:       vic/sequencer.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use super::bad_line::{VicBadLineController, VicBadLineSignals, VicMatrixAccess};
use super::bus_schedule::{PAL_BUS_SCHEDULE, VicBusScheduleEntry, VicPhi1Fetch, VicPhi2Fetch};
use super::sprite_dma::VicSpriteDma;
use super::timing::PAL_VIC_TIMING;
use super::{PAL_CYCLES_PER_RASTER_LINE, SPRITE_COUNT};

const RESULT_AEC_LOW: u16 = 1 << 0;
const RESULT_BA_LOW: u16 = 1 << 1;
const RESULT_BAD_LINE: u16 = 1 << 2;
const RESULT_BAD_LINE_CONDITION: u16 = 1 << 3;
const RESULT_ENTER_DISPLAY_STATE: u16 = 1 << 4;
const RESULT_FRAME_STARTED: u16 = 1 << 5;
const RESULT_LINE_STARTED: u16 = 1 << 6;
const RESULT_RESET_ROW_COUNTER: u16 = 1 << 7;
const STATE_AEC_LOW: u8 = 1 << 0;
const STATE_BA_LOW: u8 = 1 << 1;
const STATE_BAD_LINE: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicCycleSignals {
    pub display_enabled: bool,
    pub sprite_enable_mask: u8,
    pub sprite_vertical_expansion_mask: u8,
    pub vertical_scroll: u8,
    pub sprite_y: [u8; SPRITE_COUNT as usize],
}

impl Default for VicCycleSignals {
    fn default() -> Self {
        Self {
            display_enabled: false,
            sprite_enable_mask: 0,
            sprite_vertical_expansion_mask: 0,
            vertical_scroll: 0,
            sprite_y: [0; SPRITE_COUNT as usize],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicCycleResult {
    flags: u16,
    pub bus_schedule: VicBusScheduleEntry,
    pub completed_raster_line: Option<u16>,
    pub cycle: u8,
    pub late_video_counter_reload_column: Option<u8>,
    pub matrix_access: Option<VicMatrixAccess>,
    pub raster_line: u16,
    pub sprite_data_offsets: [Option<u8>; 2],
    pub sprite_display_mask: u8,
    pub sprite_dma_mask: u8,
}

impl VicCycleResult {
    pub const fn aec_low(self) -> bool {
        self.flags & RESULT_AEC_LOW != 0
    }

    pub const fn ba_low(self) -> bool {
        self.flags & RESULT_BA_LOW != 0
    }

    pub const fn bad_line(self) -> bool {
        self.flags & RESULT_BAD_LINE != 0
    }

    pub const fn bad_line_condition(self) -> bool {
        self.flags & RESULT_BAD_LINE_CONDITION != 0
    }

    pub const fn enter_display_state(self) -> bool {
        self.flags & RESULT_ENTER_DISPLAY_STATE != 0
    }

    pub const fn frame_started(self) -> bool {
        self.flags & RESULT_FRAME_STARTED != 0
    }

    pub const fn line_started(self) -> bool {
        self.flags & RESULT_LINE_STARTED != 0
    }

    pub const fn reset_row_counter(self) -> bool {
        self.flags & RESULT_RESET_ROW_COUNTER != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VicCycleSequencer {
    bad_line_controller: VicBadLineController,
    sprite_dma: [VicSpriteDma; SPRITE_COUNT as usize],
    sprite_ba_mask_by_cycle: [u8; PAL_CYCLES_PER_RASTER_LINE as usize + 1],
    state_flags: u8,
    current_sprite_display_mask: u8,
    current_sprite_dma_mask: u8,
    cycle_position: u8,
    raster_position: u16,
}

impl Default for VicCycleSequencer {
    fn default() -> Self {
        Self::new()
    }
}

impl VicCycleSequencer {
    pub const fn new() -> Self {
        Self {
            bad_line_controller: VicBadLineController::new(PAL_VIC_TIMING),
            sprite_dma: [VicSpriteDma::new(); SPRITE_COUNT as usize],
            sprite_ba_mask_by_cycle: build_sprite_ba_mask_table(),
            state_flags: 0,
            current_sprite_display_mask: 0,
            current_sprite_dma_mask: 0,
            cycle_position: 0,
            raster_position: 0,
        }
    }

    pub const fn aec_low(&self) -> bool {
        self.state_flag(STATE_AEC_LOW)
    }

    pub const fn ba_low(&self) -> bool {
        self.state_flag(STATE_BA_LOW)
    }

    pub const fn bad_line(&self) -> bool {
        self.state_flag(STATE_BAD_LINE)
    }

    pub const fn cycle(&self) -> u8 {
        self.cycle_position
    }

    pub const fn raster_line(&self) -> u16 {
        self.raster_position
    }

    pub const fn sprite_dma_mask(&self) -> u8 {
        self.current_sprite_dma_mask
    }

    pub const fn sprite_display_mask(&self) -> u8 {
        self.current_sprite_display_mask
    }

    pub fn write_sprite_vertical_expansion_register(&mut self, value: u8) {
        let apply_counter_crunch =
            self.cycle_position == PAL_VIC_TIMING.sprite.memory_counter_crunch_cycle;
        for (index, sprite) in self.sprite_dma.iter_mut().enumerate() {
            if value & (1_u8 << index) == 0 {
                sprite.clear_vertical_expansion(apply_counter_crunch);
            }
        }
    }

    pub fn tick(&mut self, signals: &VicCycleSignals) -> VicCycleResult {
        let (line_started, frame_started) = self.advance_raster_position();
        let bus_schedule = PAL_BUS_SCHEDULE[usize::from(self.cycle_position - 1)];
        let bad_line_cycle = self.bad_line_controller.tick(VicBadLineSignals {
            cycle: self.cycle_position,
            display_enabled: signals.display_enabled,
            frame_started,
            line_started,
            raster_line: self.raster_position,
            vertical_scroll: signals.vertical_scroll,
        });
        self.set_state_flag(STATE_BAD_LINE, bad_line_cycle.active());
        self.update_sprite_dma(signals);
        let sprite_data_offsets = self.consume_scheduled_sprite_data(bus_schedule);
        let ba_low = bad_line_cycle.ba_low()
            || self.current_sprite_dma_mask
                & self.sprite_ba_mask_by_cycle[usize::from(self.cycle_position)]
                != 0;
        let aec_low = bad_line_cycle.aec_low() || sprite_data_offsets[1].is_some();
        self.set_state_flag(STATE_BA_LOW, ba_low);
        self.set_state_flag(STATE_AEC_LOW, aec_low);
        let completed_raster_line = if self.cycle_position == PAL_CYCLES_PER_RASTER_LINE {
            Some(self.raster_position)
        } else {
            None
        };

        let mut flags = 0;
        set_result_flag(&mut flags, RESULT_AEC_LOW, aec_low);
        set_result_flag(&mut flags, RESULT_BA_LOW, ba_low);
        set_result_flag(&mut flags, RESULT_BAD_LINE, bad_line_cycle.active());
        set_result_flag(
            &mut flags,
            RESULT_BAD_LINE_CONDITION,
            bad_line_cycle.condition(),
        );
        set_result_flag(
            &mut flags,
            RESULT_ENTER_DISPLAY_STATE,
            bad_line_cycle.enter_display_state(),
        );
        set_result_flag(&mut flags, RESULT_FRAME_STARTED, frame_started);
        set_result_flag(&mut flags, RESULT_LINE_STARTED, line_started);
        set_result_flag(
            &mut flags,
            RESULT_RESET_ROW_COUNTER,
            bad_line_cycle.reset_row_counter(),
        );
        VicCycleResult {
            flags,
            bus_schedule,
            completed_raster_line,
            cycle: self.cycle_position,
            late_video_counter_reload_column: bad_line_cycle.late_video_counter_reload_column,
            matrix_access: bad_line_cycle.matrix_access,
            raster_line: self.raster_position,
            sprite_data_offsets,
            sprite_display_mask: self.current_sprite_display_mask,
            sprite_dma_mask: self.current_sprite_dma_mask,
        }
    }

    pub fn reset(&mut self) {
        self.bad_line_controller.reset();
        self.state_flags = 0;
        self.current_sprite_display_mask = 0;
        self.current_sprite_dma_mask = 0;
        self.cycle_position = 0;
        self.raster_position = 0;
        for sprite in &mut self.sprite_dma {
            sprite.reset();
        }
    }

    fn advance_raster_position(&mut self) -> (bool, bool) {
        if self.cycle_position == 0 {
            self.cycle_position = 1;
            return (true, true);
        }
        if self.cycle_position == PAL_CYCLES_PER_RASTER_LINE {
            self.cycle_position = 1;
            self.raster_position = (self.raster_position + 1) % PAL_VIC_TIMING.raster_line_count;
            return (true, self.raster_position == 0);
        }
        self.cycle_position += 1;
        (false, false)
    }

    fn update_sprite_dma(&mut self, signals: &VicCycleSignals) {
        let mut dma_mask_changed = false;
        if self.cycle_position == PAL_VIC_TIMING.sprite.memory_counter_update_cycle {
            for sprite in &mut self.sprite_dma {
                sprite.update_memory_counter_base();
            }
            dma_mask_changed = true;
        }
        if PAL_VIC_TIMING
            .sprite
            .dma_check_cycles
            .contains(&self.cycle_position)
        {
            let raster_low = self.raster_position.to_le_bytes()[0];
            for (index, sprite) in self.sprite_dma.iter_mut().enumerate() {
                let bit = 1_u8 << index;
                if !sprite.active
                    && signals.sprite_enable_mask & bit != 0
                    && signals.sprite_y[index] == raster_low
                {
                    sprite.start();
                }
            }
            dma_mask_changed = true;
        }
        if self.cycle_position == PAL_VIC_TIMING.sprite.expansion_check_cycle {
            for (index, sprite) in self.sprite_dma.iter_mut().enumerate() {
                sprite.clock_vertical_expansion(
                    signals.sprite_vertical_expansion_mask & (1_u8 << index) != 0,
                );
            }
        }
        if self.cycle_position == PAL_VIC_TIMING.sprite.prepare_display_cycle {
            let raster_low = self.raster_position.to_le_bytes()[0];
            for (index, sprite) in self.sprite_dma.iter_mut().enumerate() {
                let bit = 1_u8 << index;
                sprite.prepare_display_row(
                    signals.sprite_enable_mask & bit != 0 && signals.sprite_y[index] == raster_low,
                );
            }
            self.current_sprite_display_mask = self.compose_sprite_display_mask();
        }
        if dma_mask_changed {
            self.current_sprite_dma_mask = self.compose_sprite_dma_mask();
        }
    }

    fn consume_scheduled_sprite_data(
        &mut self,
        bus_schedule: VicBusScheduleEntry,
    ) -> [Option<u8>; 2] {
        let phi1 = if let VicPhi1Fetch::SpriteData { sprite_index, .. } = bus_schedule.phi1 {
            self.sprite_dma[usize::from(sprite_index)].consume_data_byte()
        } else {
            None
        };
        let phi2 = if let Some(VicPhi2Fetch::SpriteData { sprite_index, .. }) = bus_schedule.phi2 {
            self.sprite_dma[usize::from(sprite_index)].consume_data_byte()
        } else {
            None
        };
        [phi1, phi2]
    }

    fn compose_sprite_dma_mask(&self) -> u8 {
        self.sprite_dma
            .iter()
            .enumerate()
            .fold(0, |mask, (index, sprite)| {
                if sprite.active {
                    mask | (1_u8 << index)
                } else {
                    mask
                }
            })
    }

    fn compose_sprite_display_mask(&self) -> u8 {
        self.sprite_dma
            .iter()
            .enumerate()
            .fold(0, |mask, (index, sprite)| {
                if sprite.display_active {
                    mask | (1_u8 << index)
                } else {
                    mask
                }
            })
    }

    const fn state_flag(&self, flag: u8) -> bool {
        self.state_flags & flag != 0
    }

    fn set_state_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.state_flags |= flag;
        } else {
            self.state_flags &= !flag;
        }
    }
}

const fn build_sprite_ba_mask_table() -> [u8; PAL_CYCLES_PER_RASTER_LINE as usize + 1] {
    let mut masks = [0_u8; PAL_CYCLES_PER_RASTER_LINE as usize + 1];
    let mut sprite_index = 0_u8;
    while sprite_index < SPRITE_COUNT {
        let first_cycle = wrap_cycle(
            PAL_VIC_TIMING.sprite.ba_first_cycle
                + sprite_index * PAL_VIC_TIMING.sprite.start_cycle_spacing,
        );
        let mut offset = 0_u8;
        while offset < PAL_VIC_TIMING.sprite.ba_cycle_count {
            let cycle = wrap_cycle(first_cycle + offset);
            masks[cycle as usize] |= 1_u8 << sprite_index;
            offset += 1;
        }
        sprite_index += 1;
    }
    masks
}

const fn wrap_cycle(cycle: u8) -> u8 {
    (cycle - 1) % PAL_CYCLES_PER_RASTER_LINE + 1
}

fn set_result_flag(flags: &mut u16, flag: u16, enabled: bool) {
    if enabled {
        *flags |= flag;
    }
}

#[cfg(test)]
mod tests {
    use super::{VicCycleSequencer, VicCycleSignals};

    #[test]
    fn raster_wraps_after_312_lines_of_63_cycles() {
        let mut sequencer = VicCycleSequencer::new();
        let signals = VicCycleSignals::default();
        let first = sequencer.tick(&signals);
        assert!(first.frame_started());
        assert!(first.line_started());
        assert_eq!((first.raster_line, first.cycle), (0, 1));
        for _ in 1..(312 * 63) {
            sequencer.tick(&signals);
        }
        let wrapped = sequencer.tick(&signals);
        assert!(wrapped.frame_started());
        assert_eq!((wrapped.raster_line, wrapped.cycle), (0, 1));
    }

    #[test]
    fn sprite_dma_asserts_ba_before_phi2_data_fetch() {
        let mut sequencer = VicCycleSequencer::new();
        let mut signals = VicCycleSignals {
            sprite_enable_mask: 1,
            ..VicCycleSignals::default()
        };
        signals.sprite_y[0] = 0;
        let mut saw_ba = false;
        let mut saw_aec = false;
        for _ in 0..63 {
            let result = sequencer.tick(&signals);
            saw_ba |= result.ba_low();
            saw_aec |= result.aec_low();
        }
        assert!(saw_ba);
        assert!(saw_aec);
        assert_ne!(sequencer.sprite_dma_mask(), 0);
    }
}
