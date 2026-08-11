// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE QuickReu 17xx reference runner
//
//   File:       vice_reu.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use c64_core::devices::reu::ReuSize;
use c64_core::{C64Core, C64Firmware, CoreConfig, MemoryWriteSource};
use serde::Serialize;

const BASIC_BOOT_FRAME_LIMIT: u32 = 300;
const RESULT_FRAME_LIMIT: u32 = 600;
const MAXIMUM_CPU_SLOTS_PER_FRAME: u64 = 100_000;
const BASIC_PROGRAM_START: u16 = 0x0801;
const BASIC_RUN_COMMAND: &[u8] = b"RUN\r";
const KEYBOARD_BUFFER_START: u16 = 0x0277;
const KEYBOARD_BUFFER_CAPACITY: u16 = 0x0289;
const KEYBOARD_BUFFER_COUNT: u16 = 0x00c6;
const SCREEN_START: u16 = 0x0400;
const SCREEN_COLUMNS: usize = 40;
const SCREEN_ROWS: usize = 25;
const READY_PROMPT: [u8; 6] = [0x12, 0x05, 0x01, 0x04, 0x19, 0x2e];
const RESULT_PREFIX: &str = "TEST CLASSES WITH FAILURES:";

#[derive(Serialize)]
struct ReuReport {
    boot_frames: u32,
    dma_bus_cycles: u64,
    dma_system_cycles: u64,
    dma_vic_stall_cycles: u64,
    result_frames: u32,
}

struct Arguments {
    basic_rom: PathBuf,
    character_rom: PathBuf,
    kernal_rom: PathBuf,
    quickreu_prg: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    let firmware = C64Firmware::new(
        &fs::read(arguments.basic_rom)?,
        &fs::read(arguments.character_rom)?,
        &fs::read(arguments.kernal_rom)?,
    )?;
    let program = fs::read(arguments.quickreu_prg)?;
    let report = run_quickreu(firmware, &program)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn run_quickreu(firmware: C64Firmware, program: &[u8]) -> Result<ReuReport, Box<dyn Error>> {
    let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);
    core.attach_reu(ReuSize::Kib512)?;
    core.reset()?;
    let boot_frames = boot_to_basic_ready(&mut core)?;
    install_basic_run_prg(&mut core, program)?;

    for result_frames in 1..=RESULT_FRAME_LIMIT {
        run_frame(&mut core)?;
        let screen = screen_text(&core);
        if result_failure_count(&screen) == Some(0) {
            let diagnostics = core.diagnostics();
            return Ok(ReuReport {
                boot_frames,
                dma_bus_cycles: diagnostics.reu_dma_bus_cycles,
                dma_system_cycles: diagnostics.reu_dma_system_cycles,
                dma_vic_stall_cycles: diagnostics.reu_dma_vic_stall_cycles,
                result_frames,
            });
        }
        if result_failure_count(&screen).is_some() {
            return Err(invalid_data(format!("VICE QuickReu reported failure:\n{screen}")).into());
        }
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "VICE QuickReu did not report within {RESULT_FRAME_LIMIT} frames:\n{}",
            screen_text(&core)
        ),
    )
    .into())
}

fn result_failure_count(screen: &str) -> Option<u32> {
    screen.lines().find_map(|line| {
        line.find(RESULT_PREFIX).and_then(|offset| {
            line[offset + RESULT_PREFIX.len()..]
                .split_ascii_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
        })
    })
}

fn install_basic_run_prg(core: &mut C64Core, program: &[u8]) -> Result<(), io::Error> {
    let [low, high, payload @ ..] = program else {
        return Err(invalid_data("QuickReu PRG has no load address"));
    };
    let load_address = u16::from_le_bytes([*low, *high]);
    if load_address != BASIC_PROGRAM_START || payload.is_empty() {
        return Err(invalid_data("QuickReu is not a BASIC program at $0801"));
    }
    let payload_length = u16::try_from(payload.len())
        .map_err(|_| invalid_data("QuickReu exceeds the C64 address space"))?;
    let end_address = load_address
        .checked_add(payload_length)
        .ok_or_else(|| invalid_data("QuickReu load range overflows 16 bits"))?;
    for (value, offset) in payload.iter().copied().zip(0_u16..) {
        core.write_base_ram(load_address + offset, value, MemoryWriteSource::HostLoader);
    }
    for pointer in [0x002b, 0x00ac] {
        write_ram_word(core, pointer, load_address);
    }
    for pointer in [0x002d, 0x002f, 0x0031, 0x00ae] {
        write_ram_word(core, pointer, end_address);
    }
    enqueue_keys(core, BASIC_RUN_COMMAND)
}

