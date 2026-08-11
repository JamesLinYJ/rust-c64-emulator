// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 寄存器与芯片状态
//
//   文件:       device.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use super::{
    C64_PALETTE, NTSC_FIRST_VISIBLE_RASTER, NTSC_LAST_VISIBLE_RASTER_EXCLUSIVE,
    NTSC_RASTER_OUTPUT_HEIGHT, NTSC_VIC_TIMING, PAL_CYCLES_PER_RASTER_LINE,
    PAL_FIRST_VISIBLE_RASTER, PAL_LAST_VISIBLE_RASTER_EXCLUSIVE, PAL_RASTER_OUTPUT_HEIGHT,
    PAL_VIC_TIMING, SPRITE_COUNT, VIC_RASTER_OUTPUT_WIDTH, VicBorderController, VicBorderSignals,
    VicCycleResult, VicCycleSequencer, VicCycleSignals, VicFetchError, VicFetchPipeline,
    VicFetchRegisters, VicFetchSnapshot, VicMemoryBus, VicPixelError, VicPixelModes,
    VicPixelPipeline, VicPixelRegisters, VicSprite, VicTiming,
};

pub const VIC_REGISTER_COUNT: usize = 0x40;

const SPRITE_X_MOST_SIGNIFICANT_BITS: u8 = 0x10;
const SCREEN_CONTROL_1: u8 = 0x11;
const RASTER_COUNTER: u8 = 0x12;
const LIGHT_PEN_X: u8 = 0x13;
const LIGHT_PEN_Y: u8 = 0x14;
const SPRITE_ENABLE: u8 = 0x15;
const SCREEN_CONTROL_2: u8 = 0x16;
const SPRITE_EXPAND_VERTICAL: u8 = 0x17;
const MEMORY_POINTERS: u8 = 0x18;
const INTERRUPT_STATUS: u8 = 0x19;
const INTERRUPT_MASK: u8 = 0x1a;
const SPRITE_PRIORITY: u8 = 0x1b;
const SPRITE_MULTICOLOR_ENABLE: u8 = 0x1c;
const SPRITE_EXPAND_HORIZONTAL: u8 = 0x1d;
const SPRITE_SPRITE_COLLISION: u8 = 0x1e;
const SPRITE_FOREGROUND_COLLISION: u8 = 0x1f;
const BORDER_COLOR: u8 = 0x20;
const BACKGROUND_COLOR_0: u8 = 0x21;
const BACKGROUND_COLOR_3: u8 = 0x24;
const SPRITE_MULTICOLOR_0: u8 = 0x25;
const SPRITE_MULTICOLOR_1: u8 = 0x26;
const SPRITE_COLOR_0: u8 = 0x27;
const SPRITE_COLOR_7: u8 = 0x2e;
const FIRST_UNUSED_REGISTER: u8 = 0x2f;

const ROW_SELECT: u8 = 1 << 3;
const DISPLAY_ENABLE: u8 = 1 << 4;
const BITMAP_MODE: u8 = 1 << 5;
const EXTENDED_BACKGROUND_MODE: u8 = 1 << 6;
const RASTER_COUNTER_HIGH: u8 = 1 << 7;
const COLUMN_SELECT: u8 = 1 << 3;
const MULTICOLOR_MODE: u8 = 1 << 4;

const INTERRUPT_RASTER: u8 = 1 << 0;
const INTERRUPT_SPRITE_FOREGROUND: u8 = 1 << 1;
const INTERRUPT_SPRITE_SPRITE: u8 = 1 << 2;
const INTERRUPT_LIGHT_PEN: u8 = 1 << 3;
const INTERRUPT_ANY: u8 = 1 << 7;
const INTERRUPT_SOURCES: u8 = 0x0f;

const COLOR_MASK: u8 = 0x0f;
const SCROLL_MASK: u8 = 0x07;
const RASTER_HIGH: u16 = 0x0100;

