// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - cartridge board integration tests
//
//   File:       tests/cartridge.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::cartridge::{Cartridge, CartridgeKind};
use c64_core::media::crt::{
    CRT_CHIP_TYPE_ROM, CRT_HARDWARE_TYPE_MAGIC_DESK, CRT_HARDWARE_TYPE_OCEAN,
    CRT_HARDWARE_TYPE_STANDARD,
};
use c64_core::{C64AddressSpace, C64BusDevices, C64Chipset, C64Firmware, CpuBus, VideoStandard};

const BANK_SIZE: usize = 0x2000;

struct TestChip {
    bank: u16,
    data: Vec<u8>,
    load_address: u16,
}

fn fixture(
    chips: &[TestChip],
    hardware_type: u16,
    game_line_high: bool,
    exrom_line_high: bool,
) -> Vec<u8> {
    let header_size = 0x40_usize;
    let length = header_size
        + chips
            .iter()
            .map(|chip| 0x10 + chip.data.len())
            .sum::<usize>();
    let mut bytes = vec![0; length];
    bytes[..16].copy_from_slice(b"C64 CARTRIDGE   ");
    write_u32(&mut bytes, 0x10, u32::try_from(header_size).unwrap());
    write_u16(&mut bytes, 0x14, 0x0100);
    write_u16(&mut bytes, 0x16, hardware_type);
    bytes[0x18] = u8::from(exrom_line_high);
    bytes[0x19] = u8::from(game_line_high);

    let mut offset = header_size;
    for chip in chips {
        let packet_length = 0x10 + chip.data.len();
        bytes[offset..offset + 4].copy_from_slice(b"CHIP");
        write_u32(
            &mut bytes,
            offset + 4,
            u32::try_from(packet_length).unwrap(),
        );
        write_u16(&mut bytes, offset + 8, CRT_CHIP_TYPE_ROM);
        write_u16(&mut bytes, offset + 0x0a, chip.bank);
        write_u16(&mut bytes, offset + 0x0c, chip.load_address);
        write_u16(
            &mut bytes,
            offset + 0x0e,
            u16::try_from(chip.data.len()).unwrap(),
        );
        bytes[offset + 0x10..offset + 0x10 + chip.data.len()].copy_from_slice(&chip.data);
        offset += packet_length;
    }
    bytes
}

fn filled(value: u8) -> Vec<u8> {
    vec![value; BANK_SIZE]
}

fn banked_fixture(hardware_type: u16, count: u16, base: u8) -> Vec<u8> {
    fixture(
        &(0..count)
            .map(|bank| TestChip {
                bank,
                data: filled(base.wrapping_add(u8::try_from(bank).unwrap())),
                load_address: 0x8000,
            })
            .collect::<Vec<_>>(),
        hardware_type,
        false,
        false,
    )
}

fn mounted_bus(bytes: &[u8]) -> (C64AddressSpace, C64Chipset) {
    let cartridge = Cartridge::from_crt_bytes(bytes, false).unwrap();
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    chipset.attach_cartridge(cartridge).unwrap();
    (C64AddressSpace::new(C64Firmware::blank_for_test()), chipset)
}

