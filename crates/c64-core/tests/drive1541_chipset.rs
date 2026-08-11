// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 chipset integration tests
//
//   File:       tests/drive1541_chipset.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::IecLine;
use c64_core::devices::drive1541::gcr::Drive1541SpeedZone;
use c64_core::devices::drive1541::iec_via::{IEC_CLOCK_OUTPUT, IEC_DATA_INPUT, IEC_DATA_OUTPUT};
use c64_core::devices::drive1541::mechanism::{
    Drive1541ControlState, Drive1541DiskImage, Drive1541StepperPhase,
};
use c64_core::devices::drive1541::memory::DRIVE_1541_ROM_SIZE;
use c64_core::devices::vic::VicMemoryBus;
use c64_core::media::d64::{D64_SECTOR_SIZE, D64DiskImage, d64_sector_count_through_track};
use c64_core::{C64BusDevices, C64Chipset, C64Core, CoreConfig, MachineProfile, VideoStandard};

const C64_IEC_DATA_OUTPUT: u8 = 1 << 5;

struct ZeroVicMemory;

impl VicMemoryBus for ZeroVicMemory {
    fn cpu_data_bus_value(&self) -> u8 {
        u8::MAX
    }

    fn read_vic_byte(&mut self, _address_in_bank: u16) -> u8 {
        0
    }

    fn read_vic_color(&mut self, _index: u16) -> u8 {
        0
    }
}

fn drive_rom(program: &[u8]) -> [u8; DRIVE_1541_ROM_SIZE] {
    let mut rom = [0xea; DRIVE_1541_ROM_SIZE];
    rom[..program.len()].copy_from_slice(program);
    rom[0x3ffc] = 0x00;
    rom[0x3ffd] = 0xc0;
    rom[0x3ffe] = 0x00;
    rom[0x3fff] = 0xc0;
    rom
}

fn blank_d64() -> D64DiskImage {
    let byte_length = d64_sector_count_through_track(35).unwrap() * D64_SECTOR_SIZE;
    D64DiskImage::parse(&vec![0; byte_length], false).unwrap()
}

#[test]
fn attachment_is_explicit_atomic_and_releases_its_iec_port() {
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    let rom = drive_rom(&[0xea]);

    assert!(chipset.attach_drive1541(8, &rom[..1]).is_err());
    assert!(chipset.drive1541().is_none());
    chipset.attach_drive1541(8, &rom).unwrap();
    assert_eq!(chipset.drive1541().unwrap().device_number(), 8);
    assert!(chipset.attach_drive1541(9, &rom).is_err());

    chipset.detach_drive1541().unwrap();
    assert!(chipset.drive1541().is_none());
    chipset.attach_drive1541(9, &rom).unwrap();
    assert_eq!(chipset.drive1541().unwrap().device_number(), 9);
}

#[test]
fn chipset_clocks_the_drive_in_the_same_integer_host_domain() {
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    let mut vic_memory = ZeroVicMemory;
    chipset.attach_drive1541(8, &drive_rom(&[0xea])).unwrap();

    for _ in 0..100 {
        chipset.clock_system_cycle(&mut vic_memory).unwrap();
    }

    let drive = chipset.drive1541().unwrap();
    assert_eq!(drive.clock().target_cycles(), 101);
    assert_eq!(drive.machine().elapsed_cycles(), 101);
    assert!(drive.machine().cpu().state().program_counter > 0xc000);
}

#[test]
fn c64_output_is_sampled_on_the_exact_drive_read_cycle() {
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    let mut vic_memory = ZeroVicMemory;
    chipset
        .attach_drive1541(8, &drive_rom(&[0xad, 0x00, 0x18]))
        .unwrap();

    for _ in 0..3 {
        chipset.clock_system_cycle(&mut vic_memory).unwrap();
    }
    chipset.write_io(0xdd02, C64_IEC_DATA_OUTPUT);
    chipset.write_io(0xdd00, C64_IEC_DATA_OUTPUT);
    chipset.clock_system_cycle(&mut vic_memory).unwrap();

    assert_eq!(chipset.drive1541().unwrap().machine().elapsed_cycles(), 4);
    assert_eq!(
        chipset
            .drive1541()
            .unwrap()
            .machine()
            .cpu()
            .state()
            .accumulator
            & IEC_DATA_INPUT,
        IEC_DATA_INPUT
    );
}

#[test]
fn drive_output_reaches_cia2_on_the_exact_via_write_cycle() {
    let outputs = IEC_CLOCK_OUTPUT | IEC_DATA_OUTPUT;
    let program = [
        0xa9, outputs, // LDA #outputs
        0x8d, 0x02, 0x18, // STA $1802 (DDRB)
        0x8d, 0x00, 0x18, // STA $1800 (ORB)
    ];
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    let mut vic_memory = ZeroVicMemory;
    chipset.attach_drive1541(8, &drive_rom(&program)).unwrap();

    for _ in 0..9 {
        chipset.clock_system_cycle(&mut vic_memory).unwrap();
    }
    assert!(chipset.iec_bus().state().clock_high());
    assert!(chipset.iec_bus().state().data_high());

    chipset.clock_system_cycle(&mut vic_memory).unwrap();
    assert!(!chipset.iec_bus().state().clock_high());
    assert!(!chipset.iec_bus().state().data_high());
    assert_eq!(chipset.read_io(0xdd00, u8::MAX) & 0xc0, 0);
}

