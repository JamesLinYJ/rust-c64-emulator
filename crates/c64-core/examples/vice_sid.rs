// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE SID noise/writeback reference runner
//
//   File:       vice_sid.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use c64_core::{C64Core, C64Firmware, CoreConfig, CpuBusAccessKind, MemoryWriteSource, SidModel};
use serde::Serialize;

const BOOT_FRAME_COUNT: u64 = 200;
const TEST_FRAME_LIMIT: u32 = 1_000;
const VICE_TEST_EXIT_PORT: u16 = 0xd7ff;
const MAXIMUM_BOOT_CPU_SLOTS: u64 = 8_000_000;
const MAXIMUM_TEST_CPU_SLOTS_PER_FRAME: u64 = 100_000;

#[derive(Serialize)]
struct RunReport {
    exit_code: Option<u8>,
    frames: u32,
    memory: Vec<u8>,
    program_counter: u16,
    system_cycles: u64,
}

struct Arguments {
    basic_path: PathBuf,
    character_path: PathBuf,
    kernal_path: PathBuf,
    program_path: PathBuf,
    sid_model: SidModel,
    result_length: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    let firmware = C64Firmware::new(
        &fs::read(&arguments.basic_path)?,
        &fs::read(&arguments.character_path)?,
        &fs::read(&arguments.kernal_path)?,
    )?;
    let config = CoreConfig {
        sid_model: arguments.sid_model,
        ..CoreConfig::default()
    };
    let mut core = C64Core::with_firmware(config, firmware);
    core.reset()?;
    boot_to_instruction_boundary(&mut core)?;
    install_prg(&mut core, &fs::read(&arguments.program_path)?)?;
    if !core.set_cpu_program_counter(0x080d) {
        return Err(invalid_data(
            "VICE SID entry point was requested away from an instruction boundary",
        )
        .into());
    }

    let (exit_code, frames) = run_test(&mut core)?;
    let memory = (0..arguments.result_length)
        .map(|offset| {
            let offset = u16::try_from(offset)
                .map_err(|_| invalid_input("SID result length exceeds 16-bit address range"))?;
            Ok(core.read_base_ram(0x0400_u16.wrapping_add(offset)))
        })
        .collect::<Result<Vec<_>, io::Error>>()?;
    let report = RunReport {
        exit_code,
        frames,
        memory,
        program_counter: core.cpu_state().program_counter,
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
    let model_text = required_text(&mut arguments, "SID model")?;
    let sid_model = match model_text.as_str() {
        "6581" => SidModel::Mos6581,
        "8580" => SidModel::Mos8580,
        _ => return Err(invalid_input(format!("unsupported SID model {model_text}"))),
    };
    let result_length_text = required_text(&mut arguments, "result length")?;
    let result_length = result_length_text
        .parse::<usize>()
        .map_err(|error| invalid_input(format!("invalid result length: {error}")))?;
    if result_length > 0xfc00 {
        return Err(invalid_input("SID result range exceeds base RAM"));
    }
    if arguments.next().is_some() {
        return Err(invalid_input("unexpected extra arguments"));
    }
    Ok(Arguments {
        basic_path,
        character_path,
        kernal_path,
        program_path,
        sid_model,
        result_length,
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

fn required_text(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    label: &str,
) -> Result<String, io::Error> {
    arguments
        .next()
        .ok_or_else(|| invalid_input(format!("missing {label}")))?
        .into_string()
        .map_err(|_| invalid_input(format!("{label} is not valid Unicode")))
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
        return Err(invalid_data("VICE SID PRG has no load address"));
    };
    let load_address = usize::from(u16::from_le_bytes([*low, *high]));
    let end = load_address
        .checked_add(payload.len())
        .ok_or_else(|| invalid_data("VICE SID PRG range overflow"))?;
    if end > 0x1_0000 {
        return Err(invalid_data("VICE SID PRG exceeds 64 KiB"));
    }
    for (offset, value) in payload.iter().copied().enumerate() {
        let address = u16::try_from(load_address + offset)
            .map_err(|_| invalid_data("VICE SID PRG address conversion failed"))?;
        core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
    }
    Ok(())
}

fn run_test(core: &mut C64Core) -> Result<(Option<u8>, u32), Box<dyn Error>> {
    for frame in 1..=TEST_FRAME_LIMIT {
        let target_generation = core.devices().vic().frame_generation().saturating_add(1);
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
                return Ok((Some(transaction.value), frame));
            }
            if core.devices().vic().frame_generation() >= target_generation
                && core.cpu_is_at_instruction_boundary()
            {
                break;
            }
        }
    }
    Ok((None, TEST_FRAME_LIMIT))
}

fn reject_jammed_cpu(core: &C64Core) -> Result<(), io::Error> {
    if core.cpu_is_jammed() {
        return Err(invalid_data(format!(
            "VICE SID test entered the 6510 JAM state at ${:04x}",
            core.cpu_state().program_counter
        )));
    }
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
