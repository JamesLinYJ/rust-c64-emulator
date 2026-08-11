// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 逐周期像素管线
//
//   文件:       pixel.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use super::fetch::{VIC_SPRITE_COUNT, VicFetchError, VicFetchPipeline};
use super::sequencer::VicCycleResult;
use super::sprite::VicSprite;
use super::timing::{PAL_VIC_TIMING, VicTiming};

pub const VIC_RASTER_OUTPUT_WIDTH: usize = 403;
const VIC_RASTER_OUTPUT_WIDTH_U16: u16 = 403;
pub const C64_PALETTE: [u32; 16] = [
    pack_rgba_pixel(0x00, 0x00, 0x00),
    pack_rgba_pixel(0xff, 0xff, 0xff),
    pack_rgba_pixel(0xe0, 0x40, 0x40),
    pack_rgba_pixel(0x60, 0xff, 0xff),
    pack_rgba_pixel(0xe0, 0x60, 0xe0),
    pack_rgba_pixel(0x40, 0xe0, 0x40),
    pack_rgba_pixel(0x40, 0x40, 0xe0),
    pack_rgba_pixel(0xff, 0xff, 0x40),
    pack_rgba_pixel(0xe0, 0xa0, 0x40),
    pack_rgba_pixel(0x9c, 0x74, 0x48),
    pack_rgba_pixel(0xff, 0xa0, 0xa0),
    pack_rgba_pixel(0x54, 0x54, 0x54),
    pack_rgba_pixel(0x88, 0x88, 0x88),
    pack_rgba_pixel(0xa0, 0xff, 0xa0),
    pack_rgba_pixel(0xa0, 0xa0, 0xff),
    pack_rgba_pixel(0xc0, 0xc0, 0xc0),
];

const DEFAULT_SPRITE_COLORS: [u32; VIC_SPRITE_COUNT] = [
    C64_PALETTE[1],
    C64_PALETTE[2],
    C64_PALETTE[3],
    C64_PALETTE[4],
    C64_PALETTE[5],
    C64_PALETTE[6],
    C64_PALETTE[7],
    C64_PALETTE[12],
];
const PIXELS_PER_VIC_CYCLE: u16 = 8;
const TEXT_COLUMN_WIDTH: i32 = 8;
const TEXT_DISPLAY_WIDTH: i32 = 40 * TEXT_COLUMN_WIDTH;
const SPRITE_SOURCE_WIDTH: u16 = 24;
const DEFAULT_FIRST_VISIBLE_CYCLE: u16 = 12;
const TRANSPARENT_PIXEL: u32 = 0;
const VIC_X_COUNTER_MASK: u16 = 0x01ff;
const VIC_X_COUNTER_MODULUS: i32 = 0x0200;
const TEXT_DISPLAY_VIC_X: i32 = 0x18;
const VIC_X_ZERO_PHYSICAL_PIXEL: u16 = 112;

const MODE_BITMAP: u8 = 1 << 0;
const MODE_VALID: u8 = 1 << 1;
const MODE_EXTENDED_BACKGROUND: u8 = 1 << 2;
const MODE_MULTICOLOR: u8 = 1 << 3;
const MODE_SCREEN_VISIBLE: u8 = 1 << 4;

pub trait VicPixelDataSource {
    fn sprite_display_mask(&self) -> u8;
    /// # Errors
    ///
    /// Returns a fetch range error for a non-display column.
    fn screen_matrix_byte(&self, column: u8) -> Result<u8, VicFetchError>;
    /// # Errors
    ///
    /// Returns a fetch range error for a non-display column.
    fn color_matrix_nibble(&self, column: u8) -> Result<u8, VicFetchError>;
    /// # Errors
    ///
    /// Returns a fetch range error for a non-display column.
    fn graphics_byte(&self, column: u8) -> Result<u8, VicFetchError>;
    /// # Errors
    ///
    /// Returns a fetch range error for a non-existent sprite.
    fn sprite_data_word(&self, sprite_index: u8) -> Result<u32, VicFetchError>;
}

