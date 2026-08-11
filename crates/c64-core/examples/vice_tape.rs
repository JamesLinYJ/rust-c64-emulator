// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE Commodore 1530 end-to-end reference runner
//
//   File:       vice_tape.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use c64_core::devices::tape::DatasetteTape;
use c64_core::media::tap::{TapImage, TapVideoStandard, WritableTapImage};
use c64_core::{C64Core, C64Firmware, CoreConfig, MemoryWriteSource};
use serde::Serialize;
use sha2::{Digest, Sha256};

const BASIC_BOOT_FRAME_LIMIT: u32 = 300;
const BASIC_LINE_ENTRY_FRAME_LIMIT: u32 = 300;
const TAPE_LOAD_FRAME_LIMIT: u32 = 1_800;
const TAPE_SAVE_FRAME_LIMIT: u32 = 1_800;
const VICE_TAPE_WRITE_FRAME_LIMIT: u32 = 300;
const MAXIMUM_CPU_SLOTS_PER_FRAME: u64 = 100_000;
const KEYBOARD_BUFFER_START: u16 = 0x0277;
const KEYBOARD_BUFFER_CAPACITY: u16 = 0x0289;
const KEYBOARD_BUFFER_COUNT: u16 = 0x00c6;
const SCREEN_START: u16 = 0x0400;
const SCREEN_END_EXCLUSIVE: u16 = 0x07e8;
const SCREEN_SPACE: u8 = 0x20;
const KERNAL_IO_STATUS: u16 = 0x0090;
const KERNAL_LOAD_END_POINTER: u16 = 0x00ae;
const BASIC_TEXT_END_POINTER: u16 = 0x002d;
const BASIC_PROGRAM_START: u16 = 0x0801;
const READY_PROMPT: [u8; 6] = [0x12, 0x05, 0x01, 0x04, 0x19, 0x2e];
const LOAD_TAPE_COMMAND: &[u8] = b"LOAD\"CODEX TAPE\",1,1\r";
const ENTER_SAVE_PROGRAM_COMMAND: &[u8] = b"10 PRINT\"CODEX\"\r";
const SAVE_TAPE_COMMAND: &[u8] = b"SAVE\"CODEX SAVE\",1\r";
const LOAD_SAVED_TAPE_COMMAND: &[u8] = b"LOAD\"CODEX SAVE\",1\r";
const BASIC_RUN_COMMAND: &[u8] = b"RUN\r";
const TAPE_FILE_LOAD_ADDRESS: u16 = 0xc000;
const TAPE_FILE_PAYLOAD_LENGTH: usize = 64;
const EXPECTED_VICE_WRITE_PULSES: [u32; 4] = [0x20 * 8, 0x40 * 8, 0x60 * 8, 0x40 * 8];
const EXPECTED_SAVED_BASIC_FILE: [u8; 17] = [
    0x01, 0x08, 0x0e, 0x08, 0x0a, 0x00, 0x99, 0x22, 0x43, 0x4f, 0x44, 0x45, 0x58, 0x22, 0x00, 0x00,
    0x00,
];

#[derive(Serialize)]
struct LoadReport {
    boot_frames: u32,
    end_address: u16,
    frames: u32,
    pulse_count: usize,
    status: u8,
}

#[derive(Serialize)]
struct ViceWriteReport {
    frames: u32,
    initial_pause_cycles: u32,
    serialized_sha256: String,
    tail_pulses: Vec<u32>,
}

#[derive(Serialize)]
struct SaveReport {
    line_entry_frames: u32,
    load_frames: u32,
    pulse_count: usize,
    save_frames: u32,
    serialized_sha256: String,
}

#[derive(Serialize)]
struct TapeReport {
    load: LoadReport,
    save: SaveReport,
    vice_write: ViceWriteReport,
}

struct Arguments {
    basic_rom: PathBuf,
    character_rom: PathBuf,
    kernal_rom: PathBuf,
    load_tap: PathBuf,
    vice_write_prg: PathBuf,
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
        let write_count = usize::from(capacity - used).min(self.command.len() - self.offset);
        for index in 0..write_count {
            let address = KEYBOARD_BUFFER_START
                + u16::from(used)
                + u16::try_from(index)
                    .map_err(|_| invalid_data("keyboard command offset exceeds 16 bits"))?;
            core.write_base_ram(
                address,
                self.command[self.offset + index],
                MemoryWriteSource::HostLoader,
            );
        }
        core.write_base_ram(
            KEYBOARD_BUFFER_COUNT,
            used + u8::try_from(write_count)
                .map_err(|_| invalid_data("keyboard refill count exceeds one byte"))?,
            MemoryWriteSource::HostLoader,
        );
        self.offset += write_count;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    let firmware = C64Firmware::new(
        &fs::read(arguments.basic_rom)?,
        &fs::read(arguments.character_rom)?,
        &fs::read(arguments.kernal_rom)?,
    )?;
    let load_tape = TapImage::parse(&fs::read(arguments.load_tap)?, None)?;
    let vice_write_program = fs::read(arguments.vice_write_prg)?;

