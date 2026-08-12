// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE MOS 6567R8 NTSC timing acceptance
//
//   File:       ntsc_runtime.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::vic::{
    NTSC_RASTER_OUTPUT_HEIGHT, NTSC_VIC_TIMING, VicCycleSequencer, VicCycleSignals, VicPhi1Fetch,
};
use c64_core::{C64Core, CoreConfig, VideoStandard};

const VICE_6567R8_CYCLES_PER_FRAME: u64 = 65 * 263;

#[test]
fn ntsc_sequencer_matches_the_vice_6567r8_cycle_table() {
    assert_eq!(NTSC_VIC_TIMING.cycles_per_raster_line, 65);
    assert_eq!(NTSC_VIC_TIMING.raster_line_count, 263);
    assert_eq!(NTSC_VIC_TIMING.sprite.dma_check_cycles, [56, 57]);
    assert_eq!(NTSC_VIC_TIMING.sprite.data_first_cycle, 59);
    assert_eq!(NTSC_VIC_TIMING.sprite.prepare_display_cycle, 59);

    let mut sequencer = VicCycleSequencer::new_with_timing(NTSC_VIC_TIMING);
    let signals = VicCycleSignals::default();
    let mut observations = Vec::new();
    for _ in 0..65 {
        observations.push(sequencer.tick(&signals));
    }

    assert_eq!(observations[9].bus_schedule.phi1, VicPhi1Fetch::Idle);
    assert_eq!(
        observations[58].bus_schedule.phi1,
        VicPhi1Fetch::SpritePointer { sprite_index: 0 },
    );
    assert_eq!(
        observations[64].bus_schedule.phi1,
        VicPhi1Fetch::SpritePointer { sprite_index: 3 },
    );
    assert_eq!(observations[64].completed_raster_line, Some(0));
}

#[test]
fn ntsc_core_commits_one_263_by_65_frame_at_the_configured_clock_domain() {
    let mut core = C64Core::new(CoreConfig {
        video_standard: VideoStandard::Ntsc,
        ..CoreConfig::default()
    });

    assert_eq!(core.devices().vic().timing(), NTSC_VIC_TIMING);
    assert_eq!(
        core.devices().vic().frame_pixels().len(),
        c64_core::devices::vic::VIC_RASTER_OUTPUT_WIDTH * NTSC_RASTER_OUTPUT_HEIGHT,
    );

    let elapsed = core.run_until_next_video_frame(20_000).unwrap();
    assert_eq!(elapsed, VICE_6567R8_CYCLES_PER_FRAME);
    assert_eq!(core.devices().vic().frame_generation(), 1);
    assert_eq!(core.devices().vic().current_raster_line(), 262);
    assert_eq!(core.devices().vic().current_raster_cycle(), 65);
}

#[test]
fn ntsc_full_state_restores_the_video_domain_and_continues_deterministically() {
    let config = CoreConfig {
        video_standard: VideoStandard::Ntsc,
        ..CoreConfig::default()
    };
    let mut source = C64Core::new(config);
    source.run_system_cycles(1_337).unwrap();
    let state = source.save_state();

    let mut restored = C64Core::default();
    restored.load_state(&state).unwrap();
    assert_eq!(restored.config(), config);
    assert_eq!(restored.devices().vic().timing(), NTSC_VIC_TIMING);
    assert_eq!(restored.save_state(), state);

    source.run_system_cycles(4_096).unwrap();
    restored.run_system_cycles(4_096).unwrap();
    assert_eq!(restored.save_state(), source.save_state());
}