#[test]
fn c64_hardware_reset_pulses_iec_and_preserves_drive_physical_state() {
    let program = [0xa9, 0x5a, 0x85, 0x20, 0x4c, 0x04, 0xc0];
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    let mut vic_memory = ZeroVicMemory;
    chipset.attach_drive1541(8, &drive_rom(&program)).unwrap();
    chipset
        .drive1541_mut()
        .unwrap()
        .machine_mut()
        .mechanism_mut()
        .mount_disk(Drive1541DiskImage::D64(blank_d64()))
        .unwrap();
    chipset
        .drive1541_mut()
        .unwrap()
        .machine_mut()
        .mechanism_mut()
        .apply_control_state(Drive1541ControlState {
            led_on: false,
            motor_on: true,
            speed_zone: Drive1541SpeedZone::Zone3,
            stepper_phase: Drive1541StepperPhase::Phase3,
        })
        .unwrap();
    for _ in 0..20 {
        chipset.clock_system_cycle(&mut vic_memory).unwrap();
    }
    let reset_sequence_before = chipset.iec_bus().reset_assertion_sequence();
    assert_eq!(
        chipset.drive1541().unwrap().machine().memory().ram()[0x20],
        0x5a
    );
    assert_eq!(
        chipset
            .drive1541()
            .unwrap()
            .machine()
            .mechanism()
            .current_half_track(),
        37
    );

    chipset.reset().unwrap();

    let drive = chipset.drive1541().unwrap();
    assert_eq!(
        chipset.iec_bus().reset_assertion_sequence(),
        reset_sequence_before + 1
    );
    assert!(chipset.iec_bus().state().reset_high());
    assert_eq!(drive.machine().elapsed_cycles(), 0);
    assert_eq!(drive.machine().cpu().state().program_counter, 0xc000);
    assert_eq!(drive.machine().memory().ram()[0x20], 0x5a);
    assert!(drive.machine().mechanism().disk_present());
    assert_eq!(drive.machine().mechanism().current_half_track(), 37);
    assert!(!drive.machine().mechanism().motor_on());
}

#[test]
fn a_complete_external_reset_pulse_cannot_be_lost_between_drive_clocks() {
    let program = [0xa9, 0x5a, 0x85, 0x20, 0x4c, 0x04, 0xc0];
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    let mut vic_memory = ZeroVicMemory;
    chipset.attach_drive1541(8, &drive_rom(&program)).unwrap();
    for _ in 0..20 {
        chipset.clock_system_cycle(&mut vic_memory).unwrap();
    }
    let elapsed_before = chipset.drive1541().unwrap().machine().elapsed_cycles();
    let external = chipset.iec_bus_mut().attach().unwrap();
    chipset
        .iec_bus_mut()
        .set_port_line(external, IecLine::Reset, true)
        .unwrap();
    chipset
        .iec_bus_mut()
        .set_port_line(external, IecLine::Reset, false)
        .unwrap();
    assert_eq!(
        chipset.drive1541().unwrap().machine().elapsed_cycles(),
        elapsed_before
    );

    chipset.clock_system_cycle(&mut vic_memory).unwrap();

    let drive = chipset.drive1541().unwrap();
    assert_eq!(drive.clock().target_cycles(), 1);
    assert_eq!(drive.machine().elapsed_cycles(), 1);
    assert_eq!(drive.machine().memory().ram()[0x20], 0x5a);
}

#[test]
fn c64_core_scheduler_advances_the_attached_drive_without_a_fallback() {
    let mut core = C64Core::new(CoreConfig::default());
    core.attach_drive1541(8, &drive_rom(&[0xea])).unwrap();

    core.run_cpu_slots(100).unwrap();

    let host_cycles = core.diagnostics().elapsed_system_cycles;
    let expected_drive_cycles =
        host_cycles * 1_000_000 / u64::from(VideoStandard::PAL_SYSTEM_CLOCK_HZ);
    let drive = core.devices().drive1541().unwrap();
    assert_eq!(drive.clock().target_cycles(), expected_drive_cycles);
    assert_eq!(drive.machine().elapsed_cycles(), expected_drive_cycles);
}

#[test]
fn full_state_restores_drive_hardware_after_core_reinitialization() {
    let mut core = C64Core::new(CoreConfig::default());
    core.attach_drive1541(9, &drive_rom(&[0xea])).unwrap();
    core.run_cpu_slots(20).unwrap();
    let expected_drive = core.devices().drive1541().unwrap().clone();
    let state = core.save_state();

    core.power_cycle_with_profile(MachineProfile::VmEnhanced)
        .unwrap();
    let drive = core.devices().drive1541().unwrap();
    assert_eq!(drive.device_number(), 9);
    assert_eq!(drive.machine().elapsed_cycles(), 0);

    core.run_cpu_slots(8).unwrap();
    core.load_state(&state).unwrap();
    let drive = core.devices().drive1541().unwrap();
    assert_eq!(drive, &expected_drive);
}
