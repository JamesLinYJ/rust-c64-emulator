// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE VIC-II reference runner
//
//   File:       vice_vic.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use c64_core::devices::vic::C64_PALETTE;
use c64_core::{C64Core, C64Firmware, CoreConfig, CpuBusAccessKind, MemoryWriteSource};
use serde::Serialize;

const BOOT_FRAME_COUNT: u64 = 200;
const TEST_FRAME_LIMIT: u32 = 20;
const VICE_TEST_EXIT_PORT: u16 = 0xd7ff;
const MAXIMUM_BOOT_CPU_SLOTS: u64 = 8_000_000;
const MAXIMUM_TEST_CPU_SLOTS_PER_FRAME: u64 = 100_000;

#[derive(Serialize)]
struct RunReport {
    exit_code: u8,
    frames: u32,
    frame_generation: u64,
    program_counter: u16,
    raster_cycle: u8,
    raster_line: u16,
    system_cycles: u64,
}

struct Arguments {
    basic_path: PathBuf,
    character_path: PathBuf,
    kernal_path: PathBuf,
    program_path: PathBuf,
    entry_point: u16,
    frame_path: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    let firmware = C64Firmware::new(
        &fs::read(&arguments.basic_path)?,
        &fs::read(&arguments.character_path)?,
        &fs::read(&arguments.kernal_path)?,
    )?;
    let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);
    core.reset()?;
    boot_to_instruction_boundary(&mut core)?;
    install_prg(&mut core, &fs::read(&arguments.program_path)?)?;
    if !core.set_cpu_program_counter(arguments.entry_point) {
        return Err(invalid_data(
            "VICE entry point was requested away from an instruction boundary",
        )
        .into());
    }

    let (exit_code, frames) = run_test(&mut core)?;
    write_palette_indices(&arguments.frame_path, core.devices().vic().frame_pixels())?;
    let report = RunReport {
        exit_code,
        frames,
        frame_generation: core.devices().vic().frame_generation(),
        program_counter: core.cpu_state().program_counter,
        raster_cycle: core.devices().vic().current_raster_cycle(),
        raster_line: core.devices().vic().current_raster_line(),
        system_cycles: core.timestamp().system_cycle,
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn parse_arguments() -> Result<Arguments, io::Error> {
    let mut arguments = env::args_os().skip(1);
    let basic_path = required_path(&mut arguments, "BASIC ROM path")?;
    let character_path = required_path(&mut arguments, "character ROM path")?;
    let kernal_path = required_path(&mut arguments, "KERNAL ROM path")?;
    let program_path = required_path(&mut arguments, "PRG path")?;
    let entry_point_text = arguments
        .next()
        .ok_or_else(|| invalid_input("missing hexadecimal entry point"))?;
    let entry_point_text = entry_point_text
        .to_str()
        .ok_or_else(|| invalid_input("entry point is not valid Unicode"))?;
    let entry_point =
        u16::from_str_radix(entry_point_text.trim_start_matches("0x"), 16).map_err(|error| {
            invalid_input(format!("invalid entry point {entry_point_text}: {error}"))
        })?;
    let frame_path = required_path(&mut arguments, "frame output path")?;
    if arguments.next().is_some() {
        return Err(invalid_input("unexpected extra arguments"));
    }
    Ok(Arguments {
        basic_path,
        character_path,
        kernal_path,
        program_path,
        entry_point,
        frame_path,
    })
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

fn boot_to_instruction_boundary(core: &mut C64Core) -> Result<(), Box<dyn Error>> {
    let target_generation = core
        .devices()
        .vic()
        .frame_generation()
        .saturating_add(BOOT_FRAME_COUNT);
    for _ in 0..MAXIMUM_BOOT_CPU_SLOTS {
        if core.devices().vic().frame_generation() >= target_generation
            && core.cpu_is_at_instruction_boundary()
        {
            return Ok(());
        }
        core.run_cpu_slots(1)?;
        reject_jammed_cpu(core)?;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("C64 boot did not complete {BOOT_FRAME_COUNT} PAL frames"),
    )
    .into())
}

fn install_prg(core: &mut C64Core, program: &[u8]) -> Result<(), io::Error> {
    let [low, high, payload @ ..] = program else {
        return Err(invalid_data("VICE VIC-II PRG has no load address"));
    };
    let load_address = usize::from(u16::from_le_bytes([*low, *high]));
    let end = load_address
        .checked_add(payload.len())
        .ok_or_else(|| invalid_data("VICE VIC-II PRG range overflow"))?;
    if end > 0x1_0000 {
        return Err(invalid_data(
            "VICE VIC-II PRG exceeds the 64 KiB address space",
        ));
    }
    for (offset, value) in payload.iter().copied().enumerate() {
        let address = u16::try_from(load_address + offset)
            .map_err(|_| invalid_data("VICE VIC-II PRG address conversion failed"))?;
        core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
    }
    Ok(())
}

fn run_test(core: &mut C64Core) -> Result<(u8, u32), Box<dyn Error>> {
    let mut exit_code = None;
    for frame in 1..=TEST_FRAME_LIMIT {
        let target_generation = core.devices().vic().frame_generation().saturating_add(1);
        let mut completed_frame = false;
        for _ in 0..MAXIMUM_TEST_CPU_SLOTS_PER_FRAME {
            let transaction_count = core.diagnostics().cpu_bus_transactions;
            core.run_cpu_slots(1)?;
            reject_jammed_cpu(core)?;
            let diagnostics = core.diagnostics();
            if diagnostics.cpu_bus_transactions != transaction_count
                && let Some(transaction) = diagnostics.last_cpu_bus_transaction
                && transaction.kind == CpuBusAccessKind::Write
                && transaction.address == VICE_TEST_EXIT_PORT
            {
                exit_code = Some(transaction.value);
            }
            if core.cpu_is_at_instruction_boundary() && core.cpu_state().program_counter < 0x0200 {
                return Err(invalid_data(format!(
                    "VICE VIC-II test branched into zero page at ${:04x}",
                    core.cpu_state().program_counter
                ))
                .into());
            }
            if core.devices().vic().frame_generation() >= target_generation
                && core.cpu_is_at_instruction_boundary()
            {
                completed_frame = true;
                break;
            }
        }
        if !completed_frame {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "VICE VIC-II test did not complete a PAL frame within the slot budget",
            )
            .into());
        }
        if let Some(code) = exit_code {
            return Ok((code, frame));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("VICE VIC-II test did not report within {TEST_FRAME_LIMIT} PAL frames"),
    )
    .into())
}

fn reject_jammed_cpu(core: &C64Core) -> Result<(), io::Error> {
    if core.cpu_is_jammed() {
        return Err(invalid_data(format!(
            "VICE VIC-II test entered the 6510 JAM state at ${:04x}",
            core.cpu_state().program_counter
        )));
    }
    Ok(())
}

fn write_palette_indices(path: &Path, pixels: &[u32]) -> Result<(), io::Error> {
    let mut indices = Vec::with_capacity(pixels.len());
    for pixel in pixels {
        let index = C64_PALETTE
            .iter()
            .position(|candidate| candidate == pixel)
            .ok_or_else(|| invalid_data(format!("VIC-II emitted non-palette RGB ${pixel:06x}")))?;
        indices.push(
            u8::try_from(index)
                .map_err(|_| invalid_data("VIC-II palette index exceeds one byte"))?,
        );
    }
    fs::write(path, indices)
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