const LIGHT_PEN_LATCHED: u8 = 1 << 0;
const LIGHT_PEN_INPUT_HIGH: u8 = 1 << 1;
const STANDARD_COLUMN_MODE: u8 = 1 << 0;
const STANDARD_ROW_MODE: u8 = 1 << 1;
#[cfg(test)]
const PAL_FRAME_PIXEL_COUNT: usize = VIC_RASTER_OUTPUT_WIDTH * PAL_RASTER_OUTPUT_HEIGHT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicRasterLineSnapshot {
    pub border_colors: [u32; PAL_CYCLES_PER_RASTER_LINE as usize],
    pub border_pixel_masks: [u8; PAL_CYCLES_PER_RASTER_LINE as usize],
    pub fetch: VicFetchSnapshot,
    pub pixels: [u32; VIC_RASTER_OUTPUT_WIDTH],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VicError {
    Fetch(VicFetchError),
    Pixel(VicPixelError),
    RasterTargetTooSmall { available: usize, required: usize },
}

impl fmt::Display for VicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(error) => error.fmt(formatter),
            Self::Pixel(error) => error.fmt(formatter),
            Self::RasterTargetTooSmall {
                available,
                required,
            } => write!(
                formatter,
                "VIC-II raster metadata requires {required} entries but target has {available}"
            ),
        }
    }
}

impl std::error::Error for VicError {}

impl From<VicFetchError> for VicError {
    fn from(error: VicFetchError) -> Self {
        Self::Fetch(error)
    }
}

impl From<VicPixelError> for VicError {
    fn from(error: VicPixelError) -> Self {
        Self::Pixel(error)
    }
}

/// MOS 6569R3/6567R8。寄存器、半周期取数、边框、像素和碰撞在同一芯片时钟提交。
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicII {
    registers: [u8; VIC_REGISTER_COUNT],
    cycle_sequencer: VicCycleSequencer,
    border_controller: VicBorderController,
    fetch_pipeline: VicFetchPipeline,
    pixel_pipeline: VicPixelPipeline,
    pixel_registers: VicPixelRegisters,
    frame_pixels: Box<[u32]>,
    frame_generation: u64,
    line_border_colors: [u32; PAL_CYCLES_PER_RASTER_LINE as usize],
    line_border_pixel_masks: [u8; PAL_CYCLES_PER_RASTER_LINE as usize],
    character_memory_address: u16,
    bitmap_memory_address: u16,
    screen_memory_address: u16,
    raster_position: u16,
    raster_trigger: u16,
    display_mode: u8,
    interrupt_mask: u8,
    interrupt_latches: u8,
    sprite_sprite_collision: u8,
    sprite_foreground_collision: u8,
    light_pen_flags: u8,
    border_mode_flags: u8,
    light_pen_trigger_cycles_remaining: u8,
    raster_interrupt_matched: bool,
}

impl Default for VicII {
    fn default() -> Self {
        Self::new()
    }
}

impl VicII {
    pub fn new() -> Self {
        Self::new_with_timing(PAL_VIC_TIMING)
    }

    pub fn new_with_timing(timing: VicTiming) -> Self {
        let mut result = Self {
            registers: [0; VIC_REGISTER_COUNT],
            cycle_sequencer: VicCycleSequencer::new_with_timing(timing),
            border_controller: VicBorderController::new(timing),
            fetch_pipeline: VicFetchPipeline::new(timing),
            pixel_pipeline: VicPixelPipeline::new(timing),
            pixel_registers: VicPixelRegisters::default(),
            frame_pixels: vec![C64_PALETTE[0]; frame_pixel_count(timing)].into_boxed_slice(),
            frame_generation: 0,
            line_border_colors: [C64_PALETTE[0]; PAL_CYCLES_PER_RASTER_LINE as usize],
            line_border_pixel_masks: [u8::MAX; PAL_CYCLES_PER_RASTER_LINE as usize],
            character_memory_address: 0,
            bitmap_memory_address: 0,
            screen_memory_address: 0,
            raster_position: 0,
            raster_trigger: 0,
            display_mode: 0,
            interrupt_mask: 0,
            interrupt_latches: 0,
            sprite_sprite_collision: 0,
            sprite_foreground_collision: 0,
            light_pen_flags: LIGHT_PEN_INPUT_HIGH,
            border_mode_flags: STANDARD_COLUMN_MODE | STANDARD_ROW_MODE,
            light_pen_trigger_cycles_remaining: 0,
            raster_interrupt_matched: false,
        };
        result.reset();
        result
    }

