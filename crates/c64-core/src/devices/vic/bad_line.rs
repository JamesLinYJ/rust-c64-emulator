// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VIC-II dynamic bad-line bus acquisition
//
//   File:       vic/bad_line.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use super::timing::{PAL_VIC_TIMING, VicTiming};

const CONTROLLER_ALLOW_BAD_LINES: u8 = 1 << 0;
const CONTROLLER_BAD_LINE_ACTIVE: u8 = 1 << 1;
const CONTROLLER_CONDITION_WAS_ACTIVE: u8 = 1 << 2;
const CONTROLLER_DISPLAY_ENTERED: u8 = 1 << 3;
const CYCLE_ACTIVE: u8 = 1 << 0;
const CYCLE_AEC_LOW: u8 = 1 << 1;
const CYCLE_BA_LOW: u8 = 1 << 2;
const CYCLE_CONDITION: u8 = 1 << 3;
const CYCLE_ENTER_DISPLAY: u8 = 1 << 4;
const CYCLE_RESET_ROW_COUNTER: u8 = 1 << 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VicMatrixAccessSource {
    CpuDataBus,
    VideoMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicMatrixAccess {
    pub column: u8,
    pub source: VicMatrixAccessSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicBadLineSignals {
    pub cycle: u8,
    pub display_enabled: bool,
    pub frame_started: bool,
    pub line_started: bool,
    pub raster_line: u16,
    pub vertical_scroll: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VicBadLineCycle {
    flags: u8,
    pub late_video_counter_reload_column: Option<u8>,
    pub matrix_access: Option<VicMatrixAccess>,
}

impl VicBadLineCycle {
    pub const fn active(self) -> bool {
        self.flags & CYCLE_ACTIVE != 0
    }

    pub const fn aec_low(self) -> bool {
        self.flags & CYCLE_AEC_LOW != 0
    }

    pub const fn ba_low(self) -> bool {
        self.flags & CYCLE_BA_LOW != 0
    }

    pub const fn condition(self) -> bool {
        self.flags & CYCLE_CONDITION != 0
    }

    pub const fn enter_display_state(self) -> bool {
        self.flags & CYCLE_ENTER_DISPLAY != 0
    }

    pub const fn reset_row_counter(self) -> bool {
        self.flags & CYCLE_RESET_ROW_COUNTER != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicBadLineController {
    timing: VicTiming,
    state_flags: u8,
    dma_start_cycle: Option<u8>,
}

impl Default for VicBadLineController {
    fn default() -> Self {
        Self::new(PAL_VIC_TIMING)
    }
}

impl VicBadLineController {
    pub const fn new(timing: VicTiming) -> Self {
        Self {
            timing,
            state_flags: 0,
            dma_start_cycle: None,
        }
    }

    pub const fn timing(&self) -> VicTiming {
        self.timing
    }

    pub(super) const fn timing_ref(&self) -> &VicTiming {
        &self.timing
    }

    pub const fn active(&self) -> bool {
        self.state_flag(CONTROLLER_BAD_LINE_ACTIVE)
    }

    pub fn reset(&mut self) {
        self.state_flags = 0;
        self.dma_start_cycle = None;
    }

    pub fn tick(&mut self, signals: VicBadLineSignals) -> VicBadLineCycle {
        if signals.frame_started {
            self.set_state_flag(CONTROLLER_ALLOW_BAD_LINES, false);
        }
        if signals.line_started {
            self.begin_raster_line();
        }
        if signals.raster_line == self.timing.bad_line.first_raster_line && signals.display_enabled
        {
            self.set_state_flag(CONTROLLER_ALLOW_BAD_LINES, true);
        }

        let condition = self.is_bad_line_condition(signals.raster_line, signals.vertical_scroll);
        if !condition
            && !self.state_flag(CONTROLLER_CONDITION_WAS_ACTIVE)
            && !self.active()
            && self.dma_start_cycle.is_none()
        {
            return VicBadLineCycle::default();
        }
        let condition_started = condition && !self.state_flag(CONTROLLER_CONDITION_WAS_ACTIVE);
        let condition_ended = !condition && self.state_flag(CONTROLLER_CONDITION_WAS_ACTIVE);
        let mut enter_display_state = false;
        let mut late_video_counter_reload_column = None;

        if condition_started {
            self.set_state_flag(CONTROLLER_BAD_LINE_ACTIVE, true);
            if !self.state_flag(CONTROLLER_DISPLAY_ENTERED) {
                enter_display_state = true;
                self.set_state_flag(CONTROLLER_DISPLAY_ENTERED, true);
            }
            if signals.cycle <= self.timing.fetch.matrix_last_cycle {
                let requested_start = signals.cycle.max(self.timing.bad_line.ba_first_cycle);
                if self.dma_start_cycle.is_none() {
                    self.dma_start_cycle = Some(requested_start);
                }
                if signals.cycle > self.timing.fetch.video_counter_reload_cycle {
                    late_video_counter_reload_column =
                        Some(self.matrix_column_for_cycle(signals.cycle));
                }
            }
        }

        if condition_ended && signals.cycle <= self.timing.fetch.video_counter_reload_cycle {
            self.set_state_flag(CONTROLLER_BAD_LINE_ACTIVE, false);
            self.dma_start_cycle = None;
        }

        let matrix_access = self.matrix_access_for_cycle(signals.cycle);
        let ba_low = self.is_dma_bus_request_cycle(signals.cycle);
        let aec_low = ba_low
            && self.dma_start_cycle.is_some_and(|start| {
                signals.cycle >= start.saturating_add(self.bus_acquisition_cycle_count())
            });
        let reset_row_counter =
            signals.cycle == self.timing.fetch.video_counter_reload_cycle && condition;
        self.set_state_flag(CONTROLLER_CONDITION_WAS_ACTIVE, condition);

        let mut flags = 0;
        set_flag(&mut flags, CYCLE_ACTIVE, self.active());
        set_flag(&mut flags, CYCLE_AEC_LOW, aec_low);
        set_flag(&mut flags, CYCLE_BA_LOW, ba_low);
        set_flag(&mut flags, CYCLE_CONDITION, condition);
        set_flag(&mut flags, CYCLE_ENTER_DISPLAY, enter_display_state);
        set_flag(&mut flags, CYCLE_RESET_ROW_COUNTER, reset_row_counter);
        VicBadLineCycle {
            flags,
            late_video_counter_reload_column,
            matrix_access,
        }
    }

    const fn bus_acquisition_cycle_count(&self) -> u8 {
        self.timing.fetch.matrix_first_cycle - self.timing.bad_line.ba_first_cycle
    }

    fn begin_raster_line(&mut self) {
        self.set_state_flag(CONTROLLER_BAD_LINE_ACTIVE, false);
        self.set_state_flag(CONTROLLER_CONDITION_WAS_ACTIVE, false);
        self.set_state_flag(CONTROLLER_DISPLAY_ENTERED, false);
        self.dma_start_cycle = None;
    }

    const fn is_bad_line_condition(&self, raster_line: u16, vertical_scroll: u8) -> bool {
        self.state_flag(CONTROLLER_ALLOW_BAD_LINES)
            && raster_line >= self.timing.bad_line.first_raster_line
            && raster_line <= self.timing.bad_line.last_raster_line
            && raster_line.to_le_bytes()[0] & 0x07 == vertical_scroll
    }

    fn is_dma_bus_request_cycle(self, cycle: u8) -> bool {
        self.dma_start_cycle
            .is_some_and(|start| cycle >= start && cycle <= self.timing.bad_line.ba_last_cycle)
    }

    fn matrix_access_for_cycle(self, cycle: u8) -> Option<VicMatrixAccess> {
        let start = self.dma_start_cycle?;
        if cycle < start.max(self.timing.fetch.matrix_first_cycle)
            || cycle > self.timing.fetch.matrix_last_cycle
        {
            return None;
        }
        Some(VicMatrixAccess {
            column: self.matrix_column_for_cycle(cycle),
            source: if cycle < start.saturating_add(self.bus_acquisition_cycle_count()) {
                VicMatrixAccessSource::CpuDataBus
            } else {
                VicMatrixAccessSource::VideoMemory
            },
        })
    }

    fn matrix_column_for_cycle(&self, cycle: u8) -> u8 {
        let last_column =
            self.timing.fetch.matrix_last_cycle - self.timing.fetch.matrix_first_cycle;
        cycle
            .saturating_sub(self.timing.fetch.matrix_first_cycle)
            .min(last_column)
    }

    const fn state_flag(&self, flag: u8) -> bool {
        self.state_flags & flag != 0
    }

    fn set_state_flag(&mut self, flag: u8, enabled: bool) {
        set_flag(&mut self.state_flags, flag, enabled);
    }
}

fn set_flag(flags: &mut u8, flag: u8, enabled: bool) {
    if enabled {
        *flags |= flag;
    } else {
        *flags &= !flag;
    }
}

#[cfg(test)]
mod tests {
    use super::{VicBadLineController, VicBadLineSignals, VicMatrixAccessSource};

    #[test]
    fn inactive_mid_line_cycle_is_an_exact_noop() {
        let mut controller = VicBadLineController::default();
        let initial = controller;
        let result = controller.tick(VicBadLineSignals {
            cycle: 30,
            display_enabled: false,
            frame_started: false,
            line_started: false,
            raster_line: 20,
            vertical_scroll: 0,
        });

        assert_eq!(result, super::VicBadLineCycle::default());
        assert_eq!(controller, initial);
    }

    #[test]
    fn normal_bad_line_requests_ba_three_cycles_before_video_memory() {
        let mut controller = VicBadLineController::default();
        for line in 0..=0x30 {
            for cycle in 1..=63 {
                let result = controller.tick(VicBadLineSignals {
                    cycle,
                    display_enabled: true,
                    frame_started: line == 0 && cycle == 1,
                    line_started: cycle == 1,
                    raster_line: line,
                    vertical_scroll: 0,
                });
                if line == 0x30 {
                    assert_eq!(result.ba_low(), (12..=54).contains(&cycle));
                    assert_eq!(result.aec_low(), (15..=54).contains(&cycle));
                    if cycle == 15 {
                        assert_eq!(
                            result.matrix_access.unwrap().source,
                            VicMatrixAccessSource::VideoMemory
                        );
                    }
                }
            }
        }
    }
}
