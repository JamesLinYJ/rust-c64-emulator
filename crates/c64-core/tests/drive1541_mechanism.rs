// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 mechanism integration tests
//
//   File:       tests/drive1541_mechanism.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::collections::BTreeSet;

use c64_core::devices::drive1541::d64_gcr::D64_GCR_RAW_TRACK_SIZE;
use c64_core::devices::drive1541::gcr::Drive1541SpeedZone;
use c64_core::devices::drive1541::mechanism::{
    DRIVE_1541_DISK_INSERTION_CYCLES, DRIVE_1541_DISK_REMOVAL_CYCLES,
    DRIVE_1541_DISK_REPLACEMENT_GAP_CYCLES, DRIVE_1541_INITIAL_HALF_TRACK, Drive1541ControlState,
    Drive1541DiskImage, Drive1541Mechanism, Drive1541MechanismError, Drive1541StepperPhase,
};
use c64_core::media::d64::{D64_SECTOR_SIZE, D64DiskImage, d64_sector_count_through_track};
use c64_core::media::g64::{G64_MAXIMUM_HALF_TRACK_COUNT, G64DiskImage, G64SpeedMap, G64SpeedZone};

fn d64_disk(write_protected: bool) -> D64DiskImage {
    let sector_count = d64_sector_count_through_track(35).unwrap();
    let mut bytes = vec![0; sector_count * D64_SECTOR_SIZE];
    let directory_offset = d64_sector_count_through_track(17).unwrap() * D64_SECTOR_SIZE;
    bytes[directory_offset + 0xa2] = 0x4a;
    bytes[directory_offset + 0xa3] = 0x53;
    D64DiskImage::parse(&bytes, write_protected).unwrap()
}

fn g64_disk() -> G64DiskImage {
    let count = G64_MAXIMUM_HALF_TRACK_COUNT;
    let maximum_track_length = 16_u16;
    let mut bytes = vec![0; 12 + usize::from(count) * 8];
    bytes[..8].copy_from_slice(b"GCR-1541");
    bytes[8] = 0;
    bytes[9] = count;
    bytes[10..12].copy_from_slice(&maximum_track_length.to_le_bytes());
    let mut image = G64DiskImage::parse(&bytes, false).unwrap();
    image
        .set_half_track(
            36,
            &[0xff; 16],
            Some(G64SpeedMap::Constant(G64SpeedZone::Zone0)),
        )
        .unwrap();
    image
}

fn finish_insertion(mechanism: &mut Drive1541Mechanism) {
    mechanism.tick(DRIVE_1541_DISK_INSERTION_CYCLES).unwrap();
}

#[test]
fn starts_on_track_18_and_uses_integer_spindle_rotation() {
    let mut mechanism = Drive1541Mechanism::new();
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    mechanism.tick(10_000).unwrap();
    assert_eq!(
        mechanism.current_half_track(),
        DRIVE_1541_INITIAL_HALF_TRACK
    );
    assert_eq!(mechanism.current_whole_track(), 18);
    assert_eq!(mechanism.angular_bit_offset(), 0);
    assert!(!mechanism.motor_on());

    finish_insertion(&mut mechanism);
    mechanism.set_motor_on(true);
    mechanism.set_speed_zone(Drive1541SpeedZone::Zone0);
    mechanism.tick(40).unwrap();
    assert_eq!(mechanism.angular_bit_offset(), 11);
    mechanism.set_speed_zone(Drive1541SpeedZone::Zone3);
    mechanism.tick(13).unwrap();
    assert_eq!(mechanism.angular_bit_offset(), 15);
}

#[test]
fn detects_sync_and_latches_the_following_gcr_byte() {
    let mut mechanism = Drive1541Mechanism::new();
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    finish_insertion(&mut mechanism);
    mechanism.set_speed_zone(Drive1541SpeedZone::Zone2);
    mechanism.set_motor_on(true);

    mechanism.tick(34).unwrap();
    assert!(mechanism.sync_found());
    assert!(!mechanism.byte_ready_asserted());
    let transition_sequence = mechanism.byte_ready_transition_sequence();

    mechanism.tick(138).unwrap();
    let raw_track = mechanism.read_raw_half_track(36).unwrap();
    assert_eq!(mechanism.angular_bit_offset(), 49);
    assert_eq!(mechanism.data_byte(), raw_track[5]);
    assert!(mechanism.byte_ready_asserted());
    assert_eq!(
        mechanism.byte_ready_transition_sequence(),
        transition_sequence + 1
    );

    mechanism.acknowledge_byte_ready();
    assert!(!mechanism.byte_ready_asserted());
    assert_eq!(
        mechanism.byte_ready_transition_sequence(),
        transition_sequence + 2
    );
}

