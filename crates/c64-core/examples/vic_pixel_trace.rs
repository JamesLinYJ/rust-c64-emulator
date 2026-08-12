// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II Rust 像素差分适配器
//
//   文件:       vic_pixel_trace.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io;

use c64_core::devices::vic::{
    C64_PALETTE, VicCycleSequencer, VicCycleSignals, VicFetchError, VicPixelCollisions,
    VicPixelDataSource, VicPixelModes, VicPixelPipeline, VicPixelRegisters, VicSprite,
};
use serde::{Deserialize, Serialize};

#[path = "support/json_line.rs"]
mod json_line;

const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Tick {
        background_color_indices: [u8; 4],
        border_color_index: u8,
        border_pixel_mask: u8,
        display_mode: u8,
        horizontal_scroll: u8,
        screen_visible: bool,
        source_seed: u32,
        sprite_display_mask: u8,
        sprite_multicolor_0_index: u8,
        sprite_multicolor_1_index: u8,
        sprite_seed: u32,
    },
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    center_pixel: u32,
    first_pixel: u32,
    last_pixel: u32,
    line_hash: u32,
    sprite_foreground_mask: u8,
    sprite_sprite_mask: u8,
}

#[derive(Default)]
struct PatternPixelSource {
    source_seed: u32,
    sprite_display_mask: u8,
}

impl VicPixelDataSource for PatternPixelSource {
    fn sprite_display_mask(&self) -> u8 {
        self.sprite_display_mask
    }

    fn screen_matrix_byte(&self, column: u8) -> Result<u8, VicFetchError> {
        Ok(mix_word(self.source_seed, u32::from(column), 0x1001).to_le_bytes()[0])
    }

    fn color_matrix_nibble(&self, column: u8) -> Result<u8, VicFetchError> {
        Ok(mix_word(self.source_seed, u32::from(column), 0x1002).to_le_bytes()[0] & 0x0f)
    }

    fn graphics_byte(&self, column: u8) -> Result<u8, VicFetchError> {
        Ok(mix_word(self.source_seed, u32::from(column), 0x1003).to_le_bytes()[0])
    }

    fn sprite_data_word(&self, sprite_index: u8) -> Result<u32, VicFetchError> {
        Ok(mix_word(self.source_seed, u32::from(sprite_index), 0x1004) & 0x00ff_ffff)
    }
}

const fn mix_word(seed: u32, index: u32, salt: u32) -> u32 {
    let mut value = seed ^ index.wrapping_add(1).wrapping_mul(0x9e37_79b9) ^ salt;
    value = (value ^ (value >> 16)).wrapping_mul(0x7feb_352d);
    value = (value ^ (value >> 15)).wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

fn configure_sprites(sprites: &mut [VicSprite; 8], seed: u32) {
    for (index, sprite) in sprites.iter_mut().enumerate() {
        let value = mix_word(seed, u32::try_from(index).unwrap_or(u32::MAX), 0x5350_5254);
        sprite.x = u16::from_le_bytes([value.to_le_bytes()[0], value.to_le_bytes()[1]]) & 0x01ff;
        sprite.color = C64_PALETTE[((value >> 12) & 0x0f) as usize];
        sprite.set_foreground(value & (1 << 9) != 0);
        sprite.set_multicolor(value & (1 << 10) != 0);
        sprite.set_expand_horizontal(value & (1 << 11) != 0);
    }
}

fn hash_line(pixels: &[u32]) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    for &pixel in pixels {
        for byte in pixel.to_le_bytes() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

fn observe(pipeline: &VicPixelPipeline, collisions: VicPixelCollisions) -> Observation {
    let pixels = pipeline.pixels();
    Observation {
        center_pixel: pixels[201],
        first_pixel: pixels[0],
        last_pixel: pixels[402],
        line_hash: hash_line(pixels),
        sprite_foreground_mask: collisions.sprite_foreground_mask,
        sprite_sprite_mask: collisions.sprite_sprite_mask,
    }
}

fn palette_color(index: u8) -> u32 {
    C64_PALETTE[usize::from(index & 0x0f)]
}

fn main() -> Result<(), Box<dyn Error>> {
    let operations: Vec<Operation> = json_line::read_stdin_json_line()?;
    let mut sequencer = VicCycleSequencer::new();
    let mut pipeline = VicPixelPipeline::default();
    let mut source = PatternPixelSource::default();
    let mut registers = VicPixelRegisters::default();
    let mut observations = Vec::with_capacity(operations.len());
    pipeline.reset(C64_PALETTE[0]);

    for operation in operations {
        let collisions = match operation {
            Operation::Tick {
                background_color_indices,
                border_color_index,
                border_pixel_mask,
                display_mode,
                horizontal_scroll,
                screen_visible,
                source_seed,
                sprite_display_mask,
                sprite_multicolor_0_index,
                sprite_multicolor_1_index,
                sprite_seed,
            } => {
                registers.background_colors = background_color_indices.map(palette_color);
                registers.border_color = palette_color(border_color_index);
                registers.horizontal_scroll = horizontal_scroll;
                registers.modes = VicPixelModes::from_display_mode(display_mode, screen_visible);
                registers.sprite_multicolor_0 = palette_color(sprite_multicolor_0_index);
                registers.sprite_multicolor_1 = palette_color(sprite_multicolor_1_index);
                configure_sprites(&mut registers.sprites, sprite_seed);
                source.source_seed = source_seed;
                source.sprite_display_mask = sprite_display_mask;
                let cycle = sequencer.tick(&VicCycleSignals::default());
                pipeline.clock_cycle(&cycle, border_pixel_mask, &registers, &source)?
            }
            Operation::Reset => {
                sequencer.reset();
                pipeline.reset(C64_PALETTE[0]);
                VicPixelCollisions::default()
            }
        };
        observations.push(observe(&pipeline, collisions));
    }
    serde_json::to_writer(io::stdout().lock(), &observations)?;
    Ok(())
}
