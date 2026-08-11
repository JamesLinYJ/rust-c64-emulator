// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 independent 6502 scheduler tests
//
//   File:       tests/drive1541_machine.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::cpu::{CPU_STATUS_INTERRUPT_DISABLE, CPU_STATUS_OVERFLOW};
use c64_core::devices::drive1541::disk_via::Drive1541DiskVia;
use c64_core::devices::drive1541::gcr::Drive1541SpeedZone;
use c64_core::devices::drive1541::iec_via::Drive1541IecVia;
use c64_core::devices::drive1541::machine::Drive1541Machine;
use c64_core::devices::drive1541::mechanism::{
    DRIVE_1541_DISK_INSERTION_CYCLES, Drive1541DiskImage, Drive1541Mechanism,
};
use c64_core::devices::drive1541::memory::{DRIVE_1541_ROM_SIZE, Drive1541Memory};
use c64_core::devices::iec::IecBus;
use c64_core::devices::via::{interrupt, register};
use c64_core::media::d64::{D64_SECTOR_SIZE, D64DiskImage, d64_sector_count_through_track};

fn d64_disk() -> D64DiskImage {
    let sector_count = d64_sector_count_through_track(35).unwrap();
    let mut bytes = vec![0; sector_count * D64_SECTOR_SIZE];
    let directory_offset = d64_sector_count_through_track(17).unwrap() * D64_SECTOR_SIZE;
    bytes[directory_offset + 0xa2] = 0x4a;
    bytes[directory_offset + 0xa3] = 0x53;
    D64DiskImage::parse(&bytes, false).unwrap()
}

fn create_machine(program: &[u8]) -> (IecBus, Drive1541Machine) {
    let mut rom = [0; DRIVE_1541_ROM_SIZE];
    rom[..program.len()].copy_from_slice(program);
    rom[0x3ffc] = 0x00;
    rom[0x3ffd] = 0xc0;
    rom[0x3ffe] = 0x00;
    rom[0x3fff] = 0xc1;
    let mut bus = IecBus::new();
    let mut mechanism = Drive1541Mechanism::new();
    let iec_via = Drive1541IecVia::new(8, &mut bus).unwrap();
    let disk_via = Drive1541DiskVia::new(8, &mut mechanism).unwrap();
    let memory = Drive1541Memory::new(&rom, iec_via, disk_via).unwrap();
    let machine = Drive1541Machine::new(memory, mechanism, &mut bus).unwrap();
    (bus, machine)
}

fn raise_disk_byte_ready_interrupt(bus: &mut IecBus, machine: &mut Drive1541Machine) {
    machine
        .write_memory(
            bus,
            0x1c00 + u16::from(register::INTERRUPT_ENABLE),
            interrupt::ANY | interrupt::CA1,
        )
        .unwrap();
    machine
        .mechanism_mut()
        .mount_disk(Drive1541DiskImage::D64(d64_disk()))
        .unwrap();
    machine
        .mechanism_mut()
        .tick(DRIVE_1541_DISK_INSERTION_CYCLES)
        .unwrap();
    machine
        .mechanism_mut()
        .set_speed_zone(Drive1541SpeedZone::Zone2);
    machine.mechanism_mut().set_motor_on(true);
    machine.advance_hardware(bus, 172).unwrap();
    assert!(machine.disk_via_interrupt_pending());
}

#[test]
fn begins_at_the_reset_vector_and_advances_one_bus_cycle_at_a_time() {
    let (mut bus, mut machine) = create_machine(&[0xea]);
    assert_eq!(machine.cpu().state().program_counter, 0xc000);
    assert_eq!(machine.cpu().state().stack_pointer, 0xfd);
    assert_eq!(machine.elapsed_cycles(), 0);
    assert_eq!(machine.execute_instruction(&mut bus).unwrap(), 2);
    assert_eq!(machine.elapsed_cycles(), 2);
    assert_eq!(machine.cpu().state().program_counter, 0xc001);
}

