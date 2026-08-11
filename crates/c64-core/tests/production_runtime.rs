// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 生产 Wasm 运行边界验收
//
//   文件:       production_runtime.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::input::{C64_CONTROL_PORT_DIGITAL_MASK, RESTORE_NMI_PULSE_CYCLES};
use c64_core::devices::{C64Chipset, vic};
use c64_core::media::prg::BASIC_PRG_LOAD_ADDRESS;
use c64_core::{
    C64BusDevices, C64Core, MemoryWriteSource, VideoStandard,
    devices::cia::register as cia_register,
};

const READY_SCREEN_CODES: [u8; 6] = [0x12, 0x05, 0x01, 0x04, 0x19, 0x2e];

#[test]
fn host_input_drives_both_keyboard_scan_directions_and_control_ports() {
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    let mut rows_by_column = [0_u8; 8];
    rows_by_column[1] = 1 << 2;
    chipset
        .set_host_input(&rows_by_column, false, 1 << 4, 1 << 0, false)
        .unwrap();

    chipset.write_io(0xdc02 | u16::from(cia_register::DATA_DIRECTION_A), 0xff);
    chipset.write_io(0xdc00 | u16::from(cia_register::PORT_A), !(1 << 1));
    chipset.write_io(0xdc03 | u16::from(cia_register::DATA_DIRECTION_B), 0x00);
    let forward = chipset.read_io(0xdc00 | u16::from(cia_register::PORT_B), 0xff);
    assert_eq!(
        forward & (1 << 2),
        0,
        "PA column scan must find the key row"
    );
    assert_eq!(forward & (1 << 4), 0, "joystick port 1 must ground CIA1 PB");

    chipset.write_io(0xdc02 | u16::from(cia_register::DATA_DIRECTION_A), 0x00);
    chipset.write_io(0xdc03 | u16::from(cia_register::DATA_DIRECTION_B), 0xff);
    chipset.write_io(0xdc00 | u16::from(cia_register::PORT_B), !(1 << 2));
    let reverse = chipset.read_io(0xdc00 | u16::from(cia_register::PORT_A), 0xff);
    assert_eq!(
        reverse & (1 << 1),
        0,
        "PB row scan must find the key column"
    );
    assert_eq!(reverse & (1 << 0), 0, "joystick port 2 must ground CIA1 PA");

    assert!(
        chipset
            .set_host_input(
                &rows_by_column,
                false,
                C64_CONTROL_PORT_DIGITAL_MASK + 1,
                0,
                false,
            )
            .is_err(),
        "invalid joystick lines must be rejected atomically",
    );
}

#[test]
fn restore_press_edge_produces_one_fixed_width_nmi_pulse() {
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    chipset.set_host_input(&[0; 8], false, 0, 0, true).unwrap();
    assert!(chipset.nmi_asserted());

    for _ in 0..RESTORE_NMI_PULSE_CYCLES - 1 {
        chipset.clock_host_input();
    }
    assert!(chipset.nmi_asserted());
    chipset.clock_host_input();
    assert!(!chipset.nmi_asserted());

    chipset.set_host_input(&[0; 8], false, 0, 0, true).unwrap();
    assert!(
        !chipset.nmi_asserted(),
        "holding RESTORE must not retrigger"
    );
    chipset.set_host_input(&[0; 8], false, 0, 0, false).unwrap();
    chipset.set_host_input(&[0; 8], false, 0, 0, true).unwrap();
    assert!(chipset.nmi_asserted(), "a later press edge must retrigger");
}

#[test]
fn one_coarse_runtime_call_commits_a_frame_and_bounded_pcm_batch() {
    let mut core = C64Core::default();
    core.request_manual_turbo(20).unwrap();
    let initial_generation = core.devices().vic().frame_generation();
    let elapsed = core.run_until_next_video_frame(25_000).unwrap();

    assert!(elapsed > 0);
    assert_ne!(core.devices().vic().frame_generation(), initial_generation);
    assert_eq!(
        core.timestamp().slot,
        0,
        "coarse frame calls must return at a safe system-cycle boundary",
    );
    assert_eq!(
        core.devices().vic().frame_pixels().len(),
        vic::VIC_RASTER_OUTPUT_WIDTH * vic::PAL_RASTER_OUTPUT_HEIGHT,
    );

    let mut samples = [0.0_f32; 2_048];
    let count = core.pull_sid_samples_into(&mut samples);
    assert!(count > 0);
    assert!(count <= samples.len());
}

#[test]
fn coarse_basic_prg_install_validates_before_mutating_ram() {
    let mut core = C64Core::default();
    write_word(&mut core, 0x002b, BASIC_PRG_LOAD_ADDRESS);
    core.write_base_ram(0x0289, 10, MemoryWriteSource::HostLoader);
    let generation_before = core.memory().memory_generation();

    let invalid = [0x00, 0x20, 0xea];
    assert!(core.install_basic_prg(&invalid).is_err());
    assert_eq!(core.memory().memory_generation(), generation_before);

    let valid = [0x01, 0x08, 0x0b, 0x08, 0x00, 0x00];
    let loaded = core.install_basic_prg(&valid).unwrap();
    assert_eq!(loaded.load_address, BASIC_PRG_LOAD_ADDRESS);
    assert_eq!(loaded.size, valid.len() - 2);
    assert_eq!(core.read_base_ram(BASIC_PRG_LOAD_ADDRESS), 0x0b);
    assert_eq!(read_word(&core, 0x002d), BASIC_PRG_LOAD_ADDRESS + 4);
    assert_eq!(core.read_base_ram(0x00c6), 4);
    assert_eq!(
        [
            core.read_base_ram(0x0277),
            core.read_base_ram(0x0278),
            core.read_base_ram(0x0279),
            core.read_base_ram(0x027a),
        ],
        [0x52, 0x55, 0x4e, 0x0d],
    );
}

#[test]
fn basic_ready_is_detected_from_the_physical_screen_ram() {
    let mut core = C64Core::default();
    assert!(!core.basic_ready());
    for (offset, value) in READY_SCREEN_CODES.into_iter().enumerate() {
        core.write_base_ram(
            0x0400 + u16::try_from(offset).unwrap(),
            value,
            MemoryWriteSource::HostLoader,
        );
    }
    assert!(core.basic_ready());
}

fn write_word(core: &mut C64Core, address: u16, value: u16) {
    let [low, high] = value.to_le_bytes();
    core.write_base_ram(address, low, MemoryWriteSource::HostLoader);
    core.write_base_ram(address + 1, high, MemoryWriteSource::HostLoader);
}

fn read_word(core: &C64Core, address: u16) -> u16 {
    u16::from_le_bytes([core.read_base_ram(address), core.read_base_ram(address + 1)])
}
