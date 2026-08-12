// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 disk-side VIA integration tests
//
//   File:       tests/drive1541_disk_via.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::drive1541::disk_via::{
    DISK_LED, DISK_MOTOR, Drive1541DiskVia, Drive1541DiskViaError,
};
use c64_core::devices::drive1541::mechanism::{
    DRIVE_1541_DISK_INSERTION_CYCLES, Drive1541DiskImage, Drive1541Mechanism,
};
use c64_core::devices::via::{interrupt, peripheral_control_mode, register};
use c64_core::media::d64::{D64_SECTOR_SIZE, D64DiskImage, d64_sector_count_through_track};

fn d64_disk(write_protected: bool) -> D64DiskImage {
    let sector_count = d64_sector_count_through_track(35).unwrap();
    let mut bytes = vec![0; sector_count * D64_SECTOR_SIZE];
    let directory_offset = d64_sector_count_through_track(17).unwrap() * D64_SECTOR_SIZE;
    bytes[directory_offset + 0xa2] = 0x4a;
    bytes[directory_offset + 0xa3] = 0x53;
    D64DiskImage::parse(&bytes, write_protected).unwrap()
}

fn create_disk_via() -> (Drive1541DiskVia, Drive1541Mechanism) {
    let mut mechanism = Drive1541Mechanism::new();
    let via = Drive1541DiskVia::new(8, &mut mechanism).unwrap();
    (via, mechanism)
}

#[test]
fn validates_device_number_and_maps_port_b_controls() {
    let mut mechanism = Drive1541Mechanism::new();
    assert_eq!(
        Drive1541DiskVia::new(7, &mut mechanism),
        Err(Drive1541DiskViaError::InvalidDeviceNumber(7))
    );

    let (mut via, mut mechanism) = create_disk_via();
    via.write(&mut mechanism, u16::from(register::DATA_DIRECTION_B), 0x6f)
        .unwrap();
    via.write(
        &mut mechanism,
        u16::from(register::PORT_B),
        3 | DISK_MOTOR | DISK_LED | (2 << 5),
    )
    .unwrap();

    assert_eq!(mechanism.current_half_track(), 37);
    assert!(mechanism.motor_on());
    assert!(mechanism.led_on());
    assert_eq!(mechanism.selected_speed_zone().get(), 2);
}

#[test]
fn reports_active_low_sync_and_write_protect_sensor_inputs() {
    let (mut via, mut mechanism) = create_disk_via();
    assert_eq!(
        via.read(&mut mechanism, u16::from(register::PORT_B))
            .unwrap(),
        0xff
    );

    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    assert_eq!(
        via.read(&mut mechanism, u16::from(register::PORT_B))
            .unwrap(),
        0xef
    );
    mechanism.tick(DRIVE_1541_DISK_INSERTION_CYCLES).unwrap();
    assert_eq!(
        via.read(&mut mechanism, u16::from(register::PORT_B))
            .unwrap(),
        0xff
    );
    mechanism.set_speed_zone(c64_core::devices::drive1541::gcr::Drive1541SpeedZone::Zone2);
    mechanism.set_motor_on(true);
    mechanism.tick(34).unwrap();
    assert!(mechanism.sync_found());
    assert_eq!(
        via.read(&mut mechanism, u16::from(register::PORT_B))
            .unwrap(),
        0x7f
    );
}

#[test]
fn preserves_write_protect_after_insertion_finishes() {
    let (mut via, mut mechanism) = create_disk_via();
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(true)))
        .unwrap();
    mechanism.tick(DRIVE_1541_DISK_INSERTION_CYCLES).unwrap();

    assert_eq!(
        via.read(&mut mechanism, u16::from(register::PORT_B))
            .unwrap(),
        0xef
    );
    assert!(mechanism.write_protected());
}

