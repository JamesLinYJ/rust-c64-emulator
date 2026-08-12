// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 integer clock synchronizer tests
//
//   File:       tests/drive1541_clock.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::architecture::VideoStandard;
use c64_core::devices::drive1541::clock::{
    DRIVE_1541_PROCESSOR_CLOCK_HZ, Drive1541ClockError, Drive1541ClockSynchronizer,
};
use c64_core::devices::drive1541::disk_via::Drive1541DiskVia;
use c64_core::devices::drive1541::iec_via::{Drive1541IecVia, IEC_DATA_INPUT};
use c64_core::devices::drive1541::machine::Drive1541Machine;
use c64_core::devices::drive1541::mechanism::Drive1541Mechanism;
use c64_core::devices::drive1541::memory::{DRIVE_1541_ROM_SIZE, Drive1541Memory};
use c64_core::devices::iec::{IecBus, IecLine, IecPort};

fn create_machine(program: &[u8]) -> (IecBus, IecPort, Drive1541Machine) {
    let mut rom = [0xea; DRIVE_1541_ROM_SIZE];
    rom[..program.len()].copy_from_slice(program);
    rom[0x3ffc] = 0x00;
    rom[0x3ffd] = 0xc0;
    rom[0x3ffe] = 0x00;
    rom[0x3fff] = 0xc0;
    let mut bus = IecBus::new();
    let host_port = bus.attach().unwrap();
    let mut mechanism = Drive1541Mechanism::new();
    let iec_via = Drive1541IecVia::new(8, &mut bus).unwrap();
    let disk_via = Drive1541DiskVia::new(8, &mut mechanism).unwrap();
    let memory = Drive1541Memory::new(&rom, iec_via, disk_via).unwrap();
    let machine = Drive1541Machine::new(memory, mechanism);
    (bus, host_port, machine)
}

#[test]
fn converts_one_pal_second_without_fractional_drift() {
    let (mut bus, _, mut machine) = create_machine(&[0xea]);
    let mut clock = Drive1541ClockSynchronizer::for_video_standard(VideoStandard::Pal);

    let generated = clock
        .advance_host_cycles(
            &mut machine,
            &mut bus,
            u64::from(VideoStandard::PAL_SYSTEM_CLOCK_HZ),
        )
        .unwrap();

    assert_eq!(generated, DRIVE_1541_PROCESSOR_CLOCK_HZ);
    assert_eq!(clock.target_cycles(), DRIVE_1541_PROCESSOR_CLOCK_HZ);
    assert_eq!(machine.elapsed_cycles(), DRIVE_1541_PROCESSOR_CLOCK_HZ);
    assert_eq!(clock.phase_remainder(), 0);
    assert_eq!(clock.lead_cycles(&machine), 0);
}

#[test]
fn retains_fractional_host_cycles_without_running_ahead() {
    let (mut bus, _, mut machine) = create_machine(&[0xea]);
    let mut clock = Drive1541ClockSynchronizer::try_new(3_000_000).unwrap();

    assert_eq!(
        clock
            .advance_host_cycles(&mut machine, &mut bus, 2)
            .unwrap(),
        0
    );
    assert_eq!(clock.target_cycles(), 0);
    assert_eq!(machine.elapsed_cycles(), 0);
    assert_eq!(clock.phase_remainder(), 2_000_000);

    assert_eq!(clock.advance_host_cycle(&mut machine, &mut bus).unwrap(), 1);
    assert_eq!(clock.target_cycles(), 1);
    assert_eq!(machine.elapsed_cycles(), 1);

    assert_eq!(
        clock
            .advance_host_cycles(&mut machine, &mut bus, 6)
            .unwrap(),
        2
    );
    assert_eq!(clock.target_cycles(), 3);
    assert_eq!(machine.elapsed_cycles(), 3);
    assert_eq!(clock.phase_remainder(), 0);
}

#[test]
fn single_cycle_and_batch_paths_are_exactly_equivalent() {
    let (mut single_bus, _, mut single_machine) = create_machine(&[0xea]);
    let (mut batch_bus, _, mut batch_machine) = create_machine(&[0xea]);
    let mut single_clock = Drive1541ClockSynchronizer::for_video_standard(VideoStandard::Pal);
    let mut batch_clock = Drive1541ClockSynchronizer::for_video_standard(VideoStandard::Pal);
    let host_cycles = 100_000;

    for _ in 0..host_cycles {
        single_clock
            .advance_host_cycle(&mut single_machine, &mut single_bus)
            .unwrap();
    }
    batch_clock
        .advance_host_cycles(&mut batch_machine, &mut batch_bus, host_cycles)
        .unwrap();

    assert_eq!(single_clock, batch_clock);
    assert_eq!(single_machine, batch_machine);
    assert_eq!(single_bus, batch_bus);
}

#[test]
fn resets_rational_phase_and_machine_timing_together() {
    let (mut bus, _, mut machine) = create_machine(&[0xea]);
    let mut clock = Drive1541ClockSynchronizer::for_video_standard(VideoStandard::Pal);
    clock
        .advance_host_cycles(&mut machine, &mut bus, 20)
        .unwrap();
    assert_ne!(machine.elapsed_cycles(), 0);

    clock.reset_clock(&mut machine);

    assert_eq!(clock.target_cycles(), 0);
    assert_eq!(clock.phase_remainder(), 0);
    assert_eq!(clock.lead_cycles(&machine), 0);
    assert_eq!(machine.elapsed_cycles(), 0);
}

#[test]
fn samples_an_iec_transition_on_the_exact_drive_read_cycle() {
    let (mut bus, host_port, mut machine) = create_machine(&[0xad, 0x00, 0x18]);
    let mut clock = Drive1541ClockSynchronizer::try_new(DRIVE_1541_PROCESSOR_CLOCK_HZ).unwrap();

    clock
        .advance_host_cycles(&mut machine, &mut bus, 3)
        .unwrap();
    bus.set_port_low_mask(host_port, IecLine::Data.mask())
        .unwrap();
    clock.advance_host_cycle(&mut machine, &mut bus).unwrap();

    assert_eq!(machine.elapsed_cycles(), 4);
    assert_eq!(
        machine.cpu().state().accumulator & IEC_DATA_INPUT,
        IEC_DATA_INPUT
    );
}

#[test]
fn rejects_invalid_rates_overflow_and_an_unmatched_drive_lead() {
    assert_eq!(
        Drive1541ClockSynchronizer::try_new(0),
        Err(Drive1541ClockError::ZeroHostClock)
    );

    let (mut bus, _, mut machine) = create_machine(&[0xea]);
    let mut clock = Drive1541ClockSynchronizer::try_new(3_000_000).unwrap();
    assert_eq!(
        clock.advance_host_cycles(&mut machine, &mut bus, u64::MAX),
        Err(Drive1541ClockError::PhaseOverflow)
    );

    machine.advance_hardware(&mut bus, 2).unwrap();
    assert_eq!(
        clock.advance_host_cycle(&mut machine, &mut bus),
        Err(Drive1541ClockError::DriveAheadOfTarget {
            elapsed_cycles: 2,
            target_cycles: 0,
        })
    );
}
