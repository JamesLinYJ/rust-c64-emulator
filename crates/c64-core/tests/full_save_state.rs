// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 全硬件 save-state 根因与回放测试
//
//   文件:       tests/full_save_state.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::cartridge::{AMD_29F040B_UNLOCK_ADDRESS_1, AMD_29F040B_UNLOCK_ADDRESS_2};
use c64_core::devices::drive1541::mechanism::Drive1541DiskImage;
use c64_core::devices::drive1541::memory::DRIVE_1541_ROM_SIZE;
use c64_core::devices::reu::ReuSize;
use c64_core::media::crt::{CRT_CHIP_TYPE_FLASH, CRT_HARDWARE_TYPE_EASY_FLASH};
use c64_core::media::d64::{D64_SECTOR_SIZE, D64DiskImage, d64_sector_count_through_track};
use c64_core::media::g64::G64DiskImage;
use c64_core::media::tap::TapVideoStandard;
use c64_core::{C64Core, C64Firmware, CoreConfig, CoreError, MemoryWriteSource, StateError};

#[test]
fn load_restores_attached_hardware_and_replays_from_a_cpu_microcycle() {
    let mut checkpoint = C64Core::new(CoreConfig::default());
    checkpoint
        .attach_drive1541(8, &vec![0; DRIVE_1541_ROM_SIZE])
        .unwrap();
    checkpoint.attach_reu(ReuSize::Kib512).unwrap();
    checkpoint.insert_blank_tap(TapVideoStandard::Pal).unwrap();
    checkpoint.tape_play().unwrap();
    checkpoint.write_base_ram(0x2000, 0x5a, MemoryWriteSource::HostLoader);
    {
        let reu = checkpoint.reu_mut().unwrap();
        reu.ram_mut()[0x1234] = 0xa5;
        reu.write_register(0xdf02, 0x34);
        reu.write_register(0xdf03, 0x12);
        reu.write_register(0xdf09, 0xe0);
    }

    checkpoint.run_cpu_slots(333).unwrap();
    while !checkpoint.cpu_is_at_instruction_boundary() {
        checkpoint.run_cpu_slots(1).unwrap();
    }
    checkpoint.run_cpu_slots(1).unwrap();
    assert!(!checkpoint.cpu_is_at_instruction_boundary());
    let encoded = checkpoint.save_state();

    let mut restored = C64Core::new(CoreConfig::default());
    restored.load_state(&encoded).unwrap();

    assert_eq!(restored.devices(), checkpoint.devices());
    assert_eq!(restored.memory(), checkpoint.memory());
    assert_eq!(restored.timestamp(), checkpoint.timestamp());
    assert_eq!(restored.cpu_state(), checkpoint.cpu_state());
    assert_eq!(restored.save_state(), encoded);

    for slots in [1, 7, 127, 511] {
        checkpoint.run_cpu_slots(slots).unwrap();
        restored.run_cpu_slots(slots).unwrap();
        assert_eq!(restored.devices(), checkpoint.devices());
        assert_eq!(restored.memory(), checkpoint.memory());
        assert_eq!(restored.timestamp(), checkpoint.timestamp());
        assert_eq!(restored.cpu_state(), checkpoint.cpu_state());
        assert_eq!(restored.save_state(), checkpoint.save_state());
    }
}

#[test]
fn malformed_v2_envelopes_are_rejected_atomically() {
    let encoded = C64Core::new(CoreConfig::default()).save_state();
    let mut target = C64Core::new(CoreConfig::default());
    target.run_cpu_slots(19).unwrap();
    let unchanged = target.clone();

    let mut bad_checksum = encoded.clone();
    *bad_checksum.last_mut().unwrap() ^= 0x80;
    assert!(matches!(
        target.load_state(&bad_checksum),
        Err(CoreError::State(StateError::ChecksumMismatch { .. }))
    ));
    assert_eq!(target, unchanged);

    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(matches!(
        target.load_state(&trailing),
        Err(CoreError::State(StateError::PayloadLengthMismatch { .. }))
    ));
    assert_eq!(target, unchanged);

    let mut oversized = encoded;
    oversized[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        target.load_state(&oversized),
        Err(CoreError::State(StateError::PayloadTooLarge { .. }))
    ));
    assert_eq!(target, unchanged);
}