#[test]
fn latches_ca1_and_acknowledges_byte_ready_on_both_ports() {
    let (mut via, mut mechanism) = create_disk_via();
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    mechanism.tick(DRIVE_1541_DISK_INSERTION_CYCLES).unwrap();
    mechanism.set_speed_zone(c64_core::devices::drive1541::gcr::Drive1541SpeedZone::Zone2);
    mechanism.set_motor_on(true);
    via.write(
        &mut mechanism,
        u16::from(register::INTERRUPT_ENABLE),
        interrupt::ANY | interrupt::CA1,
    )
    .unwrap();
    mechanism.tick(172).unwrap();

    assert!(mechanism.byte_ready_asserted());
    assert_eq!(
        via.read(&mut mechanism, u16::from(register::PORT_A))
            .unwrap(),
        mechanism.data_byte()
    );
    assert!(!mechanism.byte_ready_asserted());
    assert_eq!(via.via().interrupt_flags() & interrupt::CA1, 0);

    mechanism.set_read_mode(false);
    mechanism.set_write_data_byte(0xff);
    mechanism.tick(32).unwrap();
    assert!(mechanism.byte_ready_asserted());
    via.write(&mut mechanism, u16::from(register::PORT_B), 0)
        .unwrap();
    assert!(!mechanism.byte_ready_asserted());
}

#[test]
fn routes_ca2_cb2_and_effective_port_a_output_pins() {
    let (mut via, mut mechanism) = create_disk_via();
    via.write(
        &mut mechanism,
        u16::from(register::PERIPHERAL_CONTROL),
        (peripheral_control_mode::LOW_OUTPUT << 1) | (peripheral_control_mode::HIGH_OUTPUT << 5),
    )
    .unwrap();
    assert!(!mechanism.byte_ready_enabled());
    assert!(mechanism.reading());

    via.write(
        &mut mechanism,
        u16::from(register::PERIPHERAL_CONTROL),
        (peripheral_control_mode::HIGH_OUTPUT << 1) | (peripheral_control_mode::LOW_OUTPUT << 5),
    )
    .unwrap();
    assert!(mechanism.byte_ready_enabled());
    assert!(!mechanism.reading());

    via.write(&mut mechanism, u16::from(register::DATA_DIRECTION_A), 0xf0)
        .unwrap();
    via.write(&mut mechanism, u16::from(register::PORT_A), 0xa0)
        .unwrap();
    assert_eq!(mechanism.write_data_byte(), 0xaf);

    via.write(
        &mut mechanism,
        u16::from(register::PERIPHERAL_CONTROL),
        (peripheral_control_mode::PULSE_OUTPUT << 1) | (peripheral_control_mode::PULSE_OUTPUT << 5),
    )
    .unwrap();
    via.read(&mut mechanism, u16::from(register::PORT_A))
        .unwrap();
    via.read(&mut mechanism, u16::from(register::PORT_B))
        .unwrap();
    assert!(!mechanism.byte_ready_enabled());
    assert!(!mechanism.reading());
    via.clock_cycle(&mut mechanism).unwrap();
    assert!(mechanism.byte_ready_enabled());
    assert!(mechanism.reading());
}

#[test]
fn clocks_mechanism_before_sampling_ca1_and_preserves_head_on_reset() {
    let (mut via, mut mechanism) = create_disk_via();
    assert_eq!(mechanism.write_data_byte(), 0);
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    mechanism.tick(DRIVE_1541_DISK_INSERTION_CYCLES).unwrap();
    mechanism.set_speed_zone(c64_core::devices::drive1541::gcr::Drive1541SpeedZone::Zone0);
    mechanism.set_write_data_byte(0xff);
    mechanism.set_read_mode(false);
    mechanism.set_motor_on(true);
    via.write(
        &mut mechanism,
        u16::from(register::INTERRUPT_ENABLE),
        interrupt::ANY | interrupt::CA1,
    )
    .unwrap();
    for _ in 0..32 {
        via.clock_cycle(&mut mechanism).unwrap();
    }
    assert!(via.interrupt_pending());

    via.write(&mut mechanism, u16::from(register::DATA_DIRECTION_B), 0x6f)
        .unwrap();
    via.write(
        &mut mechanism,
        u16::from(register::PORT_B),
        3 | DISK_MOTOR | (3 << 5),
    )
    .unwrap();
    assert_eq!(mechanism.current_half_track(), 37);
    via.reset(&mut mechanism);
    assert_eq!(mechanism.current_half_track(), 37);
    assert!(!mechanism.motor_on());
    assert!(mechanism.led_on());
    assert_eq!(mechanism.selected_speed_zone().get(), 0);
    assert!(mechanism.reading());
    assert!(mechanism.byte_ready_enabled());
    assert_eq!(mechanism.write_data_byte(), 0xff);
}
