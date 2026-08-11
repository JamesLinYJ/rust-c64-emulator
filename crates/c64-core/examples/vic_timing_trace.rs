// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic VIC timing differential adapter
//
//   File:       vic_timing_trace.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::vic::{
    VicBorderController, VicBorderSignals, VicCycleResult, VicCycleSequencer, VicCycleSignals,
    VicMatrixAccessSource, VicPhi1Fetch, VicPhi2Fetch,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Tick {
        column_select: bool,
        display_enabled: bool,
        row_select: bool,
        sprite_enable_mask: u8,
        sprite_vertical_expansion_mask: u8,
        vertical_scroll: u8,
        sprite_y: [u8; 8],
    },
    WriteExpansion {
        value: u8,
    },
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    border_pixel_mask: Option<u8>,
    result_flags: Option<u16>,
    cycle: u8,
    raster_line: u16,
    completed_raster_line: Option<u16>,
    late_reload_column: Option<u8>,
    matrix_access_code: Option<u16>,
    phi1_code: Option<u16>,
    phi2_code: Option<u16>,
    sprite_phi1_offset: Option<u8>,
    sprite_phi2_offset: Option<u8>,
    sprite_display_mask: u8,
    sprite_dma_mask: u8,
    current_flags: u8,
}

fn encode_result(result: VicCycleResult, border_pixel_mask: u8) -> Observation {
    let result_flags = u16::from(result.aec_low())
        | (u16::from(result.ba_low()) << 1)
        | (u16::from(result.bad_line()) << 2)
        | (u16::from(result.bad_line_condition()) << 3)
        | (u16::from(result.enter_display_state()) << 4)
        | (u16::from(result.frame_started()) << 5)
        | (u16::from(result.line_started()) << 6)
        | (u16::from(result.reset_row_counter()) << 7);
    Observation {
        border_pixel_mask: Some(border_pixel_mask),
        result_flags: Some(result_flags),
        cycle: result.cycle,
        raster_line: result.raster_line,
        completed_raster_line: result.completed_raster_line,
        late_reload_column: result.late_video_counter_reload_column,
        matrix_access_code: result.matrix_access.map(|access| {
            u16::from(access.column)
                | (match access.source {
                    VicMatrixAccessSource::CpuDataBus => 0,
                    VicMatrixAccessSource::VideoMemory => 1,
                } << 8)
        }),
        phi1_code: Some(encode_phi1(result.bus_schedule.phi1)),
        phi2_code: result.bus_schedule.phi2.map(encode_phi2),
        sprite_phi1_offset: result.sprite_data_offsets[0],
        sprite_phi2_offset: result.sprite_data_offsets[1],
        sprite_display_mask: result.sprite_display_mask,
        sprite_dma_mask: result.sprite_dma_mask,
        current_flags: u8::from(result.aec_low())
            | (u8::from(result.ba_low()) << 1)
            | (u8::from(result.bad_line()) << 2),
    }
}

fn observe_without_tick(sequencer: &VicCycleSequencer) -> Observation {
    Observation {
        border_pixel_mask: None,
        result_flags: None,
        cycle: sequencer.cycle(),
        raster_line: sequencer.raster_line(),
        completed_raster_line: None,
        late_reload_column: None,
        matrix_access_code: None,
        phi1_code: None,
        phi2_code: None,
        sprite_phi1_offset: None,
        sprite_phi2_offset: None,
        sprite_display_mask: sequencer.sprite_display_mask(),
        sprite_dma_mask: sequencer.sprite_dma_mask(),
        current_flags: u8::from(sequencer.aec_low())
            | (u8::from(sequencer.ba_low()) << 1)
            | (u8::from(sequencer.bad_line()) << 2),
    }
}

const fn encode_phi1(fetch: VicPhi1Fetch) -> u16 {
    match fetch {
        VicPhi1Fetch::Graphics => 0,
        VicPhi1Fetch::Idle => 1,
        VicPhi1Fetch::Refresh => 2,
        VicPhi1Fetch::SpriteData {
            sprite_index,
            byte_index,
        } => 3 | (sprite_index as u16) << 8 | (byte_index as u16) << 12,
        VicPhi1Fetch::SpritePointer { sprite_index } => 4 | (sprite_index as u16) << 8,
    }
}

const fn encode_phi2(fetch: VicPhi2Fetch) -> u16 {
    match fetch {
        VicPhi2Fetch::Matrix => 0,
        VicPhi2Fetch::SpriteData {
            sprite_index,
            byte_index,
        } => 1 | (sprite_index as u16) << 8 | (byte_index as u16) << 12,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let operations: Vec<Operation> = serde_json::from_str(&input)?;
    let mut sequencer = VicCycleSequencer::new();
    let mut border = VicBorderController::default();
    let mut observations = Vec::with_capacity(operations.len());
    for operation in operations {
        match operation {
            Operation::Tick {
                column_select,
                display_enabled,
                row_select,
                sprite_enable_mask,
                sprite_vertical_expansion_mask,
                vertical_scroll,
                sprite_y,
            } => {
                let result = sequencer.tick(&VicCycleSignals {
                    display_enabled,
                    sprite_enable_mask,
                    sprite_vertical_expansion_mask,
                    vertical_scroll,
                    sprite_y,
                });
                let border_pixel_mask = border.tick(VicBorderSignals {
                    column_select,
                    display_enabled,
                    raster_cycle: result.cycle,
                    raster_line: result.raster_line,
                    row_select,
                });
                observations.push(encode_result(result, border_pixel_mask));
            }
            Operation::WriteExpansion { value } => {
                sequencer.write_sprite_vertical_expansion_register(value);
                observations.push(observe_without_tick(&sequencer));
            }
            Operation::Reset => {
                sequencer.reset();
                border.reset();
                observations.push(observe_without_tick(&sequencer));
            }
        }
    }
    serde_json::to_writer(io::stdout().lock(), &observations)?;
    Ok(())
}
