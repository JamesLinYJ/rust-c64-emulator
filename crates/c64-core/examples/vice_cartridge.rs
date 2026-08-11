// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE Ocean cartridge reference runner
//
//   File:       examples/vice_cartridge.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use c64_core::devices::cartridge::Cartridge;
use c64_core::{
    C64AddressSpace, C64Chipset, C64Core, C64Firmware, CoreConfig, CpuBus, VideoStandard,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const BANK_REGISTER: u16 = 0xde00;
const ROM_LOW_START: u16 = 0x8000;
const ROM_HIGH_START: u16 = 0xa000;
const ROM_WINDOW_SIZE: usize = 0x2000;
const REFERENCE_RUN_FRAMES: u32 = 120;
const MINIMUM_ROM_PC_FRAME_SAMPLES: u32 = 20;
const MAXIMUM_CPU_SLOTS_PER_FRAME: u64 = 100_000;
const TEST_ACTIVITY_COUNTER: u16 = 0x07e7;
const TEST_SCREEN_COPY_START: u16 = 0x0400 + 3 * 40;
const TEST_SCREEN_COPY_SIZE: usize = 0x0100;
const EXPECTED_BANK_SHA256: [&str; 4] = [
    "12c0b361abb12f552bae83fc4f1cd0b646ace5b510bbd53bf9fca371f914b64a",
    "9f1dcbc35c350d6027f98be0f5c8b43b42ca52b7604459c0c42be3aa88913d47",
    "9f1dcbc35c350d6027f98be0f5c8b43b42ca52b7604459c0c42be3aa88913d47",
    "9f1dcbc35c350d6027f98be0f5c8b43b42ca52b7604459c0c42be3aa88913d47",
];
const EXPECTED_SCREEN_COPY_SHA256: &str =
    "c199b812bf803087360c2737b93b2ab39e7a6ea91af8b4b9a736b1a6308c0d5c";

#[derive(Serialize)]
struct CartridgeReport {
    activity: u8,
    bank_sha256: Vec<String>,
    frames: u32,
    rom_pc_frame_samples: u32,
    screen_copy_sha256: String,
}

struct Arguments {
    basic_rom: PathBuf,
    character_rom: PathBuf,
    kernal_rom: PathBuf,
    ocean_crt: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    let firmware = C64Firmware::new(
        &fs::read(arguments.basic_rom)?,
        &fs::read(arguments.character_rom)?,
        &fs::read(arguments.kernal_rom)?,
    )?;
    let crt_bytes = fs::read(arguments.ocean_crt)?;
    let bank_sha256 = verify_banks(firmware.clone(), &crt_bytes)?;

    let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);
    core.insert_crt(&crt_bytes, false)?;
    core.reset()?;
    let mut rom_pc_frame_samples = 0;
    for _ in 0..REFERENCE_RUN_FRAMES {
        run_frame(&mut core)?;
        reject_jammed_cpu(&core)?;
        if (ROM_LOW_START..ROM_HIGH_START).contains(&core.cpu_state().program_counter) {
            rom_pc_frame_samples += 1;
        }
    }
    if rom_pc_frame_samples < MINIMUM_ROM_PC_FRAME_SAMPLES {
        return Err(invalid_data(format!(
            "VICE Ocean CRT produced only {rom_pc_frame_samples} ROM PC frame samples"
        ))
        .into());
    }
    let activity = core.read_base_ram(TEST_ACTIVITY_COUNTER);
    if activity == 0 {
        return Err(invalid_data("VICE Ocean CRT did not advance its activity counter").into());
    }
    let screen_copy = (0..TEST_SCREEN_COPY_SIZE)
        .map(|offset| {
            core.read_base_ram(
                TEST_SCREEN_COPY_START
                    + u16::try_from(offset).expect("screen copy offset must fit in 16 bits"),
            )
        })
        .collect::<Vec<_>>();
    let screen_copy_sha256 = sha256_hex(&screen_copy);
    if screen_copy_sha256 != EXPECTED_SCREEN_COPY_SHA256 {
        return Err(invalid_data(format!(
            "VICE Ocean CRT screen copy SHA-256 changed to {screen_copy_sha256}"
        ))
        .into());
    }

    println!(
        "{}",
        serde_json::to_string(&CartridgeReport {
            activity,
            bank_sha256,
            frames: REFERENCE_RUN_FRAMES,
            rom_pc_frame_samples,
            screen_copy_sha256,
        })?
    );
    Ok(())
}

fn verify_banks(firmware: C64Firmware, crt_bytes: &[u8]) -> Result<Vec<String>, Box<dyn Error>> {
    let cartridge = Cartridge::from_crt_bytes(crt_bytes, false)?;
    let mut address_space = C64AddressSpace::new(firmware);
    let mut chipset = C64Chipset::new(VideoStandard::Pal);
    chipset.attach_cartridge(cartridge)?;
    let mut hashes = Vec::with_capacity(EXPECTED_BANK_SHA256.len());
    for (bank, expected) in EXPECTED_BANK_SHA256.iter().enumerate() {
        let (low_hash, high_hash) = {
            let mut bus = address_space.cpu_bus(&mut chipset);
            bus.write(
                BANK_REGISTER,
                u8::try_from(bank).expect("reference bank must fit in one byte"),
            );
            (
                hash_mapped_window(&mut bus, ROM_LOW_START),
                hash_mapped_window(&mut bus, ROM_HIGH_START),
            )
        };
        if low_hash != *expected || high_hash != *expected {
            return Err(invalid_data(format!(
                "Ocean bank {bank} hashes are ROML={low_hash}, ROMH={high_hash}; expected {expected}"
            ))
            .into());
        }
        hashes.push(low_hash);
    }
    Ok(hashes)
}

fn hash_mapped_window(bus: &mut impl CpuBus, start_address: u16) -> String {
    let bytes = (0..ROM_WINDOW_SIZE)
        .map(|offset| {
            bus.read(
                start_address
                    + u16::try_from(offset).expect("ROM window offset must fit in 16 bits"),
            )
        })
        .collect::<Vec<_>>();
    sha256_hex(&bytes)
}

fn run_frame(core: &mut C64Core) -> Result<(), Box<dyn Error>> {
    let target_generation = core.devices().vic().frame_generation().saturating_add(1);
    for _ in 0..MAXIMUM_CPU_SLOTS_PER_FRAME {
        core.run_cpu_slots(1)?;
        if core.devices().vic().frame_generation() >= target_generation
            && core.cpu_is_at_instruction_boundary()
        {
            return Ok(());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "C64 did not complete a PAL frame within the CPU slot budget",
    )
    .into())
}

fn reject_jammed_cpu(core: &C64Core) -> Result<(), io::Error> {
    if core.cpu_is_jammed() {
        return Err(invalid_data(format!(
            "VICE Ocean CRT entered the 6510 JAM state at ${:04x}",
            core.cpu_state().program_counter
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn parse_arguments() -> Result<Arguments, io::Error> {
    let mut arguments = env::args_os().skip(1);
    let result = Arguments {
        basic_rom: required_path(&mut arguments, "BASIC ROM path")?,
        character_rom: required_path(&mut arguments, "character ROM path")?,
        kernal_rom: required_path(&mut arguments, "KERNAL ROM path")?,
        ocean_crt: required_path(&mut arguments, "Ocean CRT path")?,
    };
    if arguments.next().is_some() {
        return Err(invalid_input("unexpected extra arguments"));
    }
    Ok(result)
}

fn required_path(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    label: &str,
) -> Result<PathBuf, io::Error> {
    arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| invalid_input(format!("missing {label}")))
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
