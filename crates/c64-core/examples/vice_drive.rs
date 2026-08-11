// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE Commodore 1541 end-to-end reference runner
//
//   File:       vice_drive.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use c64_core::devices::drive1541::mechanism::Drive1541DiskImage;
use c64_core::media::d64::D64DiskImage;
use c64_core::{C64Core, C64Firmware, CoreConfig, MemoryWriteSource};
use serde::Serialize;

const BASIC_BOOT_FRAME_LIMIT: u32 = 300;
const DRIVE_COMMAND_FRAME_LIMIT: u32 = 600;
const MAXIMUM_CPU_SLOTS_PER_FRAME: u64 = 100_000;
const SCREEN_START: u16 = 0x0400;
const SCREEN_END_EXCLUSIVE: u16 = 0x07e8;
const SCREEN_SPACE: u8 = 0x20;
const KEYBOARD_BUFFER_START: u16 = 0x0277;
const KEYBOARD_BUFFER_CAPACITY: u16 = 0x0289;
const KEYBOARD_BUFFER_COUNT: u16 = 0x00c6;
const BASIC_TEXT_END_POINTER: u16 = 0x002d;
const BASIC_PROGRAM_START: u16 = 0x0801;
const READY_PROMPT: [u8; 6] = [0x12, 0x05, 0x01, 0x04, 0x19, 0x2e];
const DIRECTORY_TITLE: &[u8] = b"1541-TESTSUITE";
const DIRECTORY_ENTRY: &[u8] = b"&00>DUMMY-TRUE0";
const LOAD_DIRECTORY_COMMAND: &[u8] = b"LOAD\"$\",8\r";

#[derive(Serialize)]
struct DirectoryReport {
    boot_frames: u32,
    directory_end_address: u16,
    directory_entry_found: bool,
    directory_frames: u32,
    directory_title_found: bool,
    drive_elapsed_cycles: u64,
    drive_half_track: u8,
    drive_lead_cycles: i128,
    drive_program_counter: u16,
    drive_target_cycles: u64,
    system_cycles: u64,
}

struct Arguments {
    basic_path: PathBuf,
    character_path: PathBuf,
    kernal_path: PathBuf,
    drive_rom_path: PathBuf,
    disk_path: PathBuf,
    scenario: String,
}

struct KeyboardFeeder<'a> {
    command: &'a [u8],
    offset: usize,
}

impl<'a> KeyboardFeeder<'a> {
    const fn new(command: &'a [u8]) -> Self {
        Self { command, offset: 0 }
    }

    fn finished(&self) -> bool {
        self.offset == self.command.len()
    }