    pub const fn timing(&self) -> VicTiming {
        self.cycle_sequencer.timing()
    }

    pub fn frame_height(&self) -> usize {
        raster_output_height(self.timing())
    }

    pub(crate) fn state_matches_timing(&self, timing: VicTiming) -> bool {
        self.cycle_sequencer.timing() == timing
            && self.border_controller.timing() == timing
            && self.fetch_pipeline.timing() == timing
            && self.pixel_pipeline.timing() == timing
            && self.current_raster_cycle() <= timing.cycles_per_raster_line
            && self.current_raster_line() < timing.raster_line_count
            && self.frame_pixels.len() == frame_pixel_count(timing)
    }

    pub const fn ba_low(&self) -> bool {
        self.cycle_sequencer.ba_low()
    }

    pub const fn aec_low(&self) -> bool {
        self.cycle_sequencer.aec_low()
    }

    pub const fn bad_line(&self) -> bool {
        self.cycle_sequencer.bad_line()
    }

    pub const fn current_raster_cycle(&self) -> u8 {
        self.cycle_sequencer.cycle()
    }

    pub const fn current_raster_line(&self) -> u16 {
        self.raster_position
    }

    pub const fn sprite_dma_mask(&self) -> u8 {
        self.cycle_sequencer.sprite_dma_mask()
    }

    pub const fn phi1_data_bus_value(&self) -> u8 {
        self.fetch_pipeline.phi1_data_bus_value()
    }

    pub const fn interrupt_pending(&self) -> bool {
        self.interrupt_latches & self.interrupt_mask & INTERRUPT_SOURCES != 0
    }

    pub const fn sprites(&self) -> &[VicSprite; SPRITE_COUNT as usize] {
        &self.pixel_registers.sprites
    }

    pub const fn pixels(&self) -> &[u32; VIC_RASTER_OUTPUT_WIDTH] {
        self.pixel_pipeline.pixels()
    }

    /// 最近一个已完成或正在生成的可见帧。每条扫描线仅在完整提交后替换。
    pub fn frame_pixels(&self) -> &[u32] {
        &self.frame_pixels
    }

    /// 完整提交的帧数；调用方可用它实现无轮询竞态的粗粒度视频交换。
    pub const fn frame_generation(&self) -> u64 {
        self.frame_generation
    }

    pub const fn registers(&self) -> &[u8; VIC_REGISTER_COUNT] {
        &self.registers
    }

    pub const fn pixel_modes(&self) -> VicPixelModes {
        self.pixel_registers.modes
    }

    pub const fn standard_column_mode(&self) -> bool {
        self.border_mode_flags & STANDARD_COLUMN_MODE != 0
    }

    pub const fn standard_row_mode(&self) -> bool {
        self.border_mode_flags & STANDARD_ROW_MODE != 0
    }

