// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - official EasyProg reference runner
//
//   File:       examples/vice_easyflash.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use c64_core::devices::cartridge::AMD_29F040B_CAPACITY_BYTES;
use c64_core::{C64Core, C64Firmware, CoreConfig, MemoryWriteSource};
use serde::Serialize;

const EASY_PROG_VERSION: &str = "1.6.3";
const BASIC_BOOT_FRAME_LIMIT: u32 = 300;
const ABOUT_FRAME_LIMIT: u32 = 600;
const READY_FRAME_LIMIT: u32 = 300;
const DIALOG_FRAME_LIMIT: u32 = 80;
const FLASH_WRITE_FRAME_LIMIT: u32 = 800;
const KEY_OBSERVATION_FRAMES: u32 = 23;
const POST_PROGRAM_OBSERVATION_FRAMES: u32 = 20;
const MINIMUM_PROGRAMMED_BYTES_PER_CHIP: usize = 0x100;
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

#[derive(Serialize)]
struct EasyFlashReport {
    boot_frames: u32,
    changed_high_bytes: usize,
    changed_low_bytes: usize,
    initialization_frames: u32,
    programming_frames: u32,
}

struct Arguments {
    basic_rom: PathBuf,
    character_rom: PathBuf,
    kernal_rom: PathBuf,
    easyprog_prg: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    let firmware = C64Firmware::new(
        &fs::read(arguments.basic_rom)?,
        &fs::read(arguments.character_rom)?,
        &fs::read(arguments.kernal_rom)?,
    )?;
    let program = fs::read(arguments.easyprog_prg)?;
    let report = run_easyprog(firmware, &program)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn run_easyprog(firmware: C64Firmware, program: &[u8]) -> Result<EasyFlashReport, Box<dyn Error>> {
    let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);
    core.insert_crt(&blank_easyflash_crt(), true)?;
    core.reset()?;
    let boot_frames = boot_to_basic_ready(&mut core)?;

    install_basic_run_prg(&mut core, program)?;
    let about_frames = wait_for_screen_text(
        &mut core,
        &format!("VERSION {EASY_PROG_VERSION}"),
        ABOUT_FRAME_LIMIT,
    )?;
    press_key(&mut core, 0x0d)?;
    let ready_frames =
        wait_for_screen_text(&mut core, "READY. PRESS <M> FOR MENU.", READY_FRAME_LIMIT)?;
    let initialized_screen = screen_text(&core);
    require_screen_text(&initialized_screen, "FLASH DRIVER:")?;
    require_screen_text(&initialized_screen, "AM/M29F040 V1.4")?;
    require_screen_text(&initialized_screen, "SLOTS:")?;
    require_screen_text(&initialized_screen, "1 * 1024 KIBYTE")?;

    press_key(&mut core, b'E')?;
    wait_for_screen_text(&mut core, "TORTURE TEST", DIALOG_FRAME_LIMIT)?;
    press_key(&mut core, b'T')?;
    wait_for_screen_text(&mut core, "THIS WILL ERASE THE CURRENT", DIALOG_FRAME_LIMIT)?;
    press_key(&mut core, 0x0d)?;
    wait_for_screen_text(&mut core, "THIS TEST RUNS ENDLESSLY.", DIALOG_FRAME_LIMIT)?;
    press_key(&mut core, 0x0d)?;

    let mut programming_frames = 0;
    for frame in 1..=FLASH_WRITE_FRAME_LIMIT {
        run_frame(&mut core)?;
        programming_frames = frame;
        let current_screen = screen_text(&core);
        if current_screen.contains("TEST FAILED") {
            return Err(invalid_data(format!(
                "EasyProg torture test reported failure:\n{current_screen}"
            ))
            .into());
        }
        if both_flash_chips_dirty(&core) {
            run_frames(&mut core, POST_PROGRAM_OBSERVATION_FRAMES)?;
            break;
        }
    }
    if !both_flash_chips_dirty(&core) {
        return Err(invalid_data(format!(
            "EasyProg did not program both flash chips within {FLASH_WRITE_FRAME_LIMIT} frames"
        ))
        .into());
    }