    fn refill(&mut self, core: &mut C64Core) -> Result<(), io::Error> {
        let used = core.read_base_ram(KEYBOARD_BUFFER_COUNT);
        let capacity = core.read_base_ram(KEYBOARD_BUFFER_CAPACITY);
        if capacity == 0 {
            return Err(invalid_data("C64 KERNAL keyboard buffer has zero capacity"));
        }
        if used > capacity {
            return Err(invalid_data(format!(
                "C64 keyboard buffer count {used} exceeds capacity {capacity}"
            )));
        }
        let available = usize::from(capacity - used);
        let remaining = self.command.len() - self.offset;
        let write_count = available.min(remaining);
        for index in 0..write_count {
            let value = self.command[self.offset + index];
            let index_offset = u16::try_from(index)
                .map_err(|_| invalid_data("keyboard command offset exceeds 16 bits"))?;
            let address = KEYBOARD_BUFFER_START + u16::from(used) + index_offset;
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        let write_count_byte = u8::try_from(write_count)
            .map_err(|_| invalid_data("keyboard refill count exceeds one byte"))?;
        core.write_base_ram(
            KEYBOARD_BUFFER_COUNT,
            used + write_count_byte,
            MemoryWriteSource::HostLoader,
        );
        self.offset += write_count;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    if arguments.scenario != "directory" {
        return Err(invalid_input(format!(
            "unsupported VICE drive scenario {}",
            arguments.scenario
        ))
        .into());
    }
    let firmware = C64Firmware::new(
        &fs::read(arguments.basic_path)?,
        &fs::read(arguments.character_path)?,
        &fs::read(arguments.kernal_path)?,
    )?;
    let drive_rom = fs::read(arguments.drive_rom_path)?;
    let disk = D64DiskImage::parse(&fs::read(arguments.disk_path)?, false)?;
    let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);

    // Match the production construction order: reset the C64 first, then
    // attach and power the independently clocked drive.
    core.reset()?;
    core.attach_drive1541(8, &drive_rom)?;
    core.drive1541_mut()?
        .mount_disk(Drive1541DiskImage::D64(disk))?;

    let boot_frames = boot_to_basic_ready(&mut core)?;
    let directory_frames = run_basic_command(
        &mut core,
        LOAD_DIRECTORY_COMMAND,
        "LOAD\"$\",8",
        DRIVE_COMMAND_FRAME_LIMIT,
    )?;
    let directory_end_address = read_ram_word(&core, BASIC_TEXT_END_POINTER);
    let drive = core
        .devices()
        .drive1541()
        .ok_or_else(|| invalid_data("the VICE drive was detached during the reference run"))?;
    let report = DirectoryReport {
        boot_frames,
        directory_end_address,
        directory_entry_found: find_ram_sequence(&core, DIRECTORY_ENTRY),
        directory_frames,
        directory_title_found: find_ram_sequence(&core, DIRECTORY_TITLE),
        drive_elapsed_cycles: drive.machine().elapsed_cycles(),
        drive_half_track: drive.machine().mechanism().current_half_track(),
        drive_lead_cycles: drive.clock().lead_cycles(drive.machine()),
        drive_program_counter: drive.machine().cpu().state().program_counter,
        drive_target_cycles: drive.clock().target_cycles(),
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
    let drive_rom_path = required_path(&mut arguments, "1541 DOS ROM path")?;
    let disk_path = required_path(&mut arguments, "D64 path")?;
    let scenario = arguments
        .next()
        .ok_or_else(|| invalid_input("missing VICE drive scenario"))?
        .into_string()
        .map_err(|_| invalid_input("VICE drive scenario is not valid Unicode"))?;
    if arguments.next().is_some() {
        return Err(invalid_input("unexpected extra arguments"));
    }
    Ok(Arguments {
        basic_path,
        character_path,
        kernal_path,
        drive_rom_path,
        disk_path,
        scenario,
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

fn boot_to_basic_ready(core: &mut C64Core) -> Result<u32, Box<dyn Error>> {
    let mut ready_was_absent = !has_basic_ready_prompt(core);
    for frame in 1..=BASIC_BOOT_FRAME_LIMIT {
        run_frame(core)?;
        if !has_basic_ready_prompt(core) {
            ready_was_absent = true;
        } else if ready_was_absent {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("C64 BASIC did not reach READY within {BASIC_BOOT_FRAME_LIMIT} PAL frames"),
    )
    .into())
}

fn run_basic_command(
    core: &mut C64Core,
    command: &[u8],
    label: &str,
    frame_limit: u32,
) -> Result<u32, Box<dyn Error>> {
    for address in SCREEN_START..SCREEN_END_EXCLUSIVE {
        core.write_base_ram(address, SCREEN_SPACE, MemoryWriteSource::HostLoader);
    }
    let mut feeder = KeyboardFeeder::new(command);
    for frame in 1..=frame_limit {
        feeder.refill(core)?;
        run_frame(core)?;
        reject_jammed_cpus(core, label)?;
        let keyboard_empty = core.read_base_ram(KEYBOARD_BUFFER_COUNT) == 0;
        if feeder.finished() && keyboard_empty && has_basic_ready_prompt(core) {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{label} did not return to BASIC READY within {frame_limit} PAL frames"),
    )
    .into())
}

fn run_frame(core: &mut C64Core) -> Result<(), Box<dyn Error>> {
    let target_generation = core.devices().vic().frame_generation().saturating_add(1);
    for _ in 0..MAXIMUM_CPU_SLOTS_PER_FRAME {
        core.run_cpu_slots(1)?;
        reject_jammed_cpus(core, "PAL frame")?;
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

fn reject_jammed_cpus(core: &C64Core, label: &str) -> Result<(), io::Error> {
    if core.cpu_is_jammed() {
        return Err(invalid_data(format!(
            "{label} entered the C64 6510 JAM state at ${:04x}",
            core.cpu_state().program_counter
        )));
    }
    let drive = core
        .devices()
        .drive1541()
        .ok_or_else(|| invalid_data(format!("{label} detached the 1541")))?;
    if drive.machine().cpu().is_jammed() {
        return Err(invalid_data(format!(
            "{label} entered the 1541 6502 JAM state at ${:04x}",
            drive.machine().cpu().state().program_counter
        )));
    }
    Ok(())
}

fn has_basic_ready_prompt(core: &C64Core) -> bool {
    let final_start = SCREEN_END_EXCLUSIVE - 6;
    (SCREEN_START..=final_start).any(|address| {
        READY_PROMPT
            .iter()
            .copied()
            .zip(0_u16..)
            .all(|(expected, offset)| core.read_base_ram(address + offset) == expected)
    })
}

fn find_ram_sequence(core: &C64Core, sequence: &[u8]) -> bool {
    let end_exclusive = 0x4000_u16;
    let Ok(sequence_length) = u16::try_from(sequence.len()) else {
        return false;
    };
    let Some(final_start) = end_exclusive.checked_sub(sequence_length) else {
        return false;
    };
    (BASIC_PROGRAM_START..=final_start).any(|address| {
        sequence
            .iter()
            .copied()
            .zip(0_u16..)
            .all(|(expected, offset)| core.read_base_ram(address + offset) == expected)
    })
}

fn read_ram_word(core: &C64Core, address: u16) -> u16 {
    u16::from_le_bytes([
        core.read_base_ram(address),
        core.read_base_ram(address.wrapping_add(1)),
    ])
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