#[test]
fn c64_firmware_is_supplied_by_the_runtime_instead_of_the_state_file() {
    let firmware = C64Firmware::new(
        &vec![0x42; 0x2000],
        &vec![0x24; 0x1000],
        &vec![0x81; 0x2000],
    )
    .unwrap();
    let blank = C64Core::new(CoreConfig::default());
    let populated = C64Core::with_firmware(CoreConfig::default(), firmware);

    assert_eq!(blank.save_state(), populated.save_state());
}

#[test]
fn easyflash_ram_and_in_flight_flash_command_survive_round_trip() {
    let mut core = C64Core::new(CoreConfig::default());
    core.insert_crt(&easyflash_crt(), false).unwrap();
    {
        let easyflash = core.easyflash_mut().unwrap();
        easyflash.write_io1(0xde00, 0x3f);
        easyflash.write_io1(0xde02, 0x87);
        easyflash.write_io2(0xdf42, 0xa5);
        let flash = easyflash.flash_low_mut();
        flash.write(AMD_29F040B_UNLOCK_ADDRESS_1, 0xaa).unwrap();
        flash.write(AMD_29F040B_UNLOCK_ADDRESS_2, 0x55).unwrap();
        flash.write(AMD_29F040B_UNLOCK_ADDRESS_1, 0xa0).unwrap();
        flash.write(0x1234, 0x5a).unwrap();
        assert!(flash.is_busy());
    }

    assert_full_round_trip(core);
}

#[test]
fn d64_and_g64_media_survive_drive_round_trip() {
    let d64_length = d64_sector_count_through_track(35).unwrap() * D64_SECTOR_SIZE;
    let disks = [
        Drive1541DiskImage::D64(D64DiskImage::parse(&vec![0; d64_length], false).unwrap()),
        Drive1541DiskImage::G64(G64DiskImage::parse(&minimal_g64(), false).unwrap()),
    ];

    for disk in disks {
        let mut core = C64Core::new(CoreConfig::default());
        core.attach_drive1541(8, &vec![0; DRIVE_1541_ROM_SIZE])
            .unwrap();
        core.drive1541_mut().unwrap().mount_disk(disk).unwrap();
        core.run_cpu_slots(31).unwrap();
        assert_full_round_trip(core);
    }
}

fn assert_full_round_trip(mut checkpoint: C64Core) {
    let encoded = checkpoint.save_state();
    let mut restored = C64Core::new(CoreConfig::default());
    restored.load_state(&encoded).unwrap();
    assert_eq!(restored.devices(), checkpoint.devices());
    assert_eq!(restored.memory(), checkpoint.memory());
    assert_eq!(restored.timestamp(), checkpoint.timestamp());
    assert_eq!(restored.cpu_state(), checkpoint.cpu_state());
    assert_eq!(restored.save_state(), encoded);

    checkpoint.run_cpu_slots(17).unwrap();
    restored.run_cpu_slots(17).unwrap();
    assert_eq!(restored.devices(), checkpoint.devices());
    assert_eq!(restored.memory(), checkpoint.memory());
    assert_eq!(restored.save_state(), checkpoint.save_state());
}

fn easyflash_crt() -> Vec<u8> {
    let data = vec![0xff; 0x4000];
    let mut bytes = vec![0; 0x40 + 0x10 + data.len()];
    bytes[..16].copy_from_slice(b"C64 CARTRIDGE   ");
    write_be_u32(&mut bytes, 0x10, 0x40);
    write_be_u16(&mut bytes, 0x14, 0x0100);
    write_be_u16(&mut bytes, 0x16, CRT_HARDWARE_TYPE_EASY_FLASH);
    bytes[0x18] = 1;
    bytes[0x40..0x44].copy_from_slice(b"CHIP");
    write_be_u32(&mut bytes, 0x44, u32::try_from(0x10 + data.len()).unwrap());
    write_be_u16(&mut bytes, 0x48, CRT_CHIP_TYPE_FLASH);
    write_be_u16(&mut bytes, 0x4a, 0);
    write_be_u16(&mut bytes, 0x4c, 0x8000);
    write_be_u16(&mut bytes, 0x4e, u16::try_from(data.len()).unwrap());
    bytes[0x50..].copy_from_slice(&data);
    bytes
}

fn minimal_g64() -> Vec<u8> {
    const HALF_TRACK_COUNT: usize = 4;
    let mut bytes = vec![0; 12 + HALF_TRACK_COUNT * 8];
    bytes[..8].copy_from_slice(b"GCR-1541");
    bytes[8] = 0;
    bytes[9] = u8::try_from(HALF_TRACK_COUNT).unwrap();
    bytes[10..12].copy_from_slice(&16_u16.to_le_bytes());
    bytes
}

fn write_be_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_be_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
