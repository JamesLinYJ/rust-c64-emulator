// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VIC-II PAL timing constants
//
//   File:       vic/timing.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicBadLineTiming {
    pub ba_first_cycle: u8,
    pub ba_last_cycle: u8,
    pub first_raster_line: u16,
    pub last_raster_line: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicBorderTiming {
    pub reduced_column_left_cycle: u8,
    pub reduced_column_right_cycle: u8,
    pub reduced_row_start_line: u16,
    pub reduced_row_stop_line: u16,
    pub standard_column_left_cycle: u8,
    pub standard_column_right_cycle: u8,
    pub standard_row_start_line: u16,
    pub standard_row_stop_line: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicFetchTiming {
    pub graphics_first_cycle: u8,
    pub graphics_last_cycle: u8,
    pub idle_first_cycle: u8,
    pub idle_last_cycle: u8,
    pub matrix_first_cycle: u8,
    pub matrix_last_cycle: u8,
    pub row_counter_update_cycle: u8,
    pub refresh_first_cycle: u8,
    pub refresh_last_cycle: u8,
    pub video_counter_reload_cycle: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicLightPenTiming {
    pub horizontal_counter_granularity_pixels: u16,
    pub horizontal_origin_pixels: u16,
    pub horizontal_position_modulo_pixels: u16,
    pub mos6569r3_register_offset: u8,
    pub trigger_delay_cycles: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicSpriteTiming {
    pub ba_cycle_count: u8,
    pub ba_first_cycle: u8,
    pub bytes_per_row: u8,
    pub data_first_cycle: u8,
    pub dma_check_cycles: [u8; 2],
    pub expansion_check_cycle: u8,
    pub line_data_ready_cycle: u8,
    pub memory_counter_crunch_cycle: u8,
    pub memory_counter_update_cycle: u8,
    pub prepare_display_cycle: u8,
    pub start_cycle_spacing: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicTiming {
    pub bad_line: VicBadLineTiming,
    pub border: VicBorderTiming,
    pub cycles_per_raster_line: u8,
    pub fetch: VicFetchTiming,
    pub light_pen: VicLightPenTiming,
    pub raster_line_count: u16,
    pub sprite: VicSpriteTiming,
}

pub const PAL_VIC_TIMING: VicTiming = VicTiming {
    bad_line: VicBadLineTiming {
        ba_first_cycle: 12,
        ba_last_cycle: 54,
        first_raster_line: 0x30,
        last_raster_line: 0xf7,
    },
    border: VicBorderTiming {
        reduced_column_left_cycle: 18,
        reduced_column_right_cycle: 56,
        reduced_row_start_line: 0x37,
        reduced_row_stop_line: 0xf7,
        standard_column_left_cycle: 17,
        standard_column_right_cycle: 57,
        standard_row_start_line: 0x33,
        standard_row_stop_line: 0xfb,
    },
    cycles_per_raster_line: 63,
    fetch: VicFetchTiming {
        graphics_first_cycle: 16,
        graphics_last_cycle: 55,
        idle_first_cycle: 56,
        idle_last_cycle: 57,
        matrix_first_cycle: 15,
        matrix_last_cycle: 54,
        row_counter_update_cycle: 58,
        refresh_first_cycle: 11,
        refresh_last_cycle: 15,
        video_counter_reload_cycle: 14,
    },
    light_pen: VicLightPenTiming {
        horizontal_counter_granularity_pixels: 8,
        horizontal_origin_pixels: 0x194,
        horizontal_position_modulo_pixels: 504,
        mos6569r3_register_offset: 2,
        trigger_delay_cycles: 1,
    },
    raster_line_count: 312,
    sprite: VicSpriteTiming {
        ba_cycle_count: 5,
        ba_first_cycle: 55,
        bytes_per_row: 3,
        data_first_cycle: 58,
        dma_check_cycles: [55, 56],
        expansion_check_cycle: 56,
        line_data_ready_cycle: 10,
        memory_counter_crunch_cycle: 15,
        memory_counter_update_cycle: 16,
        prepare_display_cycle: 58,
        start_cycle_spacing: 2,
    },
};