#[test]
fn batches_without_crossing_an_unrequested_cpu_bus_boundary() {
    let program = [0xa9, 0x42, 0x8d, 0x00, 0x00, 0xea];
    let (mut batched_bus, mut batched) = create_machine(&program);
    let (mut stepped_bus, mut stepped) = create_machine(&program);

    assert_eq!(batched.clock_cycles(&mut batched_bus, 5).unwrap(), 5);
    for _ in 0..5 {
        stepped.clock_cycle(&mut stepped_bus).unwrap();
    }
    assert_eq!(batched.cpu().state(), stepped.cpu().state());
    assert_eq!(batched.memory().ram(), stepped.memory().ram());
    assert_eq!(batched.memory().ram()[0], 0);

    batched.clock_cycle(&mut batched_bus).unwrap();
    stepped.clock_cycle(&mut stepped_bus).unwrap();
    assert_eq!(batched.memory().ram()[0], 0x42);
    assert_eq!(batched.cpu().state(), stepped.cpu().state());
}

#[test]
fn treats_zero_page_zero_and_one_as_plain_6502_ram() {
    let program = [0xa9, 0x2f, 0x85, 0x00, 0xa9, 0x37, 0x85, 0x01];
    let (mut bus, mut machine) = create_machine(&program);
    for _ in 0..4 {
        machine.execute_instruction(&mut bus).unwrap();
    }
    assert_eq!(&machine.memory().ram()[..2], &[0x2f, 0x37]);
}

#[test]
fn routes_via_irq_at_instruction_boundaries() {
    let (mut bus, mut machine) = create_machine(&[0x58, 0xea, 0xea]);
    raise_disk_byte_ready_interrupt(&mut bus, &mut machine);
    assert_ne!(
        machine.cpu().state().status & CPU_STATUS_INTERRUPT_DISABLE,
        0
    );

    assert_eq!(machine.execute_instruction(&mut bus).unwrap(), 2); // CLI
    assert_eq!(machine.execute_instruction(&mut bus).unwrap(), 2); // delayed NOP
    machine.clock_cycles(&mut bus, 7).unwrap();
    assert_eq!(machine.cpu().state().program_counter, 0xc100);
    assert!(machine.cpu().is_at_instruction_boundary());
}

#[test]
fn delivers_every_byte_ready_edge_to_so_even_while_ca1_stays_asserted() {
    let (mut bus, mut machine) = create_machine(&[0xb8]); // CLV
    machine
        .mechanism_mut()
        .mount_disk(Drive1541DiskImage::D64(d64_disk()))
        .unwrap();
    machine
        .mechanism_mut()
        .tick(DRIVE_1541_DISK_INSERTION_CYCLES)
        .unwrap();
    machine
        .mechanism_mut()
        .set_speed_zone(Drive1541SpeedZone::Zone0);
    machine.mechanism_mut().set_write_data_byte(0xff);
    machine.mechanism_mut().set_read_mode(false);
    machine.mechanism_mut().set_motor_on(true);

    machine.advance_hardware(&mut bus, 32).unwrap();
    assert!(machine.mechanism().byte_ready_asserted());
    assert_ne!(machine.cpu().state().status & CPU_STATUS_OVERFLOW, 0);
    assert_eq!(machine.execute_instruction(&mut bus).unwrap(), 2);
    assert_eq!(machine.cpu().state().status & CPU_STATUS_OVERFLOW, 0);
    assert!(machine.mechanism().byte_ready_asserted());
    machine.advance_hardware(&mut bus, 30).unwrap();
    assert_ne!(machine.cpu().state().status & CPU_STATUS_OVERFLOW, 0);
}

#[test]
fn advances_all_hardware_during_each_reset_bus_cycle() {
    let (mut bus, mut machine) = create_machine(&[0xea]);
    machine.mechanism_mut().set_motor_on(true);
    assert_eq!(machine.reset_cpu(&mut bus).unwrap(), 7);
    assert_eq!(machine.elapsed_cycles(), 7);
    assert_eq!(machine.cpu().state().program_counter, 0xc000);
    machine.reset_timing();
    assert_eq!(machine.elapsed_cycles(), 0);
}
