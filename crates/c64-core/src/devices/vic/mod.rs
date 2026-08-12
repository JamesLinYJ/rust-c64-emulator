// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VIC-II timing subsystem
//
//   File:       vic/mod.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

mod bad_line;
mod border;
mod bus_schedule;
mod device;
mod fetch;
mod pixel;
mod sequencer;
mod sprite;
mod sprite_dma;
pub mod timing;

pub const SPRITE_COUNT: u8 = 8;
pub const MAX_CYCLES_PER_RASTER_LINE: u8 = 65;
pub const PAL_CYCLES_PER_RASTER_LINE: u8 = 63;
pub const PAL_FIRST_VISIBLE_RASTER: u16 = 16;
pub const PAL_LAST_VISIBLE_RASTER_EXCLUSIVE: u16 = 300;
pub const PAL_RASTER_OUTPUT_HEIGHT: usize =
    (PAL_LAST_VISIBLE_RASTER_EXCLUSIVE - PAL_FIRST_VISIBLE_RASTER) as usize;
pub const NTSC_CYCLES_PER_RASTER_LINE: u8 = 65;
pub const NTSC_FIRST_VISIBLE_RASTER: u16 = 8;
pub const NTSC_LAST_VISIBLE_RASTER_EXCLUSIVE: u16 = 255;
pub const NTSC_RASTER_OUTPUT_HEIGHT: usize =
    (NTSC_LAST_VISIBLE_RASTER_EXCLUSIVE - NTSC_FIRST_VISIBLE_RASTER) as usize;

pub use bad_line::{
    VicBadLineController, VicBadLineCycle, VicBadLineSignals, VicMatrixAccess,
    VicMatrixAccessSource,
};
pub use border::{VicBorderController, VicBorderSignals};
pub use bus_schedule::{
    VicBusScheduleEntry, VicCycleRangeError, VicPhi1Fetch, VicPhi2Fetch, vic_bus_schedule_for_cycle,
};
pub use device::{VIC_REGISTER_COUNT, VicError, VicII, VicRasterLineSnapshot};
pub use fetch::{
    VIC_MATRIX_COLUMN_COUNT, VIC_SPRITE_COUNT, VicFetchError, VicFetchPipeline, VicFetchRegisters,
    VicFetchSnapshot, VicMemoryBus,
};
pub use pixel::{
    C64_PALETTE, VIC_RASTER_OUTPUT_WIDTH, VicPixelCollisions, VicPixelDataSource, VicPixelError,
    VicPixelModes, VicPixelPipeline, VicPixelRegisters,
};
pub use sequencer::{VicCycleResult, VicCycleSequencer, VicCycleSignals};
pub use sprite::VicSprite;
pub use timing::{NTSC_VIC_TIMING, PAL_VIC_TIMING, VicTiming};