impl VicPixelDataSource for VicFetchPipeline {
    fn sprite_display_mask(&self) -> u8 {
        self.sprite_display_mask()
    }

    fn screen_matrix_byte(&self, column: u8) -> Result<u8, VicFetchError> {
        self.screen_matrix_byte(column)
    }

    fn color_matrix_nibble(&self, column: u8) -> Result<u8, VicFetchError> {
        self.color_matrix_nibble(column)
    }

    fn graphics_byte(&self, column: u8) -> Result<u8, VicFetchError> {
        self.graphics_byte(column)
    }

    fn sprite_data_word(&self, sprite_index: u8) -> Result<u32, VicFetchError> {
        self.sprite_data_word(sprite_index)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicPixelModes(u8);

impl VicPixelModes {
    /// 三位模式号按 ECM/BMM/MCM 编码；5..=7 是 VIC-II 的非法黑色模式。
    pub const fn from_display_mode(display_mode: u8, screen_visible: bool) -> Self {
        let mut flags = 0;
        if display_mode & 0x02 != 0 {
            flags |= MODE_BITMAP;
        }
        if display_mode <= 4 {
            flags |= MODE_VALID;
        }
        if display_mode & 0x04 != 0 {
            flags |= MODE_EXTENDED_BACKGROUND;
        }
        if display_mode & 0x01 != 0 {
            flags |= MODE_MULTICOLOR;
        }
        if screen_visible {
            flags |= MODE_SCREEN_VISIBLE;
        }
        Self(flags)
    }

    pub const fn bitmap(self) -> bool {
        self.0 & MODE_BITMAP != 0
    }

    pub const fn valid(self) -> bool {
        self.0 & MODE_VALID != 0
    }

    pub const fn extended_background(self) -> bool {
        self.0 & MODE_EXTENDED_BACKGROUND != 0
    }

    pub const fn multicolor(self) -> bool {
        self.0 & MODE_MULTICOLOR != 0
    }

    pub const fn screen_visible(self) -> bool {
        self.0 & MODE_SCREEN_VISIBLE != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicPixelRegisters {
    pub background_colors: [u32; 4],
    pub border_color: u32,
    pub horizontal_scroll: u8,
    pub modes: VicPixelModes,
    pub palette: [u32; 16],
    pub sprite_multicolor_0: u32,
    pub sprite_multicolor_1: u32,
    pub sprites: [VicSprite; VIC_SPRITE_COUNT],
}

impl Default for VicPixelRegisters {
    fn default() -> Self {
        Self {
            background_colors: [C64_PALETTE[0]; 4],
            border_color: C64_PALETTE[0],
            horizontal_scroll: 0,
            modes: VicPixelModes::from_display_mode(0, false),
            palette: C64_PALETTE,
            sprite_multicolor_0: C64_PALETTE[4],
            sprite_multicolor_1: C64_PALETTE[0],
            sprites: core::array::from_fn(|index| VicSprite::new(DEFAULT_SPRITE_COLORS[index])),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VicPixelCollisions {
    pub sprite_foreground_mask: u8,
    pub sprite_sprite_mask: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VicPixelError {
    Fetch(VicFetchError),
    RasterCycleOutOfRange { cycle: u8 },
    TargetOffsetOverflow { offset: usize },
    TargetTooSmall { available: usize, required: usize },
}

impl fmt::Display for VicPixelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(error) => error.fmt(formatter),
            Self::RasterCycleOutOfRange { cycle } => {
                write!(
                    formatter,
                    "VIC-II pixel cycle {cycle} is outside the selected timing"
                )
            }
            Self::TargetOffsetOverflow { offset } => write!(
                formatter,
                "VIC-II raster target offset {offset} overflows the host address space"
            ),
            Self::TargetTooSmall {
                available,
                required,
            } => write!(
                formatter,
                "VIC-II raster requires {required} pixels but target has {available}"
            ),
        }
    }
}

impl std::error::Error for VicPixelError {}

impl From<VicFetchError> for VicPixelError {
    fn from(error: VicFetchError) -> Self {
        Self::Fetch(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GraphicsPixel {
    color: u32,
    foreground: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SpritePixel {
    behind_foreground: bool,
    color: u32,
    mask: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VicPixelPipeline {
    line_pixels: [u32; VIC_RASTER_OUTPUT_WIDTH],
    timing: VicTiming,
    border_color_output_delay: Option<u32>,
}

impl Default for VicPixelPipeline {
    fn default() -> Self {
        Self::new(PAL_VIC_TIMING)
    }
}

impl VicPixelPipeline {
    pub const fn new(timing: VicTiming) -> Self {
        Self {
            line_pixels: [C64_PALETTE[0]; VIC_RASTER_OUTPUT_WIDTH],
            timing,
            border_color_output_delay: None,
        }
    }

    pub fn reset(&mut self, border_color: u32) {
        self.line_pixels.fill(border_color);
        self.border_color_output_delay = Some(border_color);
    }

    /// # Errors
    ///
    /// Returns an explicit range or fetch invariant error; no guest-controlled value indexes the
    /// line, matrix, palette, or sprite buffers unchecked.
    pub fn clock_cycle<S: VicPixelDataSource>(
        &mut self,
        cycle: &VicCycleResult,
        border_pixel_mask: u8,
        registers: &VicPixelRegisters,
        fetch: &S,
    ) -> Result<VicPixelCollisions, VicPixelError> {
        if cycle.cycle == 0 || cycle.cycle > self.timing.cycles_per_raster_line {
            return Err(VicPixelError::RasterCycleOutOfRange { cycle: cycle.cycle });
        }
        if cycle.line_started() {
            self.line_pixels.fill(registers.border_color);
        }

        let physical_cycle_x = u16::from(cycle.cycle - 1) * PIXELS_PER_VIC_CYCLE;
        let sprite_display_mask = fetch.sprite_display_mask();
        let border_color = registers.border_color;
        let delayed_border_color = self.border_color_output_delay.unwrap_or(border_color);
        if self.border_color_output_delay != Some(border_color) {
            self.border_color_output_delay = Some(border_color);
        }

        if border_pixel_mask == u8::MAX && sprite_display_mask == 0 {
            self.fill_border_cycle(physical_cycle_x, border_color, delayed_border_color);
            return Ok(VicPixelCollisions::default());
        }

        let mut collisions = VicPixelCollisions::default();
        for pixel_in_cycle in 0..PIXELS_PER_VIC_CYCLE {
            let physical_x = physical_cycle_x + pixel_in_cycle;
            let vic_x = physical_x.wrapping_sub(VIC_X_ZERO_PHYSICAL_PIXEL) & VIC_X_COUNTER_MASK;
            let graphics = Self::resolve_graphics_pixel(vic_x, registers, fetch)?;
            let mut output_color = graphics.color;

            if sprite_display_mask != 0 {
                let sprites =
                    Self::resolve_sprite_pixel(vic_x, sprite_display_mask, registers, fetch)?;
                if sprites.mask & sprites.mask.wrapping_sub(1) != 0 {
                    collisions.sprite_sprite_mask |= sprites.mask;
                }
                if graphics.foreground && sprites.mask != 0 {
                    collisions.sprite_foreground_mask |= sprites.mask;
                }
                if sprites.color != TRANSPARENT_PIXEL
                    && !(graphics.foreground && sprites.behind_foreground)
                {
                    output_color = sprites.color;
                }
            }

            if border_pixel_mask & (0x80 >> u32::from(pixel_in_cycle)) != 0 {
                output_color = if pixel_in_cycle == 0 {
                    delayed_border_color
                } else {
                    border_color
                };
            }
            if physical_x >= visible_crop_physical_pixel() {
                let output_index = usize::from(physical_x - visible_crop_physical_pixel());
                if output_index < VIC_RASTER_OUTPUT_WIDTH {
                    self.line_pixels[output_index] = output_color;
                }
            }
        }
        Ok(collisions)
    }

    pub const fn pixels(&self) -> &[u32; VIC_RASTER_OUTPUT_WIDTH] {
        &self.line_pixels
    }

    pub const fn snapshot(&self) -> [u32; VIC_RASTER_OUTPUT_WIDTH] {
        self.line_pixels
    }

    /// # Errors
    ///
    /// Returns an overflow or capacity error if the full line cannot fit at `target_offset`.
    pub fn copy_pixels_to(
        &self,
        target: &mut [u32],
        target_offset: usize,
    ) -> Result<(), VicPixelError> {
        let required = target_offset.checked_add(VIC_RASTER_OUTPUT_WIDTH).ok_or(
            VicPixelError::TargetOffsetOverflow {
                offset: target_offset,
            },
        )?;
        if required > target.len() {
            return Err(VicPixelError::TargetTooSmall {
                available: target.len(),
                required,
            });
        }
        target[target_offset..required].copy_from_slice(&self.line_pixels);
        Ok(())
    }

    fn fill_border_cycle(
        &mut self,
        physical_cycle_x: u16,
        border_color: u32,
        delayed_border_color: u32,
    ) {
        let physical_end = physical_cycle_x + PIXELS_PER_VIC_CYCLE;
        let visible_start = visible_crop_physical_pixel();
        let visible_end = visible_start + VIC_RASTER_OUTPUT_WIDTH_U16;
        let clipped_start = physical_cycle_x.max(visible_start);
        let clipped_end = physical_end.min(visible_end);
        if clipped_start < clipped_end {
            let first_output = usize::from(clipped_start - visible_start);
            let last_output = usize::from(clipped_end - visible_start);
            self.line_pixels[first_output..last_output].fill(border_color);
        }
        if physical_cycle_x >= visible_start && physical_cycle_x < visible_end {
            let output_index = usize::from(physical_cycle_x - visible_start);
            self.line_pixels[output_index] = delayed_border_color;
        }
    }

    fn resolve_graphics_pixel<S: VicPixelDataSource>(
        vic_x: u16,
        registers: &VicPixelRegisters,
        fetch: &S,
    ) -> Result<GraphicsPixel, VicPixelError> {
        let background_0 = registers.background_colors[0];
        if !registers.modes.screen_visible() {
            return Ok(GraphicsPixel {
                color: background_0,
                foreground: false,
            });
        }
        if !registers.modes.valid() {
            return Ok(GraphicsPixel {
                color: registers.palette[0],
                foreground: true,
            });
        }

        let display_x =
            i32::from(vic_x) - TEXT_DISPLAY_VIC_X - i32::from(registers.horizontal_scroll);
        if !(0..TEXT_DISPLAY_WIDTH).contains(&display_x) {
            return Ok(GraphicsPixel {
                color: background_0,
                foreground: false,
            });
        }
        let column = u8::try_from(display_x / TEXT_COLUMN_WIDTH)
            .map_err(|_| VicFetchError::ColumnOutOfRange { column: u8::MAX })?;
        let pixel = u8::try_from(display_x % TEXT_COLUMN_WIDTH)
            .map_err(|_| VicFetchError::ColumnOutOfRange { column: u8::MAX })?;
        let screen_code = fetch.screen_matrix_byte(column)?;
        let color_ram = fetch.color_matrix_nibble(column)?;
        let graphics = fetch.graphics_byte(column)?;

        let result = if registers.modes.bitmap() {
            if registers.modes.multicolor() {
                Self::resolve_multicolor_bitmap_pixel(
                    pixel,
                    graphics,
                    screen_code,
                    color_ram,
                    registers,
                )
            } else {
                Self::resolve_high_resolution_bitmap_pixel(pixel, graphics, screen_code, registers)
            }
        } else if registers.modes.multicolor() && color_ram >= 8 {
            Self::resolve_multicolor_text_pixel(pixel, graphics, color_ram, registers)
        } else {
            Self::resolve_high_resolution_text_pixel(
                pixel,
                graphics,
                screen_code,
                color_ram,
                registers,
            )
        };
        Ok(result)
    }

    fn resolve_high_resolution_text_pixel(
        pixel: u8,
        graphics: u8,
        screen_code: u8,
        color_ram: u8,
        registers: &VicPixelRegisters,
    ) -> GraphicsPixel {
        let foreground = graphics & (0x80 >> pixel) != 0;
        if foreground {
            return GraphicsPixel {
                color: registers.palette[usize::from(color_ram)],
                foreground: true,
            };
        }
        let background_index = if registers.modes.extended_background() {
            screen_code >> 6
        } else {
            0
        };
        GraphicsPixel {
            color: registers.background_colors[usize::from(background_index)],
            foreground: false,
        }
    }

    fn resolve_multicolor_text_pixel(
        pixel: u8,
        graphics: u8,
        color_ram: u8,
        registers: &VicPixelRegisters,
    ) -> GraphicsPixel {
        let pair = pixel / 2;
        let color_index = (graphics >> (6 - pair * 2)) & 0x03;
        let color = match color_index {
            0 => registers.background_colors[0],
            1 => registers.background_colors[1],
            2 => registers.background_colors[2],
            _ => registers.palette[usize::from(color_ram & 0x07)],
        };
        GraphicsPixel {
            color,
            foreground: color_index >= 2,
        }
    }

    fn resolve_high_resolution_bitmap_pixel(
        pixel: u8,
        graphics: u8,
        screen_code: u8,
        registers: &VicPixelRegisters,
    ) -> GraphicsPixel {
        let foreground = graphics & (0x80 >> pixel) != 0;
        let palette_index = if foreground {
            screen_code >> 4
        } else {
            screen_code & 0x0f
        };
        GraphicsPixel {
            color: registers.palette[usize::from(palette_index)],
            foreground,
        }
    }

    fn resolve_multicolor_bitmap_pixel(
        pixel: u8,
        graphics: u8,
        screen_code: u8,
        color_ram: u8,
        registers: &VicPixelRegisters,
    ) -> GraphicsPixel {
        let pair = pixel / 2;
        let color_index = (graphics >> (6 - pair * 2)) & 0x03;
        let color = match color_index {
            0 => registers.background_colors[0],
            1 => registers.palette[usize::from(screen_code >> 4)],
            2 => registers.palette[usize::from(screen_code & 0x0f)],
            _ => registers.palette[usize::from(color_ram)],
        };
        GraphicsPixel {
            color,
            foreground: color_index >= 2,
        }
    }

    fn resolve_sprite_pixel<S: VicPixelDataSource>(
        vic_x: u16,
        display_mask: u8,
        registers: &VicPixelRegisters,
        fetch: &S,
    ) -> Result<SpritePixel, VicPixelError> {
        let mut mask = 0;
        let mut selected_color = TRANSPARENT_PIXEL;
        let mut selected_behind_foreground = false;
        for (index, sprite) in registers.sprites.iter().copied().enumerate() {
            let sprite_bit = 1_u8 << index;
            if display_mask & sprite_bit == 0 {
                continue;
            }
            let Some(source_pixel) = sprite_source_pixel(vic_x, sprite) else {
                continue;
            };
            let sprite_index =
                u8::try_from(index).map_err(|_| VicFetchError::SpriteIndexOutOfRange {
                    sprite_index: u8::MAX,
                })?;
            let color = sprite_pixel_color(
                fetch.sprite_data_word(sprite_index)?,
                source_pixel,
                sprite,
                registers,
            );
            if color == TRANSPARENT_PIXEL {
                continue;
            }
            mask |= sprite_bit;
            if selected_color == TRANSPARENT_PIXEL {
                selected_color = color;
                selected_behind_foreground = !sprite.foreground();
            }
        }
        Ok(SpritePixel {
            behind_foreground: selected_behind_foreground,
            color: selected_color,
            mask,
        })
    }
}

const fn visible_crop_physical_pixel() -> u16 {
    (DEFAULT_FIRST_VISIBLE_CYCLE - 1) * PIXELS_PER_VIC_CYCLE
}

fn sprite_source_pixel(vic_x: u16, sprite: VicSprite) -> Option<u8> {
    let scale = if sprite.expand_horizontal() { 2 } else { 1 };
    let relative_pixel = (i32::from(vic_x) - i32::from(sprite.x) + VIC_X_COUNTER_MODULUS)
        & i32::from(VIC_X_COUNTER_MASK);
    if relative_pixel >= i32::from(SPRITE_SOURCE_WIDTH * scale) {
        return None;
    }
    u8::try_from(relative_pixel / i32::from(scale)).ok()
}

fn sprite_pixel_color(
    data: u32,
    source_pixel: u8,
    sprite: VicSprite,
    registers: &VicPixelRegisters,
) -> u32 {
    if !sprite.multicolor() {
        return if data & (1 << (SPRITE_SOURCE_WIDTH - 1 - u16::from(source_pixel))) != 0 {
            sprite.color
        } else {
            TRANSPARENT_PIXEL
        };
    }
    let pair = source_pixel / 2;
    let color_index = (data >> (SPRITE_SOURCE_WIDTH - 2 - u16::from(pair) * 2)) & 0x03;
    match color_index {
        1 => registers.sprite_multicolor_0,
        2 => sprite.color,
        3 => registers.sprite_multicolor_1,
        _ => TRANSPARENT_PIXEL,
    }
}

const fn pack_rgba_pixel(red: u8, green: u8, blue: u8) -> u32 {
    u32::from_ne_bytes([red, green, blue, u8::MAX])
}

#[cfg(test)]
mod tests {
    use super::{
        C64_PALETTE, VIC_RASTER_OUTPUT_WIDTH, VicPixelDataSource, VicPixelPipeline,
        VicPixelRegisters,
    };
    use crate::devices::vic::{VicCycleSequencer, VicCycleSignals, VicFetchError, VicPixelModes};

    struct SolidSource;

    impl VicPixelDataSource for SolidSource {
        fn sprite_display_mask(&self) -> u8 {
            0
        }

        fn screen_matrix_byte(&self, _column: u8) -> Result<u8, VicFetchError> {
            Ok(0)
        }

        fn color_matrix_nibble(&self, _column: u8) -> Result<u8, VicFetchError> {
            Ok(1)
        }

        fn graphics_byte(&self, _column: u8) -> Result<u8, VicFetchError> {
            Ok(u8::MAX)
        }

        fn sprite_data_word(&self, _sprite_index: u8) -> Result<u32, VicFetchError> {
            Ok(0)
        }
    }

    #[test]
    fn visible_text_pixels_use_fixed_line_storage() {
        let mut sequencer = VicCycleSequencer::new();
        let mut pipeline = VicPixelPipeline::default();
        let mut registers = VicPixelRegisters {
            modes: VicPixelModes::from_display_mode(0, true),
            ..VicPixelRegisters::default()
        };
        pipeline.reset(C64_PALETTE[0]);
        for _ in 0..63 {
            let cycle = sequencer.tick(&VicCycleSignals::default());
            pipeline
                .clock_cycle(&cycle, 0, &registers, &SolidSource)
                .unwrap();
        }
        assert_eq!(pipeline.pixels().len(), VIC_RASTER_OUTPUT_WIDTH);
        assert!(pipeline.pixels().contains(&C64_PALETTE[1]));
        registers.border_color = C64_PALETTE[2];
        pipeline.reset(registers.border_color);
        assert!(
            pipeline
                .pixels()
                .iter()
                .all(|&pixel| pixel == C64_PALETTE[2])
        );
    }
}