    /// 推进一个完整 VIC-II 芯片周期。
    ///
    /// # Errors
    ///
    /// 内部半周期计划与取数/像素流水线不一致时返回显式不变量错误。
    pub fn clock_cycle<M: VicMemoryBus>(
        &mut self,
        memory: &mut M,
    ) -> Result<VicCycleResult, VicError> {
        let signals = VicCycleSignals {
            display_enabled: self.screen_visible(),
            sprite_enable_mask: self.registers[usize::from(SPRITE_ENABLE)],
            sprite_vertical_expansion_mask: self.registers[usize::from(SPRITE_EXPAND_VERTICAL)],
            vertical_scroll: self.vertical_scroll(),
            sprite_y: self.pixel_registers.sprites.map(|sprite| sprite.y),
        };
        let cycle = self.cycle_sequencer.tick(&signals);
        self.fetch_pipeline.execute_cycle(
            &cycle,
            VicFetchRegisters::new(
                self.bitmap_memory_address,
                self.display_mode & 0x02 != 0,
                self.character_memory_address,
                self.display_mode & 0x04 != 0,
                self.screen_memory_address,
            ),
            memory,
        )?;
        let border_pixel_mask = self.border_controller.tick(VicBorderSignals {
            column_select: self.standard_column_mode(),
            display_enabled: self.screen_visible(),
            raster_cycle: cycle.cycle,
            raster_line: cycle.raster_line,
            row_select: self.standard_row_mode(),
        });
        let cycle_index = usize::from(cycle.cycle - 1);
        if let (Some(mask), Some(color)) = (
            self.line_border_pixel_masks.get_mut(cycle_index),
            self.line_border_colors.get_mut(cycle_index),
        ) {
            *mask = border_pixel_mask;
            *color = self.pixel_registers.border_color;
        }
        let collisions = self.pixel_pipeline.clock_cycle(
            &cycle,
            border_pixel_mask,
            &self.pixel_registers,
            &self.fetch_pipeline,
        )?;
        self.record_sprite_sprite_collision(collisions.sprite_sprite_mask);
        self.record_sprite_foreground_collision(collisions.sprite_foreground_mask);
        self.capture_completed_raster_line(cycle.completed_raster_line)?;
        self.raster_position = cycle.raster_line;
        if cycle.frame_started() {
            self.set_light_pen_flag(LIGHT_PEN_LATCHED, false);
            if !self.light_pen_flag(LIGHT_PEN_INPUT_HIGH) {
                self.light_pen_trigger_cycles_remaining =
                    self.timing().light_pen.trigger_delay_cycles;
            }
        }
        self.clock_light_pen_trigger(cycle.cycle, cycle.raster_line);
        self.update_raster_interrupt_comparison();
        Ok(cycle)
    }

    pub fn reset(&mut self) {
        let light_pen_input_high = self.light_pen_flag(LIGHT_PEN_INPUT_HIGH);
        self.registers.fill(0);
        self.pixel_registers = VicPixelRegisters::default();
        self.character_memory_address = 0;
        self.bitmap_memory_address = 0;
        self.screen_memory_address = 0;
        self.raster_position = 0;
        self.raster_trigger = 0;
        self.display_mode = 0;
        self.interrupt_mask = 0;
        self.interrupt_latches = 0;
        self.sprite_sprite_collision = 0;
        self.sprite_foreground_collision = 0;
        self.light_pen_flags = if light_pen_input_high {
            LIGHT_PEN_INPUT_HIGH
        } else {
            0
        };
        self.border_mode_flags = STANDARD_COLUMN_MODE | STANDARD_ROW_MODE;
        self.light_pen_trigger_cycles_remaining = if light_pen_input_high {
            0
        } else {
            self.timing().light_pen.trigger_delay_cycles
        };
        self.raster_interrupt_matched = false;
        self.cycle_sequencer.reset();
        self.border_controller.reset();
        self.fetch_pipeline.reset();
        self.pixel_pipeline.reset(self.pixel_registers.border_color);
        self.frame_pixels.fill(self.pixel_registers.border_color);
        self.frame_generation = 0;
        self.line_border_colors
            .fill(self.pixel_registers.border_color);
        self.line_border_pixel_masks.fill(u8::MAX);
    }