#[test]
fn steps_only_for_adjacent_energized_phases() {
    let mut mechanism = Drive1541Mechanism::new();
    let control = |stepper_phase| Drive1541ControlState {
        led_on: false,
        motor_on: true,
        speed_zone: Drive1541SpeedZone::Zone0,
        stepper_phase,
    };

    mechanism
        .apply_control_state(control(Drive1541StepperPhase::Phase3))
        .unwrap();
    assert_eq!(mechanism.current_half_track(), 37);
    mechanism
        .apply_control_state(control(Drive1541StepperPhase::Phase2))
        .unwrap();
    assert_eq!(mechanism.current_half_track(), 36);
    mechanism
        .apply_control_state(control(Drive1541StepperPhase::Phase0))
        .unwrap();
    assert_eq!(mechanism.current_half_track(), 36);
    mechanism
        .apply_control_state(Drive1541ControlState {
            motor_on: false,
            stepper_phase: Drive1541StepperPhase::Phase3,
            ..control(Drive1541StepperPhase::Phase3)
        })
        .unwrap();
    assert_eq!(mechanism.current_half_track(), 36);
}

#[test]
fn writes_and_commits_d64_tracks_without_bypassing_write_protection() {
    let mut writable = Drive1541Mechanism::new();
    writable
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    finish_insertion(&mut writable);
    writable.set_speed_zone(Drive1541SpeedZone::Zone0);
    writable.set_write_data_byte(0xa5);
    writable.set_read_mode(false);
    writable.set_motor_on(true);
    writable.tick(32).unwrap();
    assert_eq!(writable.read_raw_half_track(36).unwrap()[0], 0x00);
    writable.tick(32).unwrap();
    assert_eq!(writable.read_raw_half_track(36).unwrap()[1], 0xa5);
    assert_eq!(writable.dirty_half_tracks(), [36]);
    assert_eq!(
        writable.eject_disk(),
        Err(Drive1541MechanismError::UncommittedD64Writes)
    );
    let report = writable.commit_raw_track_writes_to_d64().unwrap();
    assert_eq!(report.committed_half_tracks, [36]);
    assert!(report.failures.is_empty());
    assert!(report.remaining_dirty_half_tracks.is_empty());
    assert!(matches!(
        writable.eject_disk().unwrap(),
        Drive1541DiskImage::D64(_)
    ));

    let mut protected = Drive1541Mechanism::new();
    protected
        .mount_disk(Drive1541DiskImage::D64(d64_disk(true)))
        .unwrap();
    finish_insertion(&mut protected);
    protected.set_speed_zone(Drive1541SpeedZone::Zone0);
    protected.set_write_data_byte(0xa5);
    protected.set_read_mode(false);
    protected.set_motor_on(true);
    protected.tick(64).unwrap();
    assert_eq!(protected.read_raw_half_track(36).unwrap()[0], 0xff);
    assert!(protected.dirty_half_tracks().is_empty());
}

#[test]
fn exposes_two_sensor_pulses_for_immediate_disk_replacement() {
    let mut mechanism = Drive1541Mechanism::new();
    assert!(mechanism.write_protected());
    assert!(!mechanism.write_protect_sensor_active());
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    finish_insertion(&mut mechanism);
    assert!(!mechanism.write_protect_sensor_active());

    mechanism.eject_disk().unwrap();
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    assert!(mechanism.write_protect_sensor_active());
    mechanism.tick(DRIVE_1541_DISK_REMOVAL_CYCLES).unwrap();
    assert!(!mechanism.write_protect_sensor_active());
    mechanism
        .tick(DRIVE_1541_DISK_REPLACEMENT_GAP_CYCLES - DRIVE_1541_DISK_REMOVAL_CYCLES)
        .unwrap();
    assert!(mechanism.write_protect_sensor_active());
    mechanism
        .tick(DRIVE_1541_DISK_INSERTION_CYCLES - DRIVE_1541_DISK_REPLACEMENT_GAP_CYCLES)
        .unwrap();
    assert!(!mechanism.write_protect_sensor_active());
}

#[test]
fn emits_an_so_edge_for_each_byte_while_ca1_remains_asserted() {
    let mut mechanism = Drive1541Mechanism::new();
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    finish_insertion(&mut mechanism);
    mechanism.set_speed_zone(Drive1541SpeedZone::Zone0);
    mechanism.set_write_data_byte(0xff);
    mechanism.set_read_mode(false);
    mechanism.set_motor_on(true);
    mechanism.tick(64).unwrap();

    assert_eq!(mechanism.byte_ready_edge_sequence(), 2);
    assert_eq!(mechanism.byte_ready_transition_sequence(), 1);
    assert!(mechanism.byte_ready_asserted());
}

