// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - CRT cartridge image integration tests
//
//   File:       tests/crt_media.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::media::crt::{
    CRT_CHIP_TYPE_ROM, CRT_HARDWARE_TYPE_OCEAN, CRT_HEADER_SIZE, CrtImage, CrtImageError,
};

struct TestChip {
    bank: u16,
    chip_type: u16,
    data: Vec<u8>,
    load_address: u16,
    padding: usize,
}

fn fixture(chips: &[TestChip], hardware_type: u16) -> Vec<u8> {
    let length = CRT_HEADER_SIZE
        + chips
            .iter()
            .map(|chip| 0x10 + chip.data.len() + chip.padding)
            .sum::<usize>();
    let mut bytes = vec![0; length];
    bytes[..16].copy_from_slice(b"C64 CARTRIDGE   ");
    write_u32(&mut bytes, 0x10, u32::try_from(CRT_HEADER_SIZE).unwrap());
    write_u16(&mut bytes, 0x14, 0x0100);
    write_u16(&mut bytes, 0x16, hardware_type);
    bytes[0x18] = 0;
    bytes[0x19] = 0;
    bytes[0x1a] = 7;
    bytes[0x20..0x2a].copy_from_slice(b"CODEX CART");

    let mut offset = CRT_HEADER_SIZE;
    for chip in chips {
        let packet_length = 0x10 + chip.data.len() + chip.padding;
        bytes[offset..offset + 4].copy_from_slice(b"CHIP");
        write_u32(
            &mut bytes,
            offset + 4,
            u32::try_from(packet_length).unwrap(),
        );
        write_u16(&mut bytes, offset + 8, chip.chip_type);
        write_u16(&mut bytes, offset + 0x0a, chip.bank);
        write_u16(&mut bytes, offset + 0x0c, chip.load_address);
        write_u16(
            &mut bytes,
            offset + 0x0e,
            u16::try_from(chip.data.len()).unwrap(),
        );
        bytes[offset + 0x10..offset + 0x10 + chip.data.len()].copy_from_slice(&chip.data);
        offset += packet_length;
    }
    bytes
}

#[test]
fn parses_big_endian_header_packets_and_ignores_declared_packet_padding() {
    let bytes = fixture(
        &[TestChip {
            bank: 3,
            chip_type: CRT_CHIP_TYPE_ROM,
            data: vec![0x81, 0x82, 0x83],
            load_address: 0x8000,
            padding: 5,
        }],
        CRT_HARDWARE_TYPE_OCEAN,
    );
    let image = CrtImage::parse(&bytes).unwrap();

    assert_eq!(image.header.header_length, CRT_HEADER_SIZE);
    assert_eq!(image.header.version, 0x0100);
    assert_eq!(image.header.hardware_type, CRT_HARDWARE_TYPE_OCEAN);
    assert_eq!(image.header.hardware_subtype, 7);
    assert_eq!(image.header.name, "CODEX CART");
    assert!(!image.header.game_line_high);
    assert!(!image.header.exrom_line_high);
    assert_eq!(image.chips.len(), 1);
    assert_eq!(image.chips[0].bank, 3);
    assert_eq!(image.chips[0].load_address, 0x8000);
    assert_eq!(image.chips[0].image_size, 3);
    assert_eq!(image.chips[0].packet_length, 0x18);
    assert_eq!(image.chips[0].data, [0x81, 0x82, 0x83]);
}

#[test]
fn rejects_invalid_lines_truncated_packets_and_ranges_crossing_sixteen_bits() {
    let mut invalid_line = fixture(&[], CRT_HARDWARE_TYPE_OCEAN);
    invalid_line[0x18] = 2;
    assert_eq!(
        CrtImage::parse(&invalid_line),
        Err(CrtImageError::InvalidDigitalLine {
            name: "EXROM",
            value: 2,
        })
    );

    let mut truncated = fixture(
        &[TestChip {
            bank: 0,
            chip_type: CRT_CHIP_TYPE_ROM,
            data: vec![0; 4],
            load_address: 0x8000,
            padding: 0,
        }],
        CRT_HARDWARE_TYPE_OCEAN,
    );
    truncated.truncate(truncated.len() - 1);
    assert!(matches!(
        CrtImage::parse(&truncated),
        Err(CrtImageError::TruncatedRange { .. })
    ));

    let crossing = fixture(
        &[TestChip {
            bank: 0,
            chip_type: CRT_CHIP_TYPE_ROM,
            data: vec![0; 2],
            load_address: 0xffff,
            padding: 0,
        }],
        CRT_HARDWARE_TYPE_OCEAN,
    );
    assert_eq!(
        CrtImage::parse(&crossing),
        Err(CrtImageError::ChipRangeCrossesAddressSpace {
            load_address: 0xffff,
            image_size: 2,
        })
    );
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