    pub fn read_register(&mut self, address: u16) -> u8 {
        let register = address.to_le_bytes()[0] & 0x3f;
        if register >= FIRST_UNUSED_REGISTER {
            return u8::MAX;
        }
        match register {
            SCREEN_CONTROL_1 => {
                (self.registers[usize::from(register)] & !RASTER_COUNTER_HIGH)
                    | ((self.raster_position >> 1).to_le_bytes()[0] & RASTER_COUNTER_HIGH)
            }
            RASTER_COUNTER => self.raster_position.to_le_bytes()[0],
            SCREEN_CONTROL_2 => self.registers[usize::from(register)] | 0xc0,
            MEMORY_POINTERS => self.registers[usize::from(register)] | 0x01,
            INTERRUPT_STATUS => self.read_interrupt_status(),
            INTERRUPT_MASK => self.interrupt_mask | 0xf0,
            SPRITE_SPRITE_COLLISION => self.read_sprite_collision(true),
            SPRITE_FOREGROUND_COLLISION => self.read_sprite_collision(false),
            BORDER_COLOR..=SPRITE_COLOR_7 => self.registers[usize::from(register)] | 0xf0,
            _ => self.registers[usize::from(register)],
        }
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        let register = address.to_le_bytes()[0] & 0x3f;
        if register >= FIRST_UNUSED_REGISTER {
            return;
        }
        match register {
            0x00..=0x0f => self.write_sprite_coordinate(register, value),
            SPRITE_X_MOST_SIGNIFICANT_BITS => {
                self.registers[usize::from(register)] = value;
                self.for_each_sprite_bit(value, |sprite, set| {
                    sprite.x = if set {
                        sprite.x | RASTER_HIGH
                    } else {
                        sprite.x & 0xff
                    };
                });
            }
            SCREEN_CONTROL_1 => self.write_screen_control_1(value),
            RASTER_COUNTER => {
                self.raster_trigger = (self.raster_trigger & RASTER_HIGH) | u16::from(value);
                self.update_raster_interrupt_comparison();
            }
            LIGHT_PEN_X | LIGHT_PEN_Y | SPRITE_SPRITE_COLLISION | SPRITE_FOREGROUND_COLLISION => {}
            SPRITE_ENABLE => {
                self.registers[usize::from(register)] = value;
                self.for_each_sprite_bit(value, VicSprite::set_enabled);
            }
            SCREEN_CONTROL_2 => self.write_screen_control_2(value),
            SPRITE_EXPAND_VERTICAL => {
                if self.registers[usize::from(register)] != value {
                    self.cycle_sequencer
                        .write_sprite_vertical_expansion_register(value);
                }
                self.registers[usize::from(register)] = value;
                self.for_each_sprite_bit(value, VicSprite::set_expand_vertical);
            }
            MEMORY_POINTERS => self.write_memory_pointers(value),
            INTERRUPT_STATUS => {
                self.interrupt_latches &= !(value & INTERRUPT_SOURCES);
            }
            INTERRUPT_MASK => {
                self.interrupt_mask = value & INTERRUPT_SOURCES;
                self.registers[usize::from(register)] = self.interrupt_mask | 0xf0;
            }
            SPRITE_PRIORITY => {
                self.registers[usize::from(register)] = value;
                self.for_each_sprite_bit(value, |sprite, set| sprite.set_foreground(!set));
            }
            SPRITE_MULTICOLOR_ENABLE => {
                self.registers[usize::from(register)] = value;
                self.for_each_sprite_bit(value, VicSprite::set_multicolor);
            }
            SPRITE_EXPAND_HORIZONTAL => {
                self.registers[usize::from(register)] = value;
                self.for_each_sprite_bit(value, VicSprite::set_expand_horizontal);
            }
            BORDER_COLOR..=SPRITE_COLOR_7 => {
                self.write_color_register(register, value);
            }
            _ => {
                self.registers[usize::from(register)] = value;
            }
        }
    }

    pub fn latch_light_pen(&mut self, x: u16, y: u16) {
        if self.light_pen_flag(LIGHT_PEN_LATCHED) {
            return;
        }
        self.set_light_pen_flag(LIGHT_PEN_LATCHED, true);
        self.registers[usize::from(LIGHT_PEN_X)] = (x / 2).to_le_bytes()[0];
        self.registers[usize::from(LIGHT_PEN_Y)] = y.to_le_bytes()[0];
        self.interrupt_latches |= INTERRUPT_LIGHT_PEN;
    }

    pub fn set_light_pen_input_high(&mut self, high: bool) {
        if self.light_pen_flag(LIGHT_PEN_INPUT_HIGH) == high {
            return;
        }
        self.set_light_pen_flag(LIGHT_PEN_INPUT_HIGH, high);
        if !high {
            self.light_pen_trigger_cycles_remaining = self.timing().light_pen.trigger_delay_cycles;
        }
    }

    pub const fn capture_raster_line_state(&self) -> VicRasterLineSnapshot {
        VicRasterLineSnapshot {
            border_colors: self.line_border_colors,
            border_pixel_masks: self.line_border_pixel_masks,
            fetch: self.fetch_pipeline.snapshot(),
            pixels: self.pixel_pipeline.snapshot(),
        }
    }

