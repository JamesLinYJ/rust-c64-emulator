// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - G64 raw half-track media integration tests
//
//   File:       tests/g64_media.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::media::g64::{
    G64_FIRST_HALF_TRACK, G64DiskImage, G64ImageError, G64SpeedMap, G64SpeedZone,
};

const HALF_TRACK_COUNT: u8 = 4;
const MAXIMUM_TRACK_LENGTH: usize = 8;
const TABLE_END: usize = 12 + 4 * 8;
const TRACK_BLOCK_LENGTH: usize = 2 + MAXIMUM_TRACK_LENGTH;
const SPEED_MAP_LENGTH: usize = MAXIMUM_TRACK_LENGTH / 4;

fn fixture() -> Vec<u8> {
    let track_offset = TABLE_END;
    let speed_map_offset = track_offset + TRACK_BLOCK_LENGTH;
    let mut bytes = vec![0; speed_map_offset + SPEED_MAP_LENGTH];
    bytes[..8].copy_from_slice(b"GCR-1541");
    bytes[8] = 0;
    bytes[9] = HALF_TRACK_COUNT;
    write_u16(&mut bytes, 10, u16::try_from(MAXIMUM_TRACK_LENGTH).unwrap());
    write_u32(&mut bytes, 12, u32::try_from(track_offset).unwrap());

    let speed_table_offset = 12 + usize::from(HALF_TRACK_COUNT) * 4;
    write_u32(
        &mut bytes,
        speed_table_offset,
        u32::try_from(speed_map_offset).unwrap(),
    );
    write_u32(&mut bytes, speed_table_offset + 4, 3);
    write_u32(&mut bytes, speed_table_offset + 8, 2);
    write_u32(&mut bytes, speed_table_offset + 12, 1);

    let track_data = [0xff, 0x55, 0xa5, 0x00, 0x81];
    write_u16(
        &mut bytes,
        track_offset,
        u16::try_from(track_data.len()).unwrap(),
    );
    bytes[track_offset + 2..track_offset + 2 + track_data.len()].copy_from_slice(&track_data);
    bytes[speed_map_offset..].copy_from_slice(&[0b11_10_01_00, 0b01_01_01_01]);
    bytes
}

#[test]
fn parses_absent_half_tracks_and_per_byte_speed_zones() {
    let image = G64DiskImage::parse(&fixture(), false).unwrap();

    assert_eq!(image.first_half_track(), G64_FIRST_HALF_TRACK);
    assert_eq!(image.last_half_track(), 5);
    assert_eq!(image.maximum_track_length(), MAXIMUM_TRACK_LENGTH);
    assert_eq!(
        image.half_track(2).unwrap().unwrap().bytes(),
        &[0xff, 0x55, 0xa5, 0x00, 0x81]
    );
    assert!(image.half_track(3).unwrap().is_none());
    assert_eq!(
        (0..5)
            .map(|index| image.speed_zone_at_byte(2, index).unwrap())
            .collect::<Vec<_>>(),
        [
            G64SpeedZone::Zone3,
            G64SpeedZone::Zone2,
            G64SpeedZone::Zone1,
            G64SpeedZone::Zone0,
            G64SpeedZone::Zone1,
        ]
    );
    assert_eq!(image.speed_zone_at_byte(3, 0).unwrap(), G64SpeedZone::Zone3);
}

#[test]
fn canonical_serialization_preserves_tracks_and_absent_variable_maps() {
    let original = G64DiskImage::parse(&fixture(), false).unwrap();
    let reparsed = G64DiskImage::parse(&original.to_bytes().unwrap(), false).unwrap();
    assert_eq!(reparsed.half_track(2), original.half_track(2));
    assert!(reparsed.half_track(3).unwrap().is_none());
    for byte_index in 0..MAXIMUM_TRACK_LENGTH {
        assert_eq!(
            reparsed.speed_zone_at_byte(2, byte_index),
            original.speed_zone_at_byte(2, byte_index)
        );
    }

    let mut bytes = fixture();
    let extra_speed_map_offset = bytes.len();
    bytes.extend_from_slice(&[0b00_01_10_11, 0]);
    let speed_table_offset = 12 + usize::from(HALF_TRACK_COUNT) * 4;
    write_u32(
        &mut bytes,
        speed_table_offset + 4,
        u32::try_from(extra_speed_map_offset).unwrap(),
    );
    let parsed = G64DiskImage::parse(&bytes, false).unwrap();
    let reparsed = G64DiskImage::parse(&parsed.to_bytes().unwrap(), false).unwrap();
    assert!(reparsed.half_track(3).unwrap().is_none());
    assert_eq!(
        (0..4)
            .map(|index| reparsed.speed_zone_at_byte(3, index).unwrap())
            .collect::<Vec<_>>(),
        [
            G64SpeedZone::Zone0,
            G64SpeedZone::Zone1,
            G64SpeedZone::Zone2,
            G64SpeedZone::Zone3,
        ]
    );
}