fn boot_to_basic_ready(core: &mut C64Core) -> Result<u32, Box<dyn Error>> {
    let mut ready_was_absent = !has_basic_ready_prompt(core);
    for frame in 1..=BASIC_BOOT_FRAME_LIMIT {
        run_frame(core)?;
        let ready = has_basic_ready_prompt(core);
        if !ready {
            ready_was_absent = true;
        } else if ready_was_absent {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("C64 BASIC did not reach READY within {BASIC_BOOT_FRAME_LIMIT} frames"),
    )
    .into())
}

fn enqueue_keys(core: &mut C64Core, keys: &[u8]) -> Result<(), io::Error> {
    let used = core.read_base_ram(KEYBOARD_BUFFER_COUNT);
    let capacity = core.read_base_ram(KEYBOARD_BUFFER_CAPACITY);
    let key_count = u8::try_from(keys.len())
        .map_err(|_| invalid_data("keyboard input exceeds one-byte capacity"))?;
    if used.saturating_add(key_count) > capacity {
        return Err(invalid_data("C64 keyboard buffer is full"));
    }
    for (key, offset) in keys.iter().copied().zip(0_u16..) {
        core.write_base_ram(
            KEYBOARD_BUFFER_START + u16::from(used) + offset,
            key,
            MemoryWriteSource::HostLoader,
        );
    }
    core.write_base_ram(
        KEYBOARD_BUFFER_COUNT,
        used + key_count,
        MemoryWriteSource::HostLoader,
    );
    Ok(())
}

fn run_frame(core: &mut C64Core) -> Result<(), Box<dyn Error>> {
    let target_generation = core.devices().vic().frame_generation().saturating_add(1);
    for _ in 0..MAXIMUM_CPU_SLOTS_PER_FRAME {
        core.run_cpu_slots(1)?;
        if core.cpu_is_jammed() {
            return Err(invalid_data(format!(
                "QuickReu entered the 6510 JAM state at ${:04x}",
                core.cpu_state().program_counter
            ))
            .into());
        }
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

fn screen_text(core: &C64Core) -> String {
    let mut lines = Vec::with_capacity(SCREEN_ROWS);
    for row in 0..SCREEN_ROWS {
        let mut line = String::with_capacity(SCREEN_COLUMNS);
        for column in 0..SCREEN_COLUMNS {
            let offset = row * SCREEN_COLUMNS + column;
            let address = SCREEN_START
                + u16::try_from(offset).expect("screen offset must fit in the C64 address space");
            line.push(screen_code_to_ascii(core.read_base_ram(address)));
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn screen_code_to_ascii(value: u8) -> char {
    match value & 0x7f {
        0x20 => ' ',
        code @ 0x01..=0x1a => char::from(0x40 + code),
        code @ (0x21..=0x3f | 0x41..=0x5a) => char::from(code),
        _ => '.',
    }
}

fn has_basic_ready_prompt(core: &C64Core) -> bool {
    let screen_end = SCREEN_START + u16::try_from(SCREEN_COLUMNS * SCREEN_ROWS).unwrap_or(0);
    let final_start = screen_end - u16::try_from(READY_PROMPT.len()).unwrap_or(0);
    (SCREEN_START..=final_start).any(|address| {
        READY_PROMPT
            .iter()
            .copied()
            .zip(0_u16..)
            .all(|(expected, offset)| core.read_base_ram(address + offset) == expected)
    })
}

fn write_ram_word(core: &mut C64Core, address: u16, value: u16) {
    let [low, high] = value.to_le_bytes();
    core.write_base_ram(address, low, MemoryWriteSource::HostLoader);
    core.write_base_ram(address + 1, high, MemoryWriteSource::HostLoader);
}

fn parse_arguments() -> Result<Arguments, io::Error> {
    let mut arguments = env::args_os().skip(1);
    let result = Arguments {
        basic_rom: required_path(&mut arguments, "BASIC ROM path")?,
        character_rom: required_path(&mut arguments, "character ROM path")?,
        kernal_rom: required_path(&mut arguments, "KERNAL ROM path")?,
        quickreu_prg: required_path(&mut arguments, "QuickReu PRG path")?,
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