    let report = TapeReport {
        load: verify_kernal_load(firmware.clone(), load_tape)?,
        save: verify_kernal_save_round_trip(firmware.clone())?,
        vice_write: verify_vice_write_waveform(firmware, &vice_write_program)?,
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn verify_kernal_load(firmware: C64Firmware, tape: TapImage) -> Result<LoadReport, Box<dyn Error>> {
    let mut core = new_reset_core(firmware)?;
    let boot_frames = boot_to_basic_ready(&mut core)?;
    for offset in 0..TAPE_FILE_PAYLOAD_LENGTH {
        core.write_base_ram(
            TAPE_FILE_LOAD_ADDRESS
                + u16::try_from(offset)
                    .map_err(|_| invalid_data("tape payload offset exceeds 16 bits"))?,
            0xa5,
            MemoryWriteSource::HostLoader,
        );
    }
    core.datasette_mut()?
        .insert_tape(DatasetteTape::ReadOnly(tape))?;
    core.datasette_mut()?.press_play();
    let frames = run_basic_command(
        &mut core,
        LOAD_TAPE_COMMAND,
        "KERNAL tape LOAD fixture",
        TAPE_LOAD_FRAME_LIMIT,
    )?;
    for offset in 0..TAPE_FILE_PAYLOAD_LENGTH {
        let expected = u8::try_from(offset)
            .map_err(|_| invalid_data("tape payload offset exceeds one byte"))?
            .wrapping_mul(73)
            .wrapping_add(0x35);
        let address = TAPE_FILE_LOAD_ADDRESS
            + u16::try_from(offset)
                .map_err(|_| invalid_data("tape payload offset exceeds 16 bits"))?;
        let actual = core.read_base_ram(address);
        if actual != expected {
            return Err(invalid_data(format!(
                "KERNAL tape LOAD mismatch at ${address:04x}: received ${actual:02x}, expected ${expected:02x}"
            ))
            .into());
        }
    }
    let end_address = read_ram_word(&core, KERNAL_LOAD_END_POINTER);
    let expected_end_address = TAPE_FILE_LOAD_ADDRESS
        + u16::try_from(TAPE_FILE_PAYLOAD_LENGTH)
            .map_err(|_| invalid_data("tape payload length exceeds 16 bits"))?;
    if end_address != expected_end_address {
        return Err(invalid_data(format!(
            "KERNAL tape LOAD ended at ${end_address:04x}; expected ${expected_end_address:04x}"
        ))
        .into());
    }
    let status = core.read_base_ram(KERNAL_IO_STATUS);
    if status != 0 {
        return Err(invalid_data(format!("KERNAL tape LOAD returned status ${status:02x}")).into());
    }
    let pulse_count = core.devices().datasette().pulse_index();
    if pulse_count == 0 {
        return Err(invalid_data("KERNAL tape LOAD consumed no physical READ pulses").into());
    }
    Ok(LoadReport {
        boot_frames,
        end_address,
        frames,
        pulse_count,
        status,
    })
}

fn verify_vice_write_waveform(
    firmware: C64Firmware,
    program: &[u8],
) -> Result<ViceWriteReport, Box<dyn Error>> {
    let mut core = new_reset_core(firmware)?;
    boot_to_basic_ready(&mut core)?;
    core.datasette_mut()?
        .insert_tape(DatasetteTape::Writable(WritableTapImage::new(
            TapVideoStandard::Pal,
        )))?;
    core.datasette_mut()?.press_record()?;
    install_basic_run_prg(&mut core, program)?;

    let mut completed_frame = None;
    for frame in 1..=VICE_TAPE_WRITE_FRAME_LIMIT {
        run_frame(&mut core)?;
        reject_jammed_cpu(&core, "VICE tape WRITE reference")?;
        let pulse_count = core
            .devices()
            .datasette()
            .mounted_tape()
            .map_or(0, |tape| tape.pulses().len());
        if pulse_count > EXPECTED_VICE_WRITE_PULSES.len() {
            completed_frame = Some(frame);
            break;
        }
    }
    let frames = completed_frame.ok_or_else(|| {
        invalid_data(format!(
            "VICE tape WRITE reference did not emit five pulses within {VICE_TAPE_WRITE_FRAME_LIMIT} PAL frames"
        ))
    })?;
    core.datasette_mut()?.press_stop();
    let tape = core.datasette_mut()?.eject_tape()?;
    let pulses = tape
        .pulses()
        .iter()
        .map(|pulse| pulse.source_cycles)
        .collect::<Vec<_>>();
    if pulses.len() != EXPECTED_VICE_WRITE_PULSES.len() + 1 {
        return Err(invalid_data(format!(
            "VICE tape WRITE reference emitted {} pulses instead of five",
            pulses.len()
        ))
        .into());
    }
    let initial_pause_cycles = pulses[0];
    if initial_pause_cycles < TapVideoStandard::Pal.source_clock_hz() {
        return Err(invalid_data(format!(
            "VICE tape WRITE initial pause lasted {initial_pause_cycles} cycles; expected over one second"
        ))
        .into());
    }
    let tail_pulses = pulses[1..].to_vec();
    if tail_pulses != EXPECTED_VICE_WRITE_PULSES {
        return Err(invalid_data(format!(
            "VICE tape WRITE tail pulses changed to {tail_pulses:?}"
        ))
        .into());
    }
    let serialized = tape.to_bytes()?;
    let reparsed = TapImage::parse(&serialized, None)?;
    let reparsed_tail = reparsed
        .pulses()
        .iter()
        .skip(1)
        .map(|pulse| pulse.source_cycles)
        .collect::<Vec<_>>();
    if reparsed_tail != EXPECTED_VICE_WRITE_PULSES {
        return Err(invalid_data("serialized VICE tape WRITE pulses changed").into());
    }
    Ok(ViceWriteReport {
        frames,
        initial_pause_cycles,
        serialized_sha256: sha256_hex(&serialized),
        tail_pulses,
    })
}

fn verify_kernal_save_round_trip(firmware: C64Firmware) -> Result<SaveReport, Box<dyn Error>> {
    let mut save_core = new_reset_core(firmware.clone())?;
    boot_to_basic_ready(&mut save_core)?;
    let line_entry_frames = enter_basic_program_line(&mut save_core)?;
    save_core
        .datasette_mut()?
        .insert_tape(DatasetteTape::Writable(WritableTapImage::new(
            TapVideoStandard::Pal,
        )))?;
    save_core.datasette_mut()?.press_record()?;
    let save_frames = run_basic_command(
        &mut save_core,
        SAVE_TAPE_COMMAND,
        "KERNAL tape SAVE",
        TAPE_SAVE_FRAME_LIMIT,
    )?;
    if save_core.read_base_ram(KERNAL_IO_STATUS) != 0 {
        return Err(invalid_data("KERNAL tape SAVE returned a nonzero status").into());
    }
    save_core.datasette_mut()?.press_stop();
    let saved_tape = save_core.datasette_mut()?.eject_tape()?;
    let pulse_count = saved_tape.pulses().len();
    if pulse_count == 0 {
        return Err(invalid_data("KERNAL tape SAVE recorded no physical WRITE pulses").into());
    }
    let serialized = saved_tape.to_bytes()?;
    let serialized_sha256 = sha256_hex(&serialized);

    let mut load_core = new_reset_core(firmware)?;
    boot_to_basic_ready(&mut load_core)?;
    for address in BASIC_PROGRAM_START
        ..BASIC_PROGRAM_START
            + u16::try_from(EXPECTED_SAVED_BASIC_FILE.len())
                .map_err(|_| invalid_data("saved BASIC fixture exceeds 16 bits"))?
    {
        load_core.write_base_ram(address, 0xa5, MemoryWriteSource::HostLoader);
    }
    load_core
        .datasette_mut()?
        .insert_tape(DatasetteTape::ReadOnly(TapImage::parse(&serialized, None)?))?;
    load_core.datasette_mut()?.press_play();
    let load_frames = run_basic_command(
        &mut load_core,
        LOAD_SAVED_TAPE_COMMAND,
        "KERNAL saved-tape LOAD",
        TAPE_LOAD_FRAME_LIMIT,
    )?;
    if !basic_program_matches_memory(&load_core) {
        return Err(invalid_data("saved BASIC LOAD did not preserve the tokenized program").into());
    }
    if load_core.read_base_ram(KERNAL_IO_STATUS) != 0 {
        return Err(invalid_data("saved BASIC LOAD returned a nonzero status").into());
    }
    Ok(SaveReport {
        line_entry_frames,
        load_frames,
        pulse_count,
        save_frames,
        serialized_sha256,
    })
}

fn new_reset_core(firmware: C64Firmware) -> Result<C64Core, Box<dyn Error>> {
    let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);
    core.reset()?;
    Ok(core)
}

fn install_basic_run_prg(core: &mut C64Core, program: &[u8]) -> Result<(), io::Error> {
    let [low, high, payload @ ..] = program else {
        return Err(invalid_data("VICE tape WRITE PRG has no load address"));
    };
    let load_address = u16::from_le_bytes([*low, *high]);
    if load_address != BASIC_PROGRAM_START || payload.is_empty() {
        return Err(invalid_data(
            "VICE tape WRITE PRG is not a BASIC program at $0801",
        ));
    }
    let payload_length = u16::try_from(payload.len())
        .map_err(|_| invalid_data("VICE tape WRITE PRG exceeds 64 KiB"))?;
    let end_address = load_address
        .checked_add(payload_length)
        .ok_or_else(|| invalid_data("VICE tape WRITE PRG range overflow"))?;
    for (value, offset) in payload.iter().copied().zip(0_u16..) {
        core.write_base_ram(load_address + offset, value, MemoryWriteSource::HostLoader);
    }
    for pointer in [0x002b, 0x00ac] {
        write_ram_word(core, pointer, load_address);
    }
    for pointer in [0x002d, 0x002f, 0x0031, 0x00ae] {
        write_ram_word(core, pointer, end_address);
    }
    let used = core.read_base_ram(KEYBOARD_BUFFER_COUNT);
    let capacity = core.read_base_ram(KEYBOARD_BUFFER_CAPACITY);
    let command_length = u8::try_from(BASIC_RUN_COMMAND.len())
        .map_err(|_| invalid_data("BASIC RUN command exceeds one byte"))?;
    if used.saturating_add(command_length) > capacity {
        return Err(invalid_data("C64 keyboard buffer cannot accept BASIC RUN"));
    }
    for (value, offset) in BASIC_RUN_COMMAND.iter().copied().zip(0_u16..) {
        core.write_base_ram(
            KEYBOARD_BUFFER_START + u16::from(used) + offset,
            value,
            MemoryWriteSource::HostLoader,
        );
    }
    core.write_base_ram(
        KEYBOARD_BUFFER_COUNT,
        used + command_length,
        MemoryWriteSource::HostLoader,
    );
    Ok(())
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
        reject_jammed_cpu(core, label)?;
        if feeder.finished()
            && core.read_base_ram(KEYBOARD_BUFFER_COUNT) == 0
            && has_basic_ready_prompt(core)
        {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{label} did not return to BASIC READY within {frame_limit} PAL frames"),
    )
    .into())
}