    /// # Errors
    ///
    /// Returns a capacity error unless both targets can hold a complete 63-cycle raster line.
    pub fn copy_raster_metadata_to(
        &self,
        colors: &mut [u32],
        masks: &mut [u8],
    ) -> Result<(), VicError> {
        let required = usize::from(self.timing().cycles_per_raster_line)
            .min(PAL_CYCLES_PER_RASTER_LINE as usize);
        if colors.len() < required {
            return Err(VicError::RasterTargetTooSmall {
                available: colors.len(),
                required,
            });
        }
        if masks.len() < required {
            return Err(VicError::RasterTargetTooSmall {
                available: masks.len(),
                required,
            });
        }
        colors[..required].copy_from_slice(&self.line_border_colors);
        masks[..required].copy_from_slice(&self.line_border_pixel_masks);
        Ok(())
    }

    fn capture_completed_raster_line(
        &mut self,
        completed_raster_line: Option<u16>,
    ) -> Result<(), VicError> {
        let Some(raster_line) = completed_raster_line else {
            return Ok(());
        };
        let timing = self.timing();
        let (first_visible_raster, last_visible_raster_exclusive) = visible_raster_range(timing);
        if (first_visible_raster..last_visible_raster_exclusive).contains(&raster_line) {
            let visible_line = usize::from(raster_line - first_visible_raster);
            self.pixel_pipeline.copy_pixels_to(
                &mut self.frame_pixels,
                visible_line * VIC_RASTER_OUTPUT_WIDTH,
            )?;
        }
        if raster_line + 1 == timing.raster_line_count {
            self.frame_generation = self.frame_generation.wrapping_add(1);
        }
        Ok(())
    }

    fn write_sprite_coordinate(&mut self, register: u8, value: u8) {
        let sprite_index = usize::from(register / 2);
        let sprite = &mut self.pixel_registers.sprites[sprite_index];
        if register & 1 == 0 {
            sprite.x = (sprite.x & RASTER_HIGH) | u16::from(value);
        } else {
            sprite.y = value;
        }
        self.registers[usize::from(register)] = value;
    }

    fn write_screen_control_1(&mut self, value: u8) {
        self.set_border_mode_flag(STANDARD_ROW_MODE, value & ROW_SELECT != 0);
        self.display_mode =
            (self.display_mode & 0x01) | ((value & (BITMAP_MODE | EXTENDED_BACKGROUND_MODE)) >> 4);
        self.raster_trigger =
            (self.raster_trigger & 0xff) | (u16::from(value & RASTER_COUNTER_HIGH) << 1);
        self.registers[usize::from(SCREEN_CONTROL_1)] = value & !RASTER_COUNTER_HIGH;
        self.refresh_pixel_modes();
        self.update_raster_interrupt_comparison();
    }

    fn write_screen_control_2(&mut self, value: u8) {
        self.set_border_mode_flag(STANDARD_COLUMN_MODE, value & COLUMN_SELECT != 0);
        self.display_mode = (self.display_mode & 0x06) | u8::from(value & MULTICOLOR_MODE != 0);
        self.registers[usize::from(SCREEN_CONTROL_2)] = value;
        self.pixel_registers.horizontal_scroll = value & SCROLL_MASK;
        self.refresh_pixel_modes();
    }

    fn write_memory_pointers(&mut self, value: u8) {
        self.registers[usize::from(MEMORY_POINTERS)] = value;
        self.character_memory_address = u16::from(value & 0x0e) << 10;
        self.bitmap_memory_address = u16::from(value & 0x08) << 10;
        self.screen_memory_address = u16::from(value & 0xf0) << 6;
    }

    fn write_color_register(&mut self, register: u8, value: u8) {
        let color_index = value & COLOR_MASK;
        let color = C64_PALETTE[usize::from(color_index)];
        self.registers[usize::from(register)] = color_index;
        match register {
            BORDER_COLOR => self.pixel_registers.border_color = color,
            BACKGROUND_COLOR_0..=BACKGROUND_COLOR_3 => {
                self.pixel_registers.background_colors
                    [usize::from(register - BACKGROUND_COLOR_0)] = color;
            }
            SPRITE_MULTICOLOR_0 => self.pixel_registers.sprite_multicolor_0 = color,
            SPRITE_MULTICOLOR_1 => self.pixel_registers.sprite_multicolor_1 = color,
            SPRITE_COLOR_0..=SPRITE_COLOR_7 => {
                self.pixel_registers.sprites[usize::from(register - SPRITE_COLOR_0)].color = color;
            }
            _ => {}
        }
    }

