// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - EasyFlash cartridge integration tests
//
//   File:       tests/easyflash.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::cartridge::{
    AMD_29F040B_BYTE_PROGRAM_CYCLES, AMD_29F040B_CAPACITY_BYTES, Cartridge, EASY_FLASH_BANK_COUNT,
    EASY_FLASH_BANK_SIZE_BYTES, EasyFlashCartridge,
};
use c64_core::devices::vic::VicMemoryBus;
use c64_core::media::crt::{CRT_CHIP_TYPE_FLASH, CRT_HARDWARE_TYPE_EASY_FLASH};
use c64_core::{C64AddressSpace, C64Chipset, C64Firmware, CartridgeMode, CpuBus, VideoStandard};

fn banked_flash(base: u8) -> Vec<u8> {
    let mut bytes = vec![0; AMD_29F040B_CAPACITY_BYTES];
    for bank in 0..EASY_FLASH_BANK_COUNT {
        bytes[bank * EASY_FLASH_BANK_SIZE_BYTES..(bank + 1) * EASY_FLASH_BANK_SIZE_BYTES]
            .fill(base.wrapping_add(u8::try_from(bank).unwrap()));
    }
    bytes
}

fn create_cartridge(jumper_installed: bool) -> EasyFlashCartridge {
    EasyFlashCartridge::new(
        &banked_flash(0x00),
        &banked_flash(0x80),
        None,
        jumper_installed,
    )
    .unwrap()
}

fn mounted(cartridge: EasyFlashCartridge) -> (C64AddressSpace, C64Chipset) {
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    chipset
        .attach_cartridge(Cartridge::EasyFlash(cartridge))
        .unwrap();
    (C64AddressSpace::new(C64Firmware::blank_for_test()), chipset)
}

fn program_low_flash(bus: &mut impl CpuBus, address: u16, value: u8) {
    bus.write(0x8555, 0xaa);
    bus.write(0x82aa, 0x55);
    bus.write(0x8555, 0xa0);
    bus.write(address, value);
}

#[test]
fn registers_select_all_banks_modes_led_and_jumper_truth_table() {
    let mut cartridge = create_cartridge(false);
    assert_eq!(cartridge.mode(), CartridgeMode::Ultimax);
    cartridge.write_io1(0xdefc, 63);
    assert_eq!(cartridge.selected_bank(), 63);
    assert_eq!(cartridge.read_rom_low(0x8000), 0x3f);
    assert_eq!(cartridge.read_rom_high(0xe000), 0xbf);
    cartridge.write_io1(0xdeff, 0x86);
    assert_eq!(cartridge.mode_register(), 0x86);
    assert!(cartridge.led_on());
    assert_eq!(cartridge.mode(), CartridgeMode::Game8K);

    let expected = [
        CartridgeMode::Ultimax,
        CartridgeMode::Ultimax,
        CartridgeMode::Game16K,
        CartridgeMode::Game16K,
        CartridgeMode::Detached,
        CartridgeMode::Ultimax,
        CartridgeMode::Game8K,
        CartridgeMode::Game16K,
    ];
    for (register, mode) in expected.into_iter().enumerate() {
        cartridge.write_io1(0xde02, u8::try_from(register).unwrap());
        assert_eq!(cartridge.mode(), mode);
    }

    let mut jumper = create_cartridge(true);
    assert_eq!(jumper.mode(), CartridgeMode::Detached);
    jumper.write_io1(0xde02, 2);
    assert_eq!(jumper.mode(), CartridgeMode::Game8K);
}

#[test]
fn ultimax_programs_flash_but_normal_modes_write_hidden_c64_ram() {
    let (mut address_space, mut chipset) = mounted(create_cartridge(false));
    {
        let mut bus = address_space.cpu_bus(&mut chipset);
        bus.write(0xde00, 4);
        bus.write(0xde02, 0);
        bus.write(0xdf7a, 0x6c);
        program_low_flash(&mut bus, 0x8123, 0x00);
    }
    let mut vic_memory = ZeroVicMemory;
    for _ in 0..AMD_29F040B_BYTE_PROGRAM_CYCLES {
        chipset.clock_system_cycle(&mut vic_memory).unwrap();
    }
    {
        let mut bus = address_space.cpu_bus(&mut chipset);
        assert_eq!(bus.read(0x8123), 0x00);
        assert_eq!(bus.read(0xdf7a), 0x6c);
    }
    chipset.reset().unwrap();
    {
        let mut bus = address_space.cpu_bus(&mut chipset);
        assert_eq!(bus.read(0xdf7a), 0x6c);
        bus.write(0xde00, 4);
        assert_eq!(bus.read(0x8123), 0x00);
        bus.write(0xde02, 0x06);
        bus.write(0x8123, 0x5a);
    }
    assert_eq!(address_space.read_base_ram(0x8123), 0x5a);
}

#[test]
fn easyflash_crt_assembles_sparse_low_high_and_combined_chip_packets() {
    let bytes = easyflash_crt(&[
        (2, 0x8000, vec![0x22; 0x2000]),
        (2, 0xa000, vec![0xa2; 0x2000]),
        (7, 0x8000, {
            let mut data = vec![0x77; 0x4000];
            data[0x2000..].fill(0xf7);
            data
        }),
    ]);
    let cartridge = Cartridge::from_crt_bytes(&bytes, false).unwrap();
    let Cartridge::EasyFlash(mut easyflash) = cartridge else {
        panic!("EasyFlash CRT did not create an EasyFlash board");
    };
    easyflash.write_io1(0xde00, 2);
    assert_eq!(easyflash.read_rom_low(0x8000), 0x22);
    assert_eq!(easyflash.read_rom_high(0xe000), 0xa2);
    easyflash.write_io1(0xde00, 7);
    assert_eq!(easyflash.read_rom_low(0x8000), 0x77);
    assert_eq!(easyflash.read_rom_high(0xe000), 0xf7);
    easyflash.write_io1(0xde00, 1);
    assert_eq!(easyflash.read_rom_low(0x8000), 0xff);
    assert_eq!(easyflash.read_rom_high(0xe000), 0xff);
}

fn easyflash_crt(chips: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let header_size = 0x40_usize;
    let mut bytes =
        vec![0; header_size + chips.iter().map(|chip| 0x10 + chip.2.len()).sum::<usize>()];
    bytes[..16].copy_from_slice(b"C64 CARTRIDGE   ");
    write_u32(&mut bytes, 0x10, u32::try_from(header_size).unwrap());
    write_u16(&mut bytes, 0x14, 0x0100);
    write_u16(&mut bytes, 0x16, CRT_HARDWARE_TYPE_EASY_FLASH);
    bytes[0x18] = 1;
    bytes[0x19] = 0;
    let mut offset = header_size;
    for (bank, load_address, data) in chips {
        bytes[offset..offset + 4].copy_from_slice(b"CHIP");
        write_u32(
            &mut bytes,
            offset + 4,
            u32::try_from(0x10 + data.len()).unwrap(),
        );
        write_u16(&mut bytes, offset + 8, CRT_CHIP_TYPE_FLASH);
        write_u16(&mut bytes, offset + 0x0a, *bank);
        write_u16(&mut bytes, offset + 0x0c, *load_address);
        write_u16(
            &mut bytes,
            offset + 0x0e,
            u16::try_from(data.len()).unwrap(),
        );
        bytes[offset + 0x10..offset + 0x10 + data.len()].copy_from_slice(data);
        offset += 0x10 + data.len();
    }
    bytes
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
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
