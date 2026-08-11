// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II Rust 取数差分适配器
//
//   文件:       vic_fetch_trace.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::vic::{
    VicCycleSequencer, VicCycleSignals, VicFetchPipeline, VicFetchRegisters, VicFetchSnapshot,
    VicMemoryBus,
};
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
    Tick {
        bitmap_memory_address: u16,
        bitmap_mode: bool,
        character_memory_address: u16,
        cpu_data_bus_value: u8,
        display_enabled: bool,
        extended_background_mode: bool,
        screen_memory_address: u16,
        sprite_enable_mask: u8,
        sprite_vertical_expansion_mask: u8,
        sprite_y: [u8; 8],
        vertical_scroll: u8,
    },
    WriteExpansion {
        value: u8,
    },
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    byte_reads: u32,
    color_reads: u32,
    phi1_data_bus_value: u8,
    sprite_display_mask: u8,
    state_hash: u32,
}

#[derive(Default)]
struct PatternMemory {
    cpu_data_bus_value: u8,
    byte_reads: u32,
    color_reads: u32,
}

impl PatternMemory {
    fn reset(&mut self) {
        *self = Self::default();
    }
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
            0x6569_0001,
        )
    }

    fn read_vic_color(&mut self, index: u16) -> u8 {
        self.color_reads = self.color_reads.wrapping_add(1);
        mix_byte(u32::from(index & 0x03ff), self.color_reads, 0x6569_0002) & 0x0f
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

fn state_hash(snapshot: VicFetchSnapshot) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
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

fn observe(fetch: &VicFetchPipeline, memory: &PatternMemory) -> Observation {
    Observation {
        byte_reads: memory.byte_reads,
        color_reads: memory.color_reads,
        phi1_data_bus_value: fetch.phi1_data_bus_value(),
        sprite_display_mask: fetch.sprite_display_mask(),
        state_hash: state_hash(fetch.snapshot()),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let operations: Vec<Operation> = serde_json::from_str(&input)?;
    let mut sequencer = VicCycleSequencer::new();
    let mut fetch = VicFetchPipeline::default();
    let mut memory = PatternMemory::default();
    let mut observations = Vec::with_capacity(operations.len());

    for operation in operations {
        match operation {
            Operation::Tick {
                bitmap_memory_address,
                bitmap_mode,
                character_memory_address,
                cpu_data_bus_value,
                display_enabled,
                extended_background_mode,
                screen_memory_address,
                sprite_enable_mask,
                sprite_vertical_expansion_mask,
                sprite_y,
                vertical_scroll,
            } => {
                memory.cpu_data_bus_value = cpu_data_bus_value;
                let cycle = sequencer.tick(&VicCycleSignals {
                    display_enabled,
                    sprite_enable_mask,
                    sprite_vertical_expansion_mask,
                    vertical_scroll,
                    sprite_y,
                });
                fetch.execute_cycle(
                    &cycle,
                    VicFetchRegisters::new(
                        bitmap_memory_address,
                        bitmap_mode,
                        character_memory_address,
                        extended_background_mode,
                        screen_memory_address,
                    ),
                    &mut memory,
                )?;
            }
            Operation::WriteExpansion { value } => {
                sequencer.write_sprite_vertical_expansion_register(value);
            }
            Operation::Reset => {
                sequencer.reset();
                fetch.reset();
                memory.reset();
            }
        }
        observations.push(observe(&fetch, &memory));
    }
    serde_json::to_writer(io::stdout().lock(), &observations)?;
    Ok(())
}