#[test]
fn mounts_g64_and_preserves_speed_metadata_independently_from_rpm() {
    let mut image = g64_disk();
    image
        .set_half_track(
            36,
            &[0x00, 0x00],
            Some(G64SpeedMap::Constant(G64SpeedZone::Zone0)),
        )
        .unwrap();
    image
        .write_half_track_byte(36, 0, 0x00, Some(G64SpeedZone::Zone2))
        .unwrap();
    let mut mechanism = Drive1541Mechanism::new();
    mechanism
        .mount_disk(Drive1541DiskImage::G64(image))
        .unwrap();
    finish_insertion(&mut mechanism);
    mechanism.set_motor_on(true);
    mechanism.tick(12_500).unwrap();
    assert_eq!(mechanism.angular_bit_offset(), 1);
    mechanism.tick(12_500 * 15).unwrap();
    assert_eq!(mechanism.angular_bit_offset(), 0);
    mechanism.eject_disk().unwrap();

    let mut writer = Drive1541Mechanism::new();
    writer
        .mount_disk(Drive1541DiskImage::G64(g64_disk()))
        .unwrap();
    finish_insertion(&mut writer);
    writer.set_speed_zone(Drive1541SpeedZone::Zone2);
    writer.set_write_data_byte(0xa5);
    writer.set_read_mode(false);
    writer.set_motor_on(true);
    writer.tick(56).unwrap();
    let Drive1541DiskImage::G64(image) = writer.eject_disk().unwrap() else {
        panic!("mounted G64 changed media kind");
    };
    assert_eq!(image.half_track(36).unwrap().unwrap().bytes()[0], 0x00);
    assert_eq!(image.half_track(36).unwrap().unwrap().bytes()[1], 0xa5);
    assert_eq!(
        image.speed_zone_at_byte(36, 1).unwrap(),
        G64SpeedZone::Zone2
    );
}

#[test]
fn absent_g64_half_tracks_generate_deterministic_weak_flux() {
    let mut mechanism = Drive1541Mechanism::new();
    mechanism
        .mount_disk(Drive1541DiskImage::G64(g64_disk()))
        .unwrap();
    finish_insertion(&mut mechanism);
    mechanism
        .apply_control_state(Drive1541ControlState {
            led_on: false,
            motor_on: true,
            speed_zone: Drive1541SpeedZone::Zone2,
            stepper_phase: Drive1541StepperPhase::Phase1,
        })
        .unwrap();
    assert_eq!(
        mechanism.read_raw_half_track(35).unwrap(),
        vec![0; D64_GCR_RAW_TRACK_SIZE[3]]
    );

    let mut observed = BTreeSet::new();
    let mut previous_sequence = mechanism.byte_ready_edge_sequence();
    for _ in 0..2_000 {
        mechanism.tick(1).unwrap();
        if mechanism.byte_ready_edge_sequence() != previous_sequence {
            previous_sequence = mechanism.byte_ready_edge_sequence();
            observed.insert(mechanism.last_byte_ready_edge_data());
        }
    }
    assert!(observed.len() > 1);
    assert!(observed.iter().any(|&value| value != 0x55));
}

#[test]
fn keeps_unrepresentable_half_tracks_dirty_and_requires_explicit_lifecycle() {
    let mut mechanism = Drive1541Mechanism::new();
    mechanism
        .mount_disk(Drive1541DiskImage::D64(d64_disk(false)))
        .unwrap();
    assert_eq!(
        mechanism.mount_disk(Drive1541DiskImage::D64(d64_disk(false))),
        Err(Drive1541MechanismError::DiskAlreadyMounted)
    );
    finish_insertion(&mut mechanism);
    mechanism
        .apply_control_state(Drive1541ControlState {
            led_on: false,
            motor_on: true,
            speed_zone: Drive1541SpeedZone::Zone0,
            stepper_phase: Drive1541StepperPhase::Phase3,
        })
        .unwrap();
    mechanism.set_write_data_byte(0x55);
    mechanism.set_read_mode(false);
    mechanism.tick(32).unwrap();
    let report = mechanism.commit_raw_track_writes_to_d64().unwrap();
    assert!(report.committed_half_tracks.is_empty());
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].reason.contains("full tracks only"));
    assert_eq!(report.remaining_dirty_half_tracks, [37]);
    mechanism.discard_raw_track_writes_and_eject().unwrap();
    assert_eq!(mechanism.eject_disk(), Err(Drive1541MechanismError::Empty));
}
