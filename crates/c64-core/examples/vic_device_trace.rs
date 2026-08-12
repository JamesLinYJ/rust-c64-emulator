// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II Rust 整芯片差分适配器
//
//   文件:       vic_device_trace.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::vic::{VicFetchSnapshot, VicII, VicMemoryBus, VicRasterLineSnapshot};
use serde::{Deserialize, Serialize};

const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Tick { cpu_data_bus_value: u8 },
    Read { address: u16 },
    Write { address: u16, value: u8 },
    SetLightPen { high: bool },
    LatchLightPen { x: u16, y: u16 },
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    border_hash: u32,
    current_flags: u8,
    byte_reads: u32,
    color_reads: u32,
    cycle: u8,
    display_state: u16,
    fetch_hash: u32,
    interrupt_pending: bool,
    phi1_data_bus_value: u8,
    pixel_hash: u32,
    raster_line: u16,
    read_value: Option<u8>,
    sprite_dma_mask: u8,
    sprite_data_words: [u32; 8],
    sprite_display_mask: u8,
    sprite_flags: [u8; 8],
    sprite_hash: u32,
    state_hash: u32,
}

#[derive(Default)]
struct PatternMemory {
    cpu_data_bus_value: u8,
    byte_reads: u32,
    color_reads: u32,
}

impl VicMemoryBus for PatternMemory {
    fn cpu_data_bus_value(&self) -> u8 {
        self.cpu_data_bus_value
    }

    fn read_vic_byte(&mut self, address_in_bank: u16) -> u8 {
        self.byte_reads = self.byte_reads.wrapping_add(1);
        mix_byte(
            u32::from(address_in_bank & 0x3fff),
            self.byte_reads,
            0x6569_1001,
        )
    }

    fn read_vic_color(&mut self, index: u16) -> u8 {
        self.color_reads = self.color_reads.wrapping_add(1);
        mix_byte(u32::from(index & 0x03ff), self.color_reads, 0x6569_1002) & 0x0f
    }
}

const fn mix_byte(address: u32, counter: u32, salt: u32) -> u8 {
    let mut value = address
        .wrapping_mul(0x045d_9f3b)
        .wrapping_add(counter.wrapping_mul(0x119d_e1f3))
        .wrapping_add(salt);
    value ^= value >> 16;
    value = value.wrapping_mul(0x27d4_eb2d);
    (value ^ (value >> 15)).to_le_bytes()[0]
}

const fn hash_byte(hash: u32, value: u8) -> u32 {
    (hash ^ value as u32).wrapping_mul(FNV_PRIME)
}

const fn hash_u16(hash: u32, value: u16) -> u32 {
    let bytes = value.to_le_bytes();
    hash_byte(hash_byte(hash, bytes[0]), bytes[1])
}

const fn hash_u32(hash: u32, value: u32) -> u32 {
    let bytes = value.to_le_bytes();
    hash_byte(
        hash_byte(hash_byte(hash_byte(hash, bytes[0]), bytes[1]), bytes[2]),
        bytes[3],
    )
}

fn hash_bytes(mut hash: u32, values: &[u8]) -> u32 {
    for &value in values {
        hash = hash_byte(hash, value);
    }
    hash
}

fn hash_words(mut hash: u32, values: &[u32]) -> u32 {
    for &value in values {
        hash = hash_u32(hash, value);
    }
    hash
}

fn hash_fetch(mut hash: u32, snapshot: VicFetchSnapshot) -> u32 {
    hash = hash_bytes(hash, &snapshot.color_matrix);
    hash = hash_bytes(hash, &snapshot.graphics);
    hash = hash_byte(hash, u8::from(snapshot.idle_state));
    hash = hash_byte(hash, snapshot.last_phi1_byte);
    hash = hash_byte(hash, snapshot.last_phi2_byte);
    hash = hash_words(hash, &snapshot.line_sprite_data);
    hash = hash_byte(hash, snapshot.line_sprite_display_mask);
    hash = hash_bytes(hash, &snapshot.line_sprite_pointers);
    hash = hash_byte(hash, snapshot.matrix_index);
    hash = hash_words(hash, &snapshot.sprite_data);
    hash = hash_bytes(hash, &snapshot.sprite_pointers);
    hash = hash_byte(hash, snapshot.refresh_counter);
    hash = hash_byte(hash, snapshot.row_counter);
    hash = hash_bytes(hash, &snapshot.screen_matrix);
    hash = hash_u16(hash, snapshot.video_counter);
    hash_u16(hash, snapshot.video_counter_base)
}

