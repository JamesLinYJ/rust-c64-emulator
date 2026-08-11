// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - NTSC coarse Wasm ABI acceptance
//
//   File:       ntsc_abi.rs
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_wasm::C64Vm;

#[test]
fn ntsc_vm_reports_its_frame_shape_and_exact_frame_cycle_count() {
    let mut vm = C64Vm::new(0, 1).unwrap();

    assert_eq!(vm.frame_width(), 403);
    assert_eq!(vm.frame_height(), 247);
    assert_eq!(vm.frame_pixels_len(), 403 * 247);
    assert_eq!(vm.video_standard(), 1);
    assert_eq!(vm.processor_clock_hz(), 1_022_727);
    assert_eq!(vm.video_frame_cycles(), 65 * 263);
    assert_eq!(vm.run_until_next_frame().unwrap(), 65 * 263);
}