fn enter_basic_program_line(core: &mut C64Core) -> Result<u32, Box<dyn Error>> {
    let mut feeder = KeyboardFeeder::new(ENTER_SAVE_PROGRAM_COMMAND);
    for frame in 1..=BASIC_LINE_ENTRY_FRAME_LIMIT {
        feeder.refill(core)?;
        run_frame(core)?;
        reject_jammed_cpu(core, "BASIC tape SAVE line entry")?;
        if feeder.finished()
            && core.read_base_ram(KEYBOARD_BUFFER_COUNT) == 0
            && basic_program_matches_memory(core)
        {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "BASIC tape SAVE line entry did not complete within {BASIC_LINE_ENTRY_FRAME_LIMIT} PAL frames"
        ),
    )
    .into())
}

fn run_frame(core: &mut C64Core) -> Result<(), Box<dyn Error>> {
    let target_generation = core.devices().vic().frame_generation().saturating_add(1);
    for _ in 0..MAXIMUM_CPU_SLOTS_PER_FRAME {
        core.run_cpu_slots(1)?;
        reject_jammed_cpu(core, "PAL frame")?;
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

fn reject_jammed_cpu(core: &C64Core, label: &str) -> Result<(), io::Error> {
    if core.cpu_is_jammed() {
        return Err(invalid_data(format!(
            "{label} entered the C64 6510 JAM state at ${:04x}",
            core.cpu_state().program_counter
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

fn basic_program_matches_memory(core: &C64Core) -> bool {
    let start = u16::from_le_bytes([EXPECTED_SAVED_BASIC_FILE[0], EXPECTED_SAVED_BASIC_FILE[1]]);
    let payload = &EXPECTED_SAVED_BASIC_FILE[2..];
    let Ok(payload_length) = u16::try_from(payload.len()) else {
        return false;
    };
    read_ram_word(core, BASIC_TEXT_END_POINTER) == start.saturating_add(payload_length)
        && payload
            .iter()
            .copied()
            .zip(0_u16..)
            .all(|(expected, offset)| core.read_base_ram(start + offset) == expected)
}

fn write_ram_word(core: &mut C64Core, address: u16, value: u16) {
    let [low, high] = value.to_le_bytes();
    core.write_base_ram(address, low, MemoryWriteSource::HostLoader);
    core.write_base_ram(address + 1, high, MemoryWriteSource::HostLoader);
}

fn read_ram_word(core: &C64Core, address: u16) -> u16 {
    u16::from_le_bytes([
        core.read_base_ram(address),
        core.read_base_ram(address.wrapping_add(1)),
    ])
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
        load_tap: required_path(&mut arguments, "KERNAL LOAD TAP path")?,
        vice_write_prg: required_path(&mut arguments, "VICE tape WRITE PRG path")?,
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
