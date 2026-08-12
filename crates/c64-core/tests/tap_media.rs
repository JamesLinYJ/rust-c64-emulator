// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - TAP media integration tests
//
//   File:       tests/tap_media.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::media::tap::{
    TAP_HEADER_SIZE, TapImage, TapImageError, TapPulse, TapVersion, TapVideoStandard,
    WritableTapImage,
};

fn fixture(data: &[u8], version: u8, system: u8, video_standard: u8) -> Vec<u8> {
    let mut bytes = vec![0; TAP_HEADER_SIZE + data.len()];
    bytes[..12].copy_from_slice(b"C64-TAPE-RAW");
    bytes[12] = version;
    bytes[13] = system;
    bytes[14] = video_standard;
    bytes[16..20].copy_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    bytes[TAP_HEADER_SIZE..].copy_from_slice(data);
    bytes
}

#[test]
fn parses_legacy_and_precise_pulses_without_losing_source_clock_units() {
    let legacy = TapImage::parse(&fixture(&[1, 0xff], 0, 0, 0), None).unwrap();
    assert_eq!(legacy.version(), TapVersion::Legacy);
    assert_eq!(legacy.source_clock_hz(), 985_248);
    assert_eq!(legacy.total_source_cycles(), 2_048);
    assert_eq!(
        legacy.pulses(),
        [
            TapPulse {
                data_offset: 0,
                encoded_length: 1,
                source_cycles: 8,
            },
            TapPulse {
                data_offset: 1,
                encoded_length: 1,
                source_cycles: 2_040,
            },
        ]
    );

    let precise = TapImage::parse(&fixture(&[2, 0, 0x34, 0x12, 0], 1, 0, 1), None).unwrap();
    assert_eq!(precise.version(), TapVersion::Precise);
    assert_eq!(precise.video_standard(), TapVideoStandard::Ntsc);
    assert_eq!(precise.source_clock_hz(), 1_022_730);
    assert_eq!(
        precise.pulses(),
        [
            TapPulse {
                data_offset: 0,
                encoded_length: 1,
                source_cycles: 16,
            },
            TapPulse {
                data_offset: 1,
                encoded_length: 4,
                source_cycles: 0x1234,
            },
        ]
    );
}

#[test]
fn rejects_ambiguous_or_malformed_tap_records() {
    let legacy_overflow = fixture(&[0], 0, 0, 0);
    assert_eq!(
        TapImage::parse(&legacy_overflow, None),
        Err(TapImageError::LegacyOverflowPulseDurationRequired { data_offset: 0 })
    );
    assert_eq!(
        TapImage::parse(&legacy_overflow, Some(2_500))
            .unwrap()
            .pulses()[0]
            .source_cycles,
        2_500
    );

    let mut bad_magic = fixture(&[1], 1, 0, 0);
    bad_magic[0] = 0;
    assert_eq!(
        TapImage::parse(&bad_magic, None),
        Err(TapImageError::InvalidSignature)
    );
    assert_eq!(
        TapImage::parse(&fixture(&[1], 2, 0, 0), None),
        Err(TapImageError::UnsupportedVersion(2))
    );
    assert_eq!(
        TapImage::parse(&fixture(&[1], 1, 2, 0), None),
        Err(TapImageError::UnsupportedSystem(2))
    );
    assert_eq!(
        TapImage::parse(&fixture(&[1], 1, 0, 4), None),
        Err(TapImageError::InvalidVideoStandard(4))
    );

    let mut bad_length = fixture(&[1], 1, 0, 0);
    bad_length[16..20].copy_from_slice(&2_u32.to_le_bytes());
    assert_eq!(
        TapImage::parse(&bad_length, None),
        Err(TapImageError::DeclaredDataLengthMismatch {
            declared: 2,
            actual: 1,
        })
    );
    assert_eq!(
        TapImage::parse(&fixture(&[0, 1, 2], 1, 0, 0), None),
        Err(TapImageError::TruncatedPrecisePulse { data_offset: 0 })
    );
    assert_eq!(
        TapImage::parse(&fixture(&[0, 0, 0, 0], 1, 0, 0), None),
        Err(TapImageError::ZeroPrecisePulse { data_offset: 0 })
    );
}

#[test]
fn writable_tap_serializes_and_truncates_only_at_pulse_boundaries() {
    let mut image = WritableTapImage::new(TapVideoStandard::Ntsc);
    image.append_pulse(384).unwrap();
    image.append_pulse(529).unwrap();
    image.append_pulse(0xff_ffff).unwrap();

    let bytes = image.to_bytes().unwrap();
    assert_eq!(
        &bytes[TAP_HEADER_SIZE..],
        &[0x30, 0, 0x11, 0x02, 0, 0, 0xff, 0xff, 0xff]
    );
    let reparsed = TapImage::parse(&bytes, None).unwrap();
    assert_eq!(
        reparsed
            .pulses()
            .iter()
            .map(|pulse| (pulse.encoded_length, pulse.source_cycles))
            .collect::<Vec<_>>(),
        [(1, 384), (4, 529), (4, 0xff_ffff)]
    );

    image.truncate_at_pulse(2).unwrap();
    image.append_pulse(24).unwrap();
    assert_eq!(
        image.pulses(),
        [
            TapPulse {
                data_offset: 0,
                encoded_length: 1,
                source_cycles: 384,
            },
            TapPulse {
                data_offset: 1,
                encoded_length: 4,
                source_cycles: 529,
            },
            TapPulse {
                data_offset: 5,
                encoded_length: 1,
                source_cycles: 24,
            },
        ]
    );
    assert_eq!(image.total_source_cycles(), 937);

    let snapshot = image.clone();
    assert_eq!(
        image.append_pulse(0),
        Err(TapImageError::InvalidRecordedPulseDuration(0))
    );
    assert_eq!(image, snapshot);
    assert_eq!(
        image.truncate_at_pulse(4),
        Err(TapImageError::InvalidPulseIndex {
            index: 4,
            pulse_count: 3,
        })
    );
}
