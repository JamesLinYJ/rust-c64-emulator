// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 memory decoder integration tests
//
//   File:       tests/drive1541_memory.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::drive1541::disk_via::Drive1541DiskVia;
use c64_core::devices::drive1541::iec_via::Drive1541IecVia;
use c64_core::devices::drive1541::mechanism::Drive1541Mechanism;
use c64_core::devices::drive1541::memory::{
    DRIVE_1541_ROM_SIZE, Drive1541Memory, Drive1541MemoryError,
};
use c64_core::devices::iec::IecBus;
use c64_core::devices::via::register;

struct Fixture {
    bus: IecBus,
    mechanism: Drive1541Mechanism,
    memory: Drive1541Memory,
    rom: [u8; DRIVE_1541_ROM_SIZE],
}

fn create_fixture() -> Fixture {
    let rom = std::array::from_fn(|index| index.to_le_bytes()[0]);
    let mut bus = IecBus::new();
    let mut mechanism = Drive1541Mechanism::new();
    let iec_via = Drive1541IecVia::new(8, &mut bus).unwrap();
    let disk_via = Drive1541DiskVia::new(8, &mut mechanism).unwrap();
    let memory = Drive1541Memory::new(&rom, iec_via, disk_via).unwrap();
    Fixture {
        bus,
        mechanism,
        memory,
        rom,
    }
}

#[test]
fn mirrors_ram_only_in_the_four_decoded_ranges() {
    let mut fixture = create_fixture();
    fixture
        .memory
        .write(&mut fixture.bus, &mut fixture.mechanism, 0x0123, 0xa5)
        .unwrap();
    for address in [0x2123, 0x4123, 0x6123] {
        assert_eq!(
            fixture
                .memory
                .read(&mut fixture.bus, &mut fixture.mechanism, address)
                .unwrap(),
            0xa5
        );
    }

    fixture
        .memory
        .write(&mut fixture.bus, &mut fixture.mechanism, 0x0923, 0x5a)
        .unwrap();
    assert_eq!(fixture.memory.last_data_bus_value(), 0x5a);
    assert_eq!(fixture.memory.ram()[0x0123], 0xa5);
}

#[test]
fn mirrors_both_via_windows_every_eight_kibibytes() {
    let mut fixture = create_fixture();
    fixture
        .memory
        .write(
            &mut fixture.bus,
            &mut fixture.mechanism,
            0x1800 + u16::from(register::DATA_DIRECTION_B),
            0x5a,
        )
        .unwrap();
    fixture
        .memory
        .write(
            &mut fixture.bus,
            &mut fixture.mechanism,
            0x1c00 + u16::from(register::DATA_DIRECTION_A),
            0xa5,
        )
        .unwrap();

    for address in [0x3802, 0x5802, 0x7802] {
        assert_eq!(
            fixture
                .memory
                .read(&mut fixture.bus, &mut fixture.mechanism, address)
                .unwrap(),
            0x5a
        );
    }
    for address in [0x3c03, 0x5c03, 0x7c03] {
        assert_eq!(
            fixture
                .memory
                .read(&mut fixture.bus, &mut fixture.mechanism, address)
                .unwrap(),
            0xa5
        );
    }
}

#[test]
fn mirrors_the_dos_rom_and_ignores_rom_writes() {
    let mut fixture = create_fixture();
    for (address, offset) in [(0x8000, 0), (0xbfff, 0x3fff), (0xc000, 0), (0xffff, 0x3fff)] {
        assert_eq!(
            fixture
                .memory
                .read(&mut fixture.bus, &mut fixture.mechanism, address)
                .unwrap(),
            fixture.rom[offset]
        );
    }
    fixture
        .memory
        .write(&mut fixture.bus, &mut fixture.mechanism, 0xc000, 0xff)
        .unwrap();
    assert_eq!(
        fixture
            .memory
            .read(&mut fixture.bus, &mut fixture.mechanism, 0xc000)
            .unwrap(),
        fixture.rom[0]
    );
}

#[test]
fn open_bus_word_and_stack_accesses_preserve_byte_cycle_order() {
    let mut fixture = create_fixture();
    fixture
        .memory
        .write(&mut fixture.bus, &mut fixture.mechanism, 0x0800, 0x3c)
        .unwrap();
    assert_eq!(
        fixture
            .memory
            .read(&mut fixture.bus, &mut fixture.mechanism, 0x17ff)
            .unwrap(),
        0x3c
    );

    fixture
        .memory
        .write_word(&mut fixture.bus, &mut fixture.mechanism, 0x07ff, 0xa55a)
        .unwrap();
    assert_eq!(fixture.memory.ram()[0x07ff], 0x5a);
    assert_eq!(fixture.memory.last_data_bus_value(), 0xa5);
    fixture
        .memory
        .write_stack(&mut fixture.bus, &mut fixture.mechanism, 0xfe, 0x81)
        .unwrap();
    assert_eq!(
        fixture
            .memory
            .read_stack(&mut fixture.bus, &mut fixture.mechanism, 0xfe)
            .unwrap(),
        0x81
    );
    assert_eq!(
        fixture
            .memory
            .read_word(&mut fixture.bus, &mut fixture.mechanism, 0x01fe)
            .unwrap(),
        0x0081
    );
}

#[test]
fn reset_restores_open_bus_and_via_electronics_without_clearing_ram() {
    let mut fixture = create_fixture();
    fixture
        .memory
        .write(&mut fixture.bus, &mut fixture.mechanism, 0x0000, 0x42)
        .unwrap();
    fixture.mechanism.set_motor_on(true);
    fixture
        .memory
        .reset_hardware(&mut fixture.bus, &mut fixture.mechanism);

    assert_eq!(fixture.memory.last_data_bus_value(), 0xff);
    assert_eq!(fixture.memory.ram()[0], 0x42);
    assert!(!fixture.mechanism.motor_on());
    assert!(fixture.mechanism.led_on());
}

#[test]
fn rejects_nonstandard_rom_lengths() {
    let mut bus = IecBus::new();
    let mut mechanism = Drive1541Mechanism::new();
    let iec_via = Drive1541IecVia::new(8, &mut bus).unwrap();
    let disk_via = Drive1541DiskVia::new(8, &mut mechanism).unwrap();
    assert_eq!(
        Drive1541Memory::new(&[0; 0x2000], iec_via, disk_via),
        Err(Drive1541MemoryError::InvalidRomLength(0x2000))
    );
}