    let cartridge = core
        .devices()
        .cartridge()
        .and_then(c64_core::devices::cartridge::Cartridge::easyflash)
        .ok_or_else(|| invalid_data("EasyFlash cartridge disappeared during the reference run"))?;
    let changed_low_bytes = count_programmed_bytes(&cartridge.flash_low().to_bytes());
    let changed_high_bytes = count_programmed_bytes(&cartridge.flash_high().to_bytes());
    if changed_low_bytes < MINIMUM_PROGRAMMED_BYTES_PER_CHIP
        || changed_high_bytes < MINIMUM_PROGRAMMED_BYTES_PER_CHIP
    {
        return Err(invalid_data(format!(
            "EasyProg changed only {changed_low_bytes} ROML and {changed_high_bytes} ROMH bytes"
        ))
        .into());
    }

    Ok(EasyFlashReport {
        boot_frames,
        changed_high_bytes,
        changed_low_bytes,
        initialization_frames: about_frames + ready_frames,
        programming_frames,
    })
}

fn blank_easyflash_crt() -> Vec<u8> {
    let mut bytes = vec![0; 0x40];
    bytes[..16].copy_from_slice(b"C64 CARTRIDGE   ");
    bytes[0x10..0x14].copy_from_slice(&0x40_u32.to_be_bytes());
    bytes[0x14..0x16].copy_from_slice(&0x0100_u16.to_be_bytes());
    bytes[0x16..0x18].copy_from_slice(&32_u16.to_be_bytes());
    bytes[0x18] = 1;
    bytes[0x19] = 0;
    bytes
}

fn install_basic_run_prg(core: &mut C64Core, program: &[u8]) -> Result<(), io::Error> {
    let [low, high, payload @ ..] = program else {
        return Err(invalid_data("EasyProg PRG has no load address"));
    };
    let load_address = u16::from_le_bytes([*low, *high]);
    if load_address != BASIC_PROGRAM_START || payload.is_empty() {
        return Err(invalid_data("EasyProg is not a BASIC program at $0801"));
    }
    let payload_length = u16::try_from(payload.len())
        .map_err(|_| invalid_data("EasyProg exceeds the C64 address space"))?;
    let end_address = load_address
        .checked_add(payload_length)
        .ok_or_else(|| invalid_data("EasyProg load range overflows 16 bits"))?;
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

fn wait_for_screen_text(
    core: &mut C64Core,
    expected: &str,
    frame_limit: u32,
) -> Result<u32, Box<dyn Error>> {
    for frame in 1..=frame_limit {
        run_frame(core)?;
        if screen_text(core).contains(expected) {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "EasyProg did not display {expected:?} within {frame_limit} frames:\n{}",
            screen_text(core)
        ),
    )
    .into())
}

fn press_key(core: &mut C64Core, key: u8) -> Result<(), Box<dyn Error>> {
    enqueue_key(core, key)?;
    run_frames(core, KEY_OBSERVATION_FRAMES)
}

fn enqueue_key(core: &mut C64Core, key: u8) -> Result<(), io::Error> {
    enqueue_keys(core, &[key])
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

fn run_frames(core: &mut C64Core, frames: u32) -> Result<(), Box<dyn Error>> {
    for _ in 0..frames {
        run_frame(core)?;
    }
    Ok(())
}

fn run_frame(core: &mut C64Core) -> Result<(), Box<dyn Error>> {
    let target_generation = core.devices().vic().frame_generation().saturating_add(1);
    for _ in 0..MAXIMUM_CPU_SLOTS_PER_FRAME {
        core.run_cpu_slots(1)?;
        if core.cpu_is_jammed() {
            return Err(invalid_data(format!(
                "EasyProg entered the 6510 JAM state at ${:04x}",
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

fn require_screen_text(screen: &str, expected: &str) -> Result<(), io::Error> {
    if screen.contains(expected) {
        return Ok(());
    }
    Err(invalid_data(format!(
        "EasyProg screen is missing {expected:?}:\n{screen}"
    )))
}

fn both_flash_chips_dirty(core: &C64Core) -> bool {
    core.devices()
        .cartridge()
        .and_then(c64_core::devices::cartridge::Cartridge::easyflash)
        .is_some_and(|cartridge| cartridge.flash_low().dirty() && cartridge.flash_high().dirty())
}

fn count_programmed_bytes(bytes: &[u8]) -> usize {
    debug_assert_eq!(bytes.len(), AMD_29F040B_CAPACITY_BYTES);
    bytes.iter().filter(|value| **value != 0xff).count()
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
        easyprog_prg: required_path(&mut arguments, "EasyProg PRG path")?,
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