    fn read_interrupt_status(&self) -> u8 {
        let active = self.interrupt_latches & INTERRUPT_SOURCES;
        0x70 | active
            | if active & self.interrupt_mask != 0 {
                INTERRUPT_ANY
            } else {
                0
            }
    }

    fn read_sprite_collision(&mut self, sprite_collision: bool) -> u8 {
        let value = if sprite_collision {
            self.sprite_sprite_collision
        } else {
            self.sprite_foreground_collision
        };
        if sprite_collision {
            self.sprite_sprite_collision = 0;
            for sprite in &mut self.pixel_registers.sprites {
                sprite.set_collision_with_sprite(false);
            }
        } else {
            self.sprite_foreground_collision = 0;
            for sprite in &mut self.pixel_registers.sprites {
                sprite.set_collision_with_foreground(false);
            }
        }
        value
    }

    fn record_sprite_sprite_collision(&mut self, mask: u8) {
        if mask == 0 {
            return;
        }
        if self.sprite_sprite_collision == 0 {
            self.interrupt_latches |= INTERRUPT_SPRITE_SPRITE;
        }
        self.sprite_sprite_collision |= mask;
        for (index, sprite) in self.pixel_registers.sprites.iter_mut().enumerate() {
            if mask & (1_u8 << index) != 0 {
                sprite.set_collision_with_sprite(true);
            }
        }
    }

    fn record_sprite_foreground_collision(&mut self, mask: u8) {
        if mask == 0 {
            return;
        }
        if self.sprite_foreground_collision == 0 {
            self.interrupt_latches |= INTERRUPT_SPRITE_FOREGROUND;
        }
        self.sprite_foreground_collision |= mask;
        for (index, sprite) in self.pixel_registers.sprites.iter_mut().enumerate() {
            if mask & (1_u8 << index) != 0 {
                sprite.set_collision_with_foreground(true);
            }
        }
    }

    fn clock_light_pen_trigger(&mut self, raster_cycle: u8, raster_line: u16) {
        if self.light_pen_trigger_cycles_remaining == 0 {
            return;
        }
        self.light_pen_trigger_cycles_remaining -= 1;
        if self.light_pen_trigger_cycles_remaining != 0 || self.light_pen_flag(LIGHT_PEN_LATCHED) {
            return;
        }
        let timing = self.timing().light_pen;
        let horizontal_pixels = (timing.horizontal_origin_pixels + u16::from(raster_cycle - 1) * 8)
            % timing.horizontal_position_modulo_pixels;
        let horizontal_counter_pixels =
            horizontal_pixels - horizontal_pixels % timing.horizontal_counter_granularity_pixels;
        self.set_light_pen_flag(LIGHT_PEN_LATCHED, true);
        self.registers[usize::from(LIGHT_PEN_X)] = (horizontal_counter_pixels / 2).to_le_bytes()[0]
            .wrapping_add(timing.mos6569r3_register_offset);
        self.registers[usize::from(LIGHT_PEN_Y)] = raster_line.to_le_bytes()[0];
        self.interrupt_latches |= INTERRUPT_LIGHT_PEN;
    }

    fn update_raster_interrupt_comparison(&mut self) {
        let matched = self.raster_position == self.raster_trigger;
        if matched && !self.raster_interrupt_matched {
            self.interrupt_latches |= INTERRUPT_RASTER;
        }
        self.raster_interrupt_matched = matched;
    }

    fn refresh_pixel_modes(&mut self) {
        self.pixel_registers.modes =
            VicPixelModes::from_display_mode(self.display_mode, self.screen_visible());
    }

    const fn screen_visible(&self) -> bool {
        self.registers[SCREEN_CONTROL_1 as usize] & DISPLAY_ENABLE != 0
    }

    const fn vertical_scroll(&self) -> u8 {
        self.registers[SCREEN_CONTROL_1 as usize] & SCROLL_MASK
    }