#[test]
fn standard_8k_16k_and_ultimax_follow_the_pla_without_shadow_write_leaks() {
    let standard_8k = fixture(
        &[TestChip {
            bank: 0,
            data: filled(0x81),
            load_address: 0x8000,
        }],
        CRT_HARDWARE_TYPE_STANDARD,
        true,
        false,
    );
    let (mut address_space, mut chipset) = mounted_bus(&standard_8k);
    {
        let mut bus = address_space.cpu_bus(&mut chipset);
        assert_eq!(bus.read(0x8000), 0x81);
        bus.write(0x8000, 0x42);
        assert_eq!(bus.read(0x8000), 0x81);
        bus.write(0x0000, 0x07);
        bus.write(0x0001, 0x00);
        assert_eq!(bus.read(0x8000), 0x42);
    }

    let standard_16k = fixture(
        &[
            TestChip {
                bank: 0,
                data: filled(0x82),
                load_address: 0x8000,
            },
            TestChip {
                bank: 0,
                data: filled(0xa2),
                load_address: 0xa000,
            },
        ],
        CRT_HARDWARE_TYPE_STANDARD,
        false,
        false,
    );
    let (mut address_space, mut chipset) = mounted_bus(&standard_16k);
    let mut bus = address_space.cpu_bus(&mut chipset);
    assert_eq!(bus.read(0x8000), 0x82);
    assert_eq!(bus.read(0xa000), 0xa2);

    let ultimax = fixture(
        &[
            TestChip {
                bank: 0,
                data: filled(0x88),
                load_address: 0x8000,
            },
            TestChip {
                bank: 0,
                data: filled(0xe8),
                load_address: 0xe000,
            },
        ],
        CRT_HARDWARE_TYPE_STANDARD,
        false,
        true,
    );
    let (mut address_space, mut chipset) = mounted_bus(&ultimax);
    address_space.write_base_ram(0x8000, 0x22, c64_core::MemoryWriteSource::HostLoader);
    {
        let mut bus = address_space.cpu_bus(&mut chipset);
        assert_eq!(bus.read(0x8000), 0x88);
        assert_eq!(bus.read(0xe000), 0xe8);
        bus.write(0x8000, 0x55);
    }
    assert_eq!(address_space.read_base_ram(0x8000), 0x22);
}

#[test]
fn ocean_and_magic_desk_decode_mirrored_io1_bank_registers() {
    let ocean = banked_fixture(CRT_HARDWARE_TYPE_OCEAN, 4, 0x40);
    let (mut address_space, mut chipset) = mounted_bus(&ocean);
    {
        let mut bus = address_space.cpu_bus(&mut chipset);
        assert_eq!(bus.read(0x8000), 0x40);
        assert_eq!(bus.read(0xa000), 0x40);
        bus.write(0xde7f, 0x83);
        assert_eq!(bus.read(0x9fff), 0x43);
        assert_eq!(bus.read(0xbfff), 0x43);
    }
    chipset.reset().unwrap();
    assert_eq!(address_space.cpu_bus(&mut chipset).read(0x8000), 0x40);

    let magic_desk = banked_fixture(CRT_HARDWARE_TYPE_MAGIC_DESK, 4, 0x80);
    let (mut address_space, mut chipset) = mounted_bus(&magic_desk);
    address_space.write_base_ram(0x8000, 0x35, c64_core::MemoryWriteSource::HostLoader);
    let mut bus = address_space.cpu_bus(&mut chipset);
    bus.write(0xde00, 0x02);
    assert_eq!(bus.read(0x8000), 0x82);
    bus.write(0xdeff, 0x83);
    assert_eq!(bus.read(0x8000), 0x35);
    bus.write(0xde7a, 0x01);
    assert_eq!(bus.read(0x8000), 0x81);
}

#[test]
fn chipset_owns_one_atomic_cartridge_slot_and_releases_lines_on_detach() {
    let image = banked_fixture(CRT_HARDWARE_TYPE_OCEAN, 4, 0x40);
    let cartridge = Cartridge::from_crt_bytes(&image, false).unwrap();
    assert_eq!(cartridge.kind(), CartridgeKind::Ocean);

    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    chipset.attach_cartridge(cartridge).unwrap();
    assert!(chipset.cartridge().is_some());
    assert!(!chipset.cartridge_lines().game_line_high);
    assert!(!chipset.cartridge_lines().exrom_line_high);
    assert!(
        chipset
            .attach_cartridge(Cartridge::from_crt_bytes(&image, false).unwrap())
            .is_err()
    );

    let detached = chipset.detach_cartridge().unwrap();
    assert_eq!(detached.kind(), CartridgeKind::Ocean);
    assert!(chipset.cartridge().is_none());
    assert!(chipset.cartridge_lines().game_line_high);
    assert!(chipset.cartridge_lines().exrom_line_high);
    assert!(chipset.detach_cartridge().is_err());
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