fn state_hash(vic: &VicII, snapshot: &VicRasterLineSnapshot) -> u32 {
    let mut hash = hash_words(FNV_OFFSET_BASIS, &snapshot.border_colors);
    hash = hash_bytes(hash, &snapshot.border_pixel_masks);
    hash = hash_fetch(hash, snapshot.fetch);
    hash = hash_words(hash, &snapshot.pixels);
    for sprite in vic.sprites().iter().copied() {
        hash = hash_u16(hash, sprite.x);
        hash = hash_byte(hash, sprite.y);
        hash = hash_u32(hash, sprite.color);
        hash = hash_byte(hash, u8::from(sprite.enabled()));
        hash = hash_byte(hash, u8::from(sprite.foreground()));
        hash = hash_byte(hash, u8::from(sprite.multicolor()));
        hash = hash_byte(hash, u8::from(sprite.expand_vertical()));
        hash = hash_byte(hash, u8::from(sprite.expand_horizontal()));
        hash = hash_byte(hash, u8::from(sprite.collision_with_sprite()));
        hash = hash_byte(hash, u8::from(sprite.collision_with_foreground()));
    }
    hash
}

fn sprite_hash(vic: &VicII) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    for sprite in vic.sprites().iter().copied() {
        hash = hash_u16(hash, sprite.x);
        hash = hash_byte(hash, sprite.y);
        hash = hash_u32(hash, sprite.color);
        hash = hash_byte(hash, u8::from(sprite.enabled()));
        hash = hash_byte(hash, u8::from(sprite.foreground()));
        hash = hash_byte(hash, u8::from(sprite.multicolor()));
        hash = hash_byte(hash, u8::from(sprite.expand_vertical()));
        hash = hash_byte(hash, u8::from(sprite.expand_horizontal()));
        hash = hash_byte(hash, u8::from(sprite.collision_with_sprite()));
        hash = hash_byte(hash, u8::from(sprite.collision_with_foreground()));
    }
    hash
}

fn observe(vic: &VicII, memory: &PatternMemory, read_value: Option<u8>) -> Observation {
    let snapshot = vic.capture_raster_line_state();
    let registers = vic.registers();
    let pixel_modes = vic.pixel_modes();
    let display_mode = u8::from(pixel_modes.multicolor())
        | (u8::from(pixel_modes.bitmap()) << 1)
        | (u8::from(pixel_modes.extended_background()) << 2);
    let display_state = u16::from(pixel_modes.screen_visible())
        | (u16::from(display_mode) << 1)
        | (u16::from(registers[0x16] & 0x07) << 4)
        | (u16::from(vic.standard_column_mode()) << 7)
        | (u16::from(vic.standard_row_mode()) << 8);
    Observation {
        border_hash: hash_bytes(
            hash_words(FNV_OFFSET_BASIS, &snapshot.border_colors),
            &snapshot.border_pixel_masks,
        ),
        current_flags: u8::from(vic.aec_low())
            | (u8::from(vic.ba_low()) << 1)
            | (u8::from(vic.bad_line()) << 2),
        byte_reads: memory.byte_reads,
        color_reads: memory.color_reads,
        cycle: vic.current_raster_cycle(),
        display_state,
        fetch_hash: hash_fetch(FNV_OFFSET_BASIS, snapshot.fetch),
        interrupt_pending: vic.interrupt_pending(),
        phi1_data_bus_value: vic.phi1_data_bus_value(),
        pixel_hash: hash_words(FNV_OFFSET_BASIS, &snapshot.pixels),
        raster_line: vic.current_raster_line(),
        read_value,
        sprite_dma_mask: vic.sprite_dma_mask(),
        sprite_data_words: snapshot.fetch.line_sprite_data,
        sprite_display_mask: snapshot.fetch.line_sprite_display_mask,
        sprite_flags: vic.sprites().map(|sprite| {
            u8::from(sprite.enabled())
                | (u8::from(sprite.foreground()) << 1)
                | (u8::from(sprite.multicolor()) << 2)
                | (u8::from(sprite.expand_vertical()) << 3)
                | (u8::from(sprite.expand_horizontal()) << 4)
                | (u8::from(sprite.collision_with_sprite()) << 5)
                | (u8::from(sprite.collision_with_foreground()) << 6)
        }),
        sprite_hash: sprite_hash(vic),
        state_hash: state_hash(vic, &snapshot),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let operations: Vec<Operation> = serde_json::from_str(&input)?;
    let mut vic = VicII::new();
    let mut memory = PatternMemory::default();
    let mut observations = Vec::with_capacity(operations.len());
    for operation in operations {
        let read_value = match operation {
            Operation::Tick { cpu_data_bus_value } => {
                memory.cpu_data_bus_value = cpu_data_bus_value;
                vic.clock_cycle(&mut memory)?;
                None
            }
            Operation::Read { address } => Some(vic.read_register(address)),
            Operation::Write { address, value } => {
                vic.write_register(address, value);
                None
            }
            Operation::SetLightPen { high } => {
                vic.set_light_pen_input_high(high);
                None
            }
            Operation::LatchLightPen { x, y } => {
                vic.latch_light_pen(x, y);
                None
            }
            Operation::Reset => {
                vic.reset();
                None
            }
        };
        observations.push(observe(&vic, &memory, read_value));
    }
    serde_json::to_writer(io::stdout().lock(), &observations)?;
    Ok(())
}
