// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1530 Datasette integration tests
//
//   File:       tests/datasette.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::cia::{interrupt, register};
use c64_core::devices::tape::{
    Commodore1530Datasette, DatasetteHostSignals, DatasetteTape, DatasetteTransport,
};
use c64_core::devices::vic::VicMemoryBus;
use c64_core::media::tap::{TAP_HEADER_SIZE, TapImage, TapVideoStandard, WritableTapImage};
use c64_core::{C64AddressSpace, C64Chipset, C64Firmware, CpuBus, VideoStandard};

fn fixture(data: &[u8], video_standard: u8) -> Vec<u8> {
    let mut bytes = vec![0; TAP_HEADER_SIZE + data.len()];
    bytes[..12].copy_from_slice(b"C64-TAPE-RAW");
    bytes[12] = 1;
    bytes[13] = 0;
    bytes[14] = video_standard;
    bytes[16..20].copy_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    bytes[TAP_HEADER_SIZE..].copy_from_slice(data);
    bytes
}

fn read_only_tape(data: &[u8], video_standard: u8) -> DatasetteTape {
    DatasetteTape::ReadOnly(TapImage::parse(&fixture(data, video_standard), None).unwrap())
}

#[test]
fn play_closes_sense_but_the_motor_gates_pulse_progress() {
    let mut datasette = Commodore1530Datasette::new(985_248).unwrap();
    datasette.insert_tape(read_only_tape(&[2, 3], 0)).unwrap();
    datasette.press_play();
    assert!(datasette.sense_switch_closed());

    assert_eq!(datasette.clock_cycles(100).unwrap().read_pulses, 0);
    assert_eq!(datasette.pulse_index(), 0);
    datasette
        .set_host_signals(DatasetteHostSignals {
            motor_active: true,
            write_high: true,
        })
        .unwrap();
    assert_eq!(datasette.clock_cycles(15).unwrap().read_pulses, 0);
    assert_eq!(datasette.clock_cycle().unwrap().read_pulses, 1);
    assert_eq!(datasette.pulse_index(), 1);
    assert_eq!(datasette.clock_cycles(24).unwrap().read_pulses, 1);
    assert_eq!(datasette.transport(), DatasetteTransport::Stopped);
    assert!(!datasette.sense_switch_closed());
}

#[test]
fn playback_scales_capture_clocks_with_a_persistent_integer_remainder() {
    let mut datasette = Commodore1530Datasette::new(985_248).unwrap();
    let ntsc_second = 1_022_730_u32;
    datasette
        .insert_tape(read_only_tape(
            &[
                0,
                ntsc_second.to_le_bytes()[0],
                ntsc_second.to_le_bytes()[1],
                ntsc_second.to_le_bytes()[2],
            ],
            1,
        ))
        .unwrap();
    datasette.press_play();
    datasette
        .set_host_signals(DatasetteHostSignals {
            motor_active: true,
            write_high: true,
        })
        .unwrap();

    assert_eq!(datasette.clock_cycles(985_247).unwrap().read_pulses, 0);
    assert_eq!(datasette.clock_cycle().unwrap().read_pulses, 1);
    assert_eq!(datasette.elapsed_target_cycles(), 985_248);
}

#[test]
fn record_measures_rising_write_edges_and_preserves_tap_quantization_remainder() {
    let mut datasette = Commodore1530Datasette::new(985_248).unwrap();
    datasette
        .insert_tape(DatasetteTape::Writable(WritableTapImage::new(
            TapVideoStandard::Pal,
        )))
        .unwrap();
    datasette.press_record().unwrap();
    datasette
        .set_host_signals(DatasetteHostSignals {
            motor_active: true,
            write_high: true,
        })
        .unwrap();
    datasette.clock_cycles(385).unwrap();
    datasette
        .set_host_signals(DatasetteHostSignals {
            motor_active: true,
            write_high: false,
        })
        .unwrap();
    datasette
        .set_host_signals(DatasetteHostSignals {
            motor_active: true,
            write_high: true,
        })
        .unwrap();
    datasette.clock_cycles(527).unwrap();
    datasette
        .set_host_signals(DatasetteHostSignals {
            motor_active: true,
            write_high: false,
        })
        .unwrap();
    datasette
        .set_host_signals(DatasetteHostSignals {
            motor_active: true,
            write_high: true,
        })
        .unwrap();

    assert_eq!(
        datasette
            .mounted_tape()
            .unwrap()
            .pulses()
            .iter()
            .map(|pulse| pulse.source_cycles)
            .collect::<Vec<_>>(),
        [384, 528]
    );
    assert_eq!(datasette.pulse_index(), 2);
    assert_eq!(datasette.elapsed_target_cycles(), 912);
}

#[test]
fn chipset_wires_6510_motor_write_and_sense_to_cia1_flag() {
    let mut address_space = C64AddressSpace::new(C64Firmware::blank_for_test());
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    chipset
        .datasette_mut()
        .insert_tape(read_only_tape(&[1], 0))
        .unwrap();
    chipset.datasette_mut().press_play();

    {
        let mut bus = address_space.cpu_bus(&mut chipset);
        assert_eq!(bus.read(0x0001) & 0x10, 0, "PLAY must pull SENSE low");
        bus.write(0x0000, 0x28);
        bus.write(0x0001, 0x08);
        bus.write(0xdc0d, 0x80 | interrupt::FLAG);
    }
    assert!(chipset.datasette().motor_active());
    assert!(chipset.datasette().host_signals().write_high);

    let mut memory = ZeroVicMemory;
    for _ in 0..8 {
        chipset.clock_system_cycle(&mut memory).unwrap();
    }
    let interrupt_control = {
        let mut bus = address_space.cpu_bus(&mut chipset);
        bus.read(0xdc00 | u16::from(register::INTERRUPT_CONTROL))
    };
    assert_eq!(interrupt_control & interrupt::FLAG, interrupt::FLAG);
}

struct ZeroVicMemory;

impl VicMemoryBus for ZeroVicMemory {
    fn cpu_data_bus_value(&self) -> u8 {
        0xff
    }

    fn read_vic_byte(&mut self, _address_in_bank: u16) -> u8 {
        0
    }

    fn read_vic_color(&mut self, _index: u16) -> u8 {
        0
    }
}
