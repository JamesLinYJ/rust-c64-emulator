// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 边框触发器
//
//   文件:       border.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use super::timing::{PAL_VIC_TIMING, VicTiming};

const BORDER_ALL_PIXELS: u8 = 0xff;
const BORDER_FIRST_SEVEN_PIXELS: u8 = 0xfe;
const BORDER_LAST_PIXEL: u8 = 0x01;
const BORDER_NO_PIXELS: u8 = 0x00;

const VERTICAL_BORDER: u8 = 1 << 0;
const PENDING_VERTICAL_BORDER: u8 = 1 << 1;
const MAIN_BORDER: u8 = 1 << 2;
const RENDERED_BORDER: u8 = 1 << 3;
const RESET_STATE: u8 = VERTICAL_BORDER | PENDING_VERTICAL_BORDER | MAIN_BORDER | RENDERED_BORDER;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicBorderSignals {
    pub column_select: bool,
    pub display_enabled: bool,
    pub raster_cycle: u8,
    pub raster_line: u16,
    pub row_select: bool,
}

/// 6569/6567 的垂直、主边框触发器，以及 38 列模式下跨八像素组的输出锁存。
#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicBorderController {
    timing: VicTiming,
    state: u8,
}

impl Default for VicBorderController {
    fn default() -> Self {
        Self::new(PAL_VIC_TIMING)
    }
}

impl VicBorderController {
    pub const fn new(timing: VicTiming) -> Self {
        Self {
            timing,
            state: RESET_STATE,
        }
    }

    pub const fn timing(&self) -> VicTiming {
        self.timing
    }

    pub fn tick(&mut self, signals: VicBorderSignals) -> u8 {
        self.check_horizontal_border(signals);
        let pixel_mask = self.draw_pixel_mask(signals.column_select);

        // 顶边框触发器在本周期像素产生后更新，避免边界提前八个像素生效。
        self.check_vertical_border_top(signals);
        self.check_vertical_border_bottom(signals.raster_line, signals.row_select);
        if signals.raster_cycle == 1 {
            self.copy_pending_vertical_border();
        }
        pixel_mask
    }

    pub fn reset(&mut self) {
        self.state = RESET_STATE;
    }

    pub const fn main_border(&self) -> bool {
        self.flag(MAIN_BORDER)
    }

    pub const fn vertical_border(&self) -> bool {
        self.flag(VERTICAL_BORDER)
    }

    fn check_horizontal_border(&mut self, signals: VicBorderSignals) {
        let left_cycle = if signals.column_select {
            self.timing.border.standard_column_left_cycle
        } else {
            self.timing.border.reduced_column_left_cycle
        };
        if signals.raster_cycle == left_cycle {
            self.check_vertical_border_bottom(signals.raster_line, signals.row_select);
            self.copy_pending_vertical_border();
            if !self.flag(VERTICAL_BORDER) {
                self.set_flag(MAIN_BORDER, false);
            }
        }

        let right_cycle = if signals.column_select {
            self.timing.border.standard_column_right_cycle
        } else {
            self.timing.border.reduced_column_right_cycle
        };
        if signals.raster_cycle == right_cycle {
            self.set_flag(MAIN_BORDER, true);
        }
    }

    fn check_vertical_border_top(&mut self, signals: VicBorderSignals) {
        let start_line = if signals.row_select {
            self.timing.border.standard_row_start_line
        } else {
            self.timing.border.reduced_row_start_line
        };
        if signals.raster_line == start_line && signals.display_enabled {
            self.set_flag(VERTICAL_BORDER, false);
            self.set_flag(PENDING_VERTICAL_BORDER, false);
        }
    }

    fn check_vertical_border_bottom(&mut self, raster_line: u16, row_select: bool) {
        let stop_line = if row_select {
            self.timing.border.standard_row_stop_line
        } else {
            self.timing.border.reduced_row_stop_line
        };
        if raster_line == stop_line {
            self.set_flag(PENDING_VERTICAL_BORDER, true);
        }
    }

    fn draw_pixel_mask(&mut self, column_select: bool) -> u8 {
        let rendered = self.flag(RENDERED_BORDER);
        let main = self.flag(MAIN_BORDER);
        if !rendered && !main {
            return BORDER_NO_PIXELS;
        }
        if rendered && main {
            return BORDER_ALL_PIXELS;
        }

        if column_select {
            let mask = if rendered {
                BORDER_ALL_PIXELS
            } else {
                BORDER_NO_PIXELS
            };
            self.set_flag(RENDERED_BORDER, main);
            return mask;
        }

        let mask = (if rendered {
            BORDER_FIRST_SEVEN_PIXELS
        } else {
            BORDER_NO_PIXELS
        }) | (if main {
            BORDER_LAST_PIXEL
        } else {
            BORDER_NO_PIXELS
        });
        self.set_flag(RENDERED_BORDER, main);
        mask
    }

    fn copy_pending_vertical_border(&mut self) {
        self.set_flag(VERTICAL_BORDER, self.flag(PENDING_VERTICAL_BORDER));
    }

    const fn flag(&self, flag: u8) -> bool {
        self.state & flag != 0
    }

    fn set_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.state |= flag;
        } else {
            self.state &= !flag;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{VicBorderController, VicBorderSignals};

    #[test]
    fn standard_border_opens_and_closes_at_pal_cycles() {
        let mut border = VicBorderController::default();
        let mut signals = VicBorderSignals {
            column_select: true,
            display_enabled: true,
            raster_cycle: 1,
            raster_line: 0x33,
            row_select: true,
        };
        assert_eq!(border.tick(signals), 0xff);
        signals.raster_cycle = 17;
        assert_eq!(border.tick(signals), 0xff);
        assert!(!border.main_border());
        signals.raster_cycle = 18;
        assert_eq!(border.tick(signals), 0x00);
        signals.raster_cycle = 57;
        assert_eq!(border.tick(signals), 0x00);
        assert!(border.main_border());
        signals.raster_cycle = 58;
        assert_eq!(border.tick(signals), 0xff);
    }

    #[test]
    fn reduced_width_border_changes_on_the_last_pixel() {
        let mut border = VicBorderController::default();
        let mut signals = VicBorderSignals {
            column_select: false,
            display_enabled: true,
            raster_cycle: 1,
            raster_line: 0x37,
            row_select: false,
        };
        border.tick(signals);
        signals.raster_cycle = 18;
        assert_eq!(border.tick(signals), 0xfe);
        signals.raster_cycle = 19;
        assert_eq!(border.tick(signals), 0x00);
        signals.raster_cycle = 56;
        assert_eq!(border.tick(signals), 0x01);
    }
}