#[test]
fn supports_writes_write_protection_and_later_half_tracks() {
    let mut image = G64DiskImage::parse(&fixture(), false).unwrap();
    let original = image.clone();
    assert_eq!(
        image.set_half_track(36, &[0xa5], Some(G64SpeedMap::Variable(vec![0]))),
        Err(G64ImageError::InvalidVariableSpeedMapLength {
            length: 1,
            expected: 2,
        })
    );
    assert_eq!(image, original);

    image
        .set_half_track(
            3,
            &[0x55, 0xaa],
            Some(G64SpeedMap::Constant(G64SpeedZone::Zone2)),
        )
        .unwrap();
    image
        .write_half_track_byte(3, 1, 0x7e, Some(G64SpeedZone::Zone3))
        .unwrap();
    assert_eq!(image.half_track(3).unwrap().unwrap().bytes(), &[0x55, 0x7e]);
    assert_eq!(image.speed_zone_at_byte(3, 0).unwrap(), G64SpeedZone::Zone2);
    assert_eq!(image.speed_zone_at_byte(3, 1).unwrap(), G64SpeedZone::Zone3);

    assert!(image.half_track(36).unwrap().is_none());
    assert_eq!(
        image.speed_zone_at_byte(36, 0).unwrap(),
        G64SpeedZone::Zone2
    );
    image
        .set_half_track(
            36,
            &[0xa5],
            Some(G64SpeedMap::Constant(G64SpeedZone::Zone2)),
        )
        .unwrap();
    let reparsed = G64DiskImage::parse(&image.to_bytes().unwrap(), false).unwrap();
    assert_eq!(reparsed.last_half_track(), 36);
    assert_eq!(reparsed.half_track(36).unwrap().unwrap().bytes(), &[0xa5]);

    let mut protected = G64DiskImage::parse(&fixture(), true).unwrap();
    assert_eq!(
        protected.write_half_track_byte(2, 0, 0, None),
        Err(G64ImageError::WriteProtected)
    );
    assert_eq!(
        protected.set_half_track(3, &[0x55], None),
        Err(G64ImageError::WriteProtected)
    );
}

#[test]
fn rejects_malformed_headers_tables_offsets_and_track_lengths() {
    let mut bad_signature = fixture();
    bad_signature[0] = 0;
    assert_eq!(
        G64DiskImage::parse(&bad_signature, false),
        Err(G64ImageError::InvalidSignature)
    );

    let mut bad_version = fixture();
    bad_version[8] = 1;
    assert_eq!(
        G64DiskImage::parse(&bad_version, false),
        Err(G64ImageError::UnsupportedVersion(1))
    );

    assert!(matches!(
        G64DiskImage::parse(&fixture()[..TABLE_END - 1], false),
        Err(G64ImageError::TruncatedTables { .. })
    ));

    let mut header_track_offset = fixture();
    write_u32(&mut header_track_offset, 12, 12);
    assert!(matches!(
        G64DiskImage::parse(&header_track_offset, false),
        Err(G64ImageError::TrackOffsetInsideTables { half_track: 2, .. })
    ));

    let mut oversized_track = fixture();
    write_u16(
        &mut oversized_track,
        TABLE_END,
        u16::try_from(MAXIMUM_TRACK_LENGTH + 1).unwrap(),
    );
    assert!(matches!(
        G64DiskImage::parse(&oversized_track, false),
        Err(G64ImageError::InvalidTrackLength { half_track: 2, .. })
    ));

    let mut bad_speed_offset = fixture();
    let bad_speed_offset_value = u32::try_from(bad_speed_offset.len()).unwrap();
    let speed_table_offset = 12 + usize::from(HALF_TRACK_COUNT) * 4;
    write_u32(
        &mut bad_speed_offset,
        speed_table_offset,
        bad_speed_offset_value,
    );
    assert!(matches!(
        G64DiskImage::parse(&bad_speed_offset, false),
        Err(G64ImageError::SpeedMapOutOfBounds { half_track: 2, .. })
    ));
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