    fn for_each_sprite_bit(&mut self, value: u8, mut apply: impl FnMut(&mut VicSprite, bool)) {
        for (index, sprite) in self.pixel_registers.sprites.iter_mut().enumerate() {
            apply(sprite, value & (1_u8 << index) != 0);
        }
    }

    const fn light_pen_flag(&self, flag: u8) -> bool {
        self.light_pen_flags & flag != 0
    }

    fn set_light_pen_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.light_pen_flags |= flag;
        } else {
            self.light_pen_flags &= !flag;
        }
    }

    fn set_border_mode_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.border_mode_flags |= flag;
        } else {
            self.border_mode_flags &= !flag;
        }
    }
}

fn visible_raster_range(timing: VicTiming) -> (u16, u16) {
    if timing == NTSC_VIC_TIMING {
        (
            NTSC_FIRST_VISIBLE_RASTER,
            NTSC_LAST_VISIBLE_RASTER_EXCLUSIVE,
        )
    } else {
        (PAL_FIRST_VISIBLE_RASTER, PAL_LAST_VISIBLE_RASTER_EXCLUSIVE)
    }
}

fn raster_output_height(timing: VicTiming) -> usize {
    if timing == NTSC_VIC_TIMING {
        NTSC_RASTER_OUTPUT_HEIGHT
    } else {
        PAL_RASTER_OUTPUT_HEIGHT
    }
}

fn frame_pixel_count(timing: VicTiming) -> usize {
    VIC_RASTER_OUTPUT_WIDTH * raster_output_height(timing)
}

#[cfg(test)]
mod tests {
    use super::{INTERRUPT_ANY, PAL_FRAME_PIXEL_COUNT, VicII};
    use crate::devices::vic::{C64_PALETTE, PAL_VIC_TIMING, VicMemoryBus};

    #[derive(Default)]
    struct ZeroMemory;

    impl VicMemoryBus for ZeroMemory {
        fn cpu_data_bus_value(&self) -> u8 {
            0xff
        }

        fn read_vic_byte(&mut self, _address_in_bank: u16) -> u8 {
            0
        }

        fn read_vic_color(&mut self, _index: u16) -> u8 {
            0
        }
    }

    #[test]
    fn register_mirrors_and_raster_irq_follow_vic_rules() {
        let mut vic = VicII::new();
        let mut memory = ZeroMemory;
        vic.write_register(0xd012, 1);
        vic.write_register(0xd01a, 1);
        for _ in 0..64 {
            vic.clock_cycle(&mut memory).unwrap();
        }
        assert_eq!(vic.current_raster_line(), 1);
        assert_eq!(vic.read_register(0xd012), 1);
        assert_ne!(vic.read_register(0xd019) & INTERRUPT_ANY, 0);
        vic.write_register(0xd019, 1);
        assert_eq!(vic.read_register(0xd019) & INTERRUPT_ANY, 0);
        assert_eq!(vic.read_register(0xd052), 1);
    }

    #[test]
    fn collision_registers_clear_on_read_but_irq_latches_require_acknowledgement() {
        let mut vic = VicII::new();
        vic.record_sprite_sprite_collision(0x03);
        assert_eq!(vic.read_register(0xd01e), 0x03);
        assert_eq!(vic.read_register(0xd01e), 0);
        assert_ne!(vic.read_register(0xd019) & 0x04, 0);
        vic.write_register(0xd019, 0x04);
        assert_eq!(vic.read_register(0xd019) & 0x04, 0);
    }

    #[test]
    fn visible_lines_commit_into_a_generation_tracked_frame_buffer() {
        let mut vic = VicII::new();
        let mut memory = ZeroMemory;
        vic.write_register(0xd020, 2);
        let frame_cycles = u32::from(PAL_VIC_TIMING.cycles_per_raster_line)
            * u32::from(PAL_VIC_TIMING.raster_line_count);
        for _ in 0..frame_cycles {
            vic.clock_cycle(&mut memory).unwrap();
        }

        assert_eq!(vic.frame_generation(), 1);
        assert_eq!(vic.frame_pixels().len(), PAL_FRAME_PIXEL_COUNT);
        assert!(
            vic.frame_pixels()
                .iter()
                .all(|pixel| *pixel == C64_PALETTE[2])
        );
    }
}
