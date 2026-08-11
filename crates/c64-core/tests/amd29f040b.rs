// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - AMD AM29F040B integration tests
//
//   File:       tests/amd29f040b.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::cartridge::{
    AMD_29F040B_BYTE_PROGRAM_CYCLES, AMD_29F040B_CAPACITY_BYTES, AMD_29F040B_CHIP_ERASE_CYCLES,
    AMD_29F040B_SECTOR_ERASE_CYCLES, AMD_29F040B_SECTOR_ERASE_WINDOW_CYCLES,
    AMD_29F040B_SECTOR_SIZE_BYTES, AMD_29F040B_STATUS_TOGGLE_BIT, AMD_29F040B_UNLOCK_ADDRESS_1,
    AMD_29F040B_UNLOCK_ADDRESS_2, Amd29F040BFlash, Amd29F040BState,
};

const PROGRAM_ADDRESS: usize = 0x12345;

fn create_flash(fill: u8) -> Amd29F040BFlash {
    Amd29F040BFlash::new(&vec![fill; AMD_29F040B_CAPACITY_BYTES]).unwrap()
}

fn unlocked_command(flash: &mut Amd29F040BFlash, command: u8) {
    flash.write(AMD_29F040B_UNLOCK_ADDRESS_1, 0xaa).unwrap();
    flash.write(AMD_29F040B_UNLOCK_ADDRESS_2, 0x55).unwrap();
    flash.write(AMD_29F040B_UNLOCK_ADDRESS_1, command).unwrap();
}

fn begin_erase(flash: &mut Amd29F040BFlash) {
    unlocked_command(flash, 0x80);
    flash.write(AMD_29F040B_UNLOCK_ADDRESS_1, 0xaa).unwrap();
    flash.write(AMD_29F040B_UNLOCK_ADDRESS_2, 0x55).unwrap();
}

#[test]
fn autoselect_and_program_busy_status_match_the_amd_command_protocol() {
    let mut flash = create_flash(0xff);
    unlocked_command(&mut flash, 0x90);
    assert_eq!(flash.state(), Amd29F040BState::Autoselect);
    assert_eq!(flash.read(0).unwrap(), 0x01);
    assert_eq!(flash.read(1).unwrap(), 0xa4);
    assert_eq!(flash.read(2).unwrap(), 0);
    flash.write(0, 0xf0).unwrap();

    unlocked_command(&mut flash, 0xa0);
    flash.write(PROGRAM_ADDRESS, 0x5a).unwrap();
    assert_eq!(flash.state(), Amd29F040BState::ByteProgramBusy);
    assert_eq!(flash.peek(PROGRAM_ADDRESS).unwrap(), 0xff);
    let first = flash.read(PROGRAM_ADDRESS).unwrap();
    let second = flash.read(PROGRAM_ADDRESS).unwrap();
    assert_eq!((first ^ second) & AMD_29F040B_STATUS_TOGGLE_BIT, 0x40);
    assert_eq!(first & 0x80, 0x80);

    flash.clock_cycles(AMD_29F040B_BYTE_PROGRAM_CYCLES - 1);
    assert_eq!(flash.peek(PROGRAM_ADDRESS).unwrap(), 0xff);
    flash.clock_cycles(1);
    assert_eq!(flash.state(), Amd29F040BState::Read);
    assert_eq!(flash.read(PROGRAM_ADDRESS).unwrap(), 0x5a);
    assert!(flash.dirty());
}

#[test]
fn zero_to_one_program_attempt_latches_dq5_until_read_reset() {
    let mut flash = create_flash(0x00);
    unlocked_command(&mut flash, 0xa0);
    flash.write(PROGRAM_ADDRESS, 0xff).unwrap();

    assert_eq!(flash.state(), Amd29F040BState::ByteProgramError);
    assert_eq!(flash.read(PROGRAM_ADDRESS).unwrap() & 0x20, 0x20);
    flash.clock_cycles(AMD_29F040B_BYTE_PROGRAM_CYCLES);
    assert_eq!(flash.peek(PROGRAM_ADDRESS).unwrap(), 0x00);
    flash.write(PROGRAM_ADDRESS, 0xf0).unwrap();
    assert_eq!(flash.state(), Amd29F040BState::Read);
}

#[test]
fn sector_window_suspend_resume_and_chip_erase_use_physical_cycle_deadlines() {
    let mut flash = create_flash(0x00);
    let sector_one = AMD_29F040B_SECTOR_SIZE_BYTES + 0x123;
    let sector_three = AMD_29F040B_SECTOR_SIZE_BYTES * 3 + 0x456;
    begin_erase(&mut flash);
    flash.write(sector_one, 0x30).unwrap();
    flash.write(sector_three, 0x30).unwrap();
    assert_eq!(flash.state(), Amd29F040BState::SectorEraseWindow);
    assert_eq!(flash.read(sector_one).unwrap() & 0x08, 0);
    flash.clock_cycles(AMD_29F040B_SECTOR_ERASE_WINDOW_CYCLES);
    assert_eq!(flash.state(), Amd29F040BState::SectorEraseBusy);
    assert_eq!(flash.read(sector_one).unwrap() & 0x08, 0x08);

    flash.clock_cycles(AMD_29F040B_SECTOR_ERASE_CYCLES);
    assert_eq!(flash.peek(sector_one).unwrap(), 0xff);
    assert_eq!(flash.peek(sector_three).unwrap(), 0x00);
    flash.write(sector_three, 0xb0).unwrap();
    flash.clock_cycles(AMD_29F040B_SECTOR_ERASE_CYCLES);
    assert_eq!(flash.peek(sector_three).unwrap(), 0x00);
    flash.write(sector_three, 0x30).unwrap();
    flash.clock_cycles(AMD_29F040B_SECTOR_ERASE_CYCLES);
    assert_eq!(flash.peek(sector_three).unwrap(), 0xff);

    let mut chip = create_flash(0x00);
    begin_erase(&mut chip);
    chip.write(AMD_29F040B_UNLOCK_ADDRESS_1, 0x10).unwrap();
    chip.clock_cycles(AMD_29F040B_CHIP_ERASE_CYCLES - 1);
    assert_eq!(chip.peek(0x70000).unwrap(), 0x00);
    chip.clock_cycles(1);
    assert_eq!(chip.peek(0x70000).unwrap(), 0xff);
    chip.reset_command_state();
    assert_eq!(chip.peek(0x70000).unwrap(), 0xff);
}
