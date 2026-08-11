// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE Commodore 1541 end-to-end reference runner
//
//   File:       vice_drive.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use c64_core::devices::drive1541::mechanism::Drive1541DiskImage;
use c64_core::media::d64::D64DiskImage;
use c64_core::{C64Core, C64Firmware, CoreConfig, CpuBusAccessKind, MemoryWriteSource};
use serde::Serialize;
use sha2::{Digest, Sha256};

const BASIC_BOOT_FRAME_LIMIT: u32 = 300;
const DRIVE_COMMAND_FRAME_LIMIT: u32 = 600;
const DRIVE_FORMAT_FRAME_LIMIT: u32 = 6_000;
const MAXIMUM_CPU_SLOTS_PER_FRAME: u64 = 100_000;
const SCREEN_START: u16 = 0x0400;
const SCREEN_END_EXCLUSIVE: u16 = 0x07e8;
const SCREEN_SPACE: u8 = 0x20;
const KEYBOARD_BUFFER_START: u16 = 0x0277;
const KEYBOARD_BUFFER_CAPACITY: u16 = 0x0289;
const KEYBOARD_BUFFER_COUNT: u16 = 0x00c6;
const BASIC_TEXT_END_POINTER: u16 = 0x002d;
const KERNAL_LOAD_END_POINTER: u16 = 0x00ae;
const BASIC_PROGRAM_START: u16 = 0x0801;
const READY_PROMPT: [u8; 6] = [0x12, 0x05, 0x01, 0x04, 0x19, 0x2e];
const DIRECTORY_TITLE: &[u8] = b"1541-TESTSUITE";
const DIRECTORY_ENTRY: &[u8] = b"&00>DUMMY-TRUE0";
const LOAD_DIRECTORY_COMMAND: &[u8] = b"LOAD\"$\",8\r";
const LOAD_FIRST_FILE_COMMAND: &[u8] = b"LOAD\"*\",8,1\r";
const NEW_COMMAND: &[u8] = b"NEW\r";
const ENTER_SAVE_PROGRAM_COMMAND: &[u8] = b"10 PRINT\"CODEX\"\r";
const SAVE_TEST_FILE_COMMAND: &[u8] = b"SAVE\"CODEX\",8\r";
const LOAD_SAVED_FILE_COMMAND: &[u8] = b"LOAD\"CODEX\",8\r";
const LOAD_FORMAT_TEST_COMMAND: &[u8] = b"LOAD\"FORMAT\",8\r";
const RUN_COMMAND: &[u8] = b"RUN\r";
const SAVED_TEST_FILE_NAME: &[u8] = b"CODEX";
const FORMAT_TEST_FILE_NAME: &[u8] = b"FORMAT";
const VICE_TEST_RESULT_ADDRESS: u16 = 0xd7ff;
const EXPECTED_SAVED_BASIC_FILE: [u8; 17] = [
    0x01, 0x08, 0x0e, 0x08, 0x0a, 0x00, 0x99, 0x22, 0x43, 0x4f, 0x44, 0x45, 0x58, 0x22, 0x00, 0x00,
    0x00,
];
const D64_DIRECTORY_TRACK: u8 = 18;
const D64_DIRECTORY_FIRST_SECTOR: u8 = 1;
const D64_DIRECTORY_ENTRY_COUNT: usize = 8;
const D64_DIRECTORY_ENTRY_SIZE: usize = 0x20;
const D64_DIRECTORY_FILE_TYPE_OFFSET: usize = 2;
const D64_DIRECTORY_FIRST_TRACK_OFFSET: usize = 3;
const D64_DIRECTORY_FIRST_SECTOR_OFFSET: usize = 4;
const D64_DIRECTORY_FILE_NAME_OFFSET: usize = 5;
const D64_DIRECTORY_FILE_NAME_LENGTH: usize = 16;
const D64_DIRECTORY_FILE_NAME_PADDING: u8 = 0xa0;

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

#[derive(Serialize)]
struct LoadReport {
    boot_frames: u32,
    directory_frames: u32,
    drive_elapsed_cycles: u64,
    drive_lead_cycles: i128,
    drive_target_cycles: u64,
    load_address: u16,
    load_end_address: u16,
    load_frames: u32,
    loaded_file_sha256: String,
    loaded_payload_length: u16,
}

#[derive(Serialize)]
struct SaveReport {
    boot_frames: u32,
    committed_half_tracks: Vec<u8>,
    directory_frames: u32,
    drive_elapsed_cycles: u64,
    drive_lead_cycles: i128,
    drive_target_cycles: u64,
    first_file_load_frames: u32,
    program_entry_frames: u32,
    reload_end_address: u16,
    reload_frames: u32,
    reloaded_file_sha256: String,
    save_frames: u32,
    saved_file_sha256: String,
}

#[derive(Serialize)]
struct FormatReport {
    boot_frames: u32,
    committed_track_count: usize,
    drive_elapsed_cycles: u64,
    drive_lead_cycles: i128,
    drive_target_cycles: u64,
    load_frames: u32,
    program_byte_length: usize,
    program_preserved: bool,
    result_code: u8,
    run_frames: u32,
}

#[derive(Clone, Copy)]
enum Scenario {
    Directory,
    Format,
    Load,
    Save,
}

impl Scenario {
    fn parse(value: &str) -> Result<Self, io::Error> {
        match value {
            "directory" => Ok(Self::Directory),
            "format" => Ok(Self::Format),
            "load" => Ok(Self::Load),
            "save" => Ok(Self::Save),
            _ => Err(invalid_input(format!(
                "unsupported VICE drive scenario {value}"
            ))),
        }
    }
}

struct Arguments {
    basic_path: PathBuf,
    character_path: PathBuf,
    kernal_path: PathBuf,
    drive_rom_path: PathBuf,
    disk_path: PathBuf,
    scenario: Scenario,
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
    let firmware = C64Firmware::new(
        &fs::read(arguments.basic_path)?,
        &fs::read(arguments.character_path)?,
        &fs::read(arguments.kernal_path)?,
    )?;
    let drive_rom = fs::read(arguments.drive_rom_path)?;
    let disk = D64DiskImage::parse(&fs::read(arguments.disk_path)?, false)?;
    let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);

    // 冷启动先连接整套硬件，再由同一次板级 RESET 建立主机与 drive 的共同
    // 时钟纪元；运行中的显式热插拔仍从插入时刻建立独立相位。
    core.attach_drive1541(8, &drive_rom)?;
    core.reset()?;
    core.drive1541_mut()?
        .mount_disk(Drive1541DiskImage::D64(disk))?;

    let boot_frames = boot_to_basic_ready(&mut core)?;
    match arguments.scenario {
        Scenario::Directory => {
            let directory_frames = load_directory(&mut core)?;
            report_directory(&core, boot_frames, directory_frames)?;
        }
        Scenario::Format => report_format(&mut core, boot_frames)?,
        Scenario::Load => {
            let directory_frames = load_directory(&mut core)?;
            report_first_file_load(&mut core, boot_frames, directory_frames)?;
        }
        Scenario::Save => {
            let directory_frames = load_directory(&mut core)?;
            report_save(&mut core, boot_frames, directory_frames)?;
        }
    }
    Ok(())
}

fn report_format(core: &mut C64Core, boot_frames: u32) -> Result<(), Box<dyn Error>> {
    let expected_program = mounted_d64_file(core, FORMAT_TEST_FILE_NAME, "format setup")?;
    let load_frames = run_basic_command(
        core,
        LOAD_FORMAT_TEST_COMMAND,
        "LOAD\"FORMAT\",8",
        DRIVE_COMMAND_FRAME_LIMIT,
    )?;
    let (run_frames, result_code) = run_basic_command_observing_cpu_write(
        core,
        RUN_COMMAND,
        "VICE drive/format/format.prg",
        DRIVE_FORMAT_FRAME_LIMIT,
        VICE_TEST_RESULT_ADDRESS,
    )?;
    let result_code = result_code.ok_or_else(|| {
        invalid_data("VICE drive format test returned without reporting at $d7ff")
    })?;
    let committed_track_count = {
        let report = core
            .drive1541_mut()?
            .machine_mut()
            .mechanism_mut()
            .commit_raw_track_writes_to_d64()?;
        if !report.failures.is_empty() || !report.remaining_dirty_half_tracks.is_empty() {
            return Err(invalid_data(format!(
                "VICE drive format left incompatible D64 raw tracks: {report:?}"
            ))
            .into());
        }
        report.committed_half_tracks.len()
    };
    let reformatted_program = mounted_d64_file(core, FORMAT_TEST_FILE_NAME, "format result")?;
    let drive = core
        .devices()
        .drive1541()
        .ok_or_else(|| invalid_data("format test detached the 1541"))?;
    let report = FormatReport {
        boot_frames,
        committed_track_count,
        drive_elapsed_cycles: drive.machine().elapsed_cycles(),
        drive_lead_cycles: drive.clock().lead_cycles(drive.machine()),
        drive_target_cycles: drive.clock().target_cycles(),
        load_frames,
        program_byte_length: reformatted_program.len(),
        program_preserved: reformatted_program == expected_program,
        result_code,
        run_frames,
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn load_directory(core: &mut C64Core) -> Result<u32, Box<dyn Error>> {
    run_basic_command(
        core,
        LOAD_DIRECTORY_COMMAND,
        "LOAD\"$\",8",
        DRIVE_COMMAND_FRAME_LIMIT,
    )
}

fn report_directory(
    core: &C64Core,
    boot_frames: u32,
    directory_frames: u32,
) -> Result<(), Box<dyn Error>> {
    let drive = core
        .devices()
        .drive1541()
        .ok_or_else(|| invalid_data("the VICE drive was detached during the reference run"))?;
    let report = DirectoryReport {
        boot_frames,
        directory_end_address: read_ram_word(core, BASIC_TEXT_END_POINTER),
        directory_entry_found: find_ram_sequence(core, DIRECTORY_ENTRY),
        directory_frames,
        directory_title_found: find_ram_sequence(core, DIRECTORY_TITLE),
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

fn report_first_file_load(
    core: &mut C64Core,
    boot_frames: u32,
    directory_frames: u32,
) -> Result<(), Box<dyn Error>> {
    let load_frames = load_first_file(core)?;
    let load_end_address = read_ram_word(core, KERNAL_LOAD_END_POINTER);
    let loaded_payload_length = load_end_address
        .checked_sub(BASIC_PROGRAM_START)
        .ok_or_else(|| {
            invalid_data(format!(
                "KERNAL LOAD ended at ${load_end_address:04x}, before its $0801 start"
            ))
        })?;
    let loaded_file_sha256 = loaded_prg_sha256(core, BASIC_PROGRAM_START, load_end_address);
    let drive = core
        .devices()
        .drive1541()
        .ok_or_else(|| invalid_data("the VICE drive was detached during the reference run"))?;
    let report = LoadReport {
        boot_frames,
        directory_frames,
        drive_elapsed_cycles: drive.machine().elapsed_cycles(),
        drive_lead_cycles: drive.clock().lead_cycles(drive.machine()),
        drive_target_cycles: drive.clock().target_cycles(),
        load_address: BASIC_PROGRAM_START,
        load_end_address,
        load_frames,
        loaded_file_sha256,
        loaded_payload_length,
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn report_save(
    core: &mut C64Core,
    boot_frames: u32,
    directory_frames: u32,
) -> Result<(), Box<dyn Error>> {
    let first_file_load_frames = load_first_file(core)?;
    run_basic_command(
        core,
        NEW_COMMAND,
        "NEW before SAVE test",
        DRIVE_COMMAND_FRAME_LIMIT,
    )?;
    let program_entry_frames = enter_basic_program_line(core)?;
    let save_frames = run_basic_command(
        core,
        SAVE_TEST_FILE_COMMAND,
        "SAVE\"CODEX\",8",
        DRIVE_COMMAND_FRAME_LIMIT,
    )?;
    let committed_half_tracks = {
        let report = core
            .drive1541_mut()?
            .machine_mut()
            .mechanism_mut()
            .commit_raw_track_writes_to_d64()?;
        if !report.failures.is_empty() || !report.remaining_dirty_half_tracks.is_empty() {
            return Err(invalid_data(format!(
                "1541 SAVE left incompatible D64 raw tracks: {report:?}"
            ))
            .into());
        }
        if report.committed_half_tracks.is_empty() {
            return Err(invalid_data("1541 SAVE returned without writing a D64 track").into());
        }
        report.committed_half_tracks
    };
    let saved_file = {
        let drive = core
            .devices()
            .drive1541()
            .ok_or_else(|| invalid_data("SAVE detached the 1541"))?;
        let disk = drive
            .machine()
            .mechanism()
            .mounted_disk()
            .ok_or_else(|| invalid_data("SAVE ejected the D64"))?;
        let Drive1541DiskImage::D64(disk) = disk else {
            return Err(invalid_data("SAVE replaced the D64 with a G64").into());
        };
        extract_named_d64_file(disk, SAVED_TEST_FILE_NAME)?
    };
    if saved_file != EXPECTED_SAVED_BASIC_FILE {
        return Err(invalid_data("SAVE produced unexpected CODEX PRG bytes").into());
    }
    let saved_file_sha256 = sha256_hex(&saved_file);

    run_basic_command(
        core,
        NEW_COMMAND,
        "NEW before saved-file LOAD",
        DRIVE_COMMAND_FRAME_LIMIT,
    )?;
    let reload_frames = run_basic_command(
        core,
        LOAD_SAVED_FILE_COMMAND,
        "LOAD\"CODEX\",8",
        DRIVE_COMMAND_FRAME_LIMIT,
    )?;
    if !basic_program_matches_memory(core, &EXPECTED_SAVED_BASIC_FILE) {
        return Err(invalid_data("reloaded CODEX PRG does not match C64 BASIC memory").into());
    }
    let reload_end_address = read_ram_word(core, BASIC_TEXT_END_POINTER);
    let reloaded_file_sha256 = loaded_prg_sha256(core, BASIC_PROGRAM_START, reload_end_address);
    let drive = core
        .devices()
        .drive1541()
        .ok_or_else(|| invalid_data("saved-file LOAD detached the 1541"))?;
    let report = SaveReport {
        boot_frames,
        committed_half_tracks,
        directory_frames,
        drive_elapsed_cycles: drive.machine().elapsed_cycles(),
        drive_lead_cycles: drive.clock().lead_cycles(drive.machine()),
        drive_target_cycles: drive.clock().target_cycles(),
        first_file_load_frames,
        program_entry_frames,
        reload_end_address,
        reload_frames,
        reloaded_file_sha256,
        save_frames,
        saved_file_sha256,
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn load_first_file(core: &mut C64Core) -> Result<u32, Box<dyn Error>> {
    run_basic_command(
        core,
        LOAD_FIRST_FILE_COMMAND,
        "LOAD\"*\",8,1",
        DRIVE_COMMAND_FRAME_LIMIT,
    )
}

fn loaded_prg_sha256(core: &C64Core, start: u16, end: u16) -> String {
    let mut digest = Sha256::new();
    digest.update(start.to_le_bytes());
    for address in start..end {
        digest.update([core.read_base_ram(address)]);
    }
    format!("{:x}", digest.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
    let scenario = Scenario::parse(&scenario)?;
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

fn run_basic_command_observing_cpu_write(
    core: &mut C64Core,
    command: &[u8],
    label: &str,
    frame_limit: u32,
    observed_address: u16,
) -> Result<(u32, Option<u8>), Box<dyn Error>> {
    for address in SCREEN_START..SCREEN_END_EXCLUSIVE {
        core.write_base_ram(address, SCREEN_SPACE, MemoryWriteSource::HostLoader);
    }
    let mut feeder = KeyboardFeeder::new(command);
    let mut observed_value = None;
    for frame in 1..=frame_limit {
        feeder.refill(core)?;
        run_frame_observing_cpu_write(core, observed_address, &mut observed_value)?;
        reject_jammed_cpus(core, label)?;
        let keyboard_empty = core.read_base_ram(KEYBOARD_BUFFER_COUNT) == 0;
        if feeder.finished() && keyboard_empty && has_basic_ready_prompt(core) {
            return Ok((frame, observed_value));
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
    for frame in 1..=DRIVE_COMMAND_FRAME_LIMIT {
        feeder.refill(core)?;
        run_frame(core)?;
        reject_jammed_cpus(core, "BASIC line entry")?;
        let keyboard_empty = core.read_base_ram(KEYBOARD_BUFFER_COUNT) == 0;
        if feeder.finished()
            && keyboard_empty
            && basic_program_matches_memory(core, &EXPECTED_SAVED_BASIC_FILE)
        {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "BASIC line entry did not produce the fixed program within {DRIVE_COMMAND_FRAME_LIMIT} PAL frames"
        ),
    )
    .into())
}

fn basic_program_matches_memory(core: &C64Core, program: &[u8]) -> bool {
    let [start_low, start_high, payload @ ..] = program else {
        return false;
    };
    let start = u16::from_le_bytes([*start_low, *start_high]);
    let Ok(payload_length) = u16::try_from(payload.len()) else {
        return false;
    };
    if read_ram_word(core, BASIC_TEXT_END_POINTER) != start.saturating_add(payload_length) {
        return false;
    }
    payload
        .iter()
        .copied()
        .zip(0_u16..)
        .all(|(expected, offset)| core.read_base_ram(start + offset) == expected)
}

fn mounted_d64_file(
    core: &C64Core,
    file_name: &[u8],
    label: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let drive = core
        .devices()
        .drive1541()
        .ok_or_else(|| invalid_data(format!("{label} detached the 1541")))?;
    let disk = drive
        .machine()
        .mechanism()
        .mounted_disk()
        .ok_or_else(|| invalid_data(format!("{label} ejected the disk")))?;
    let Drive1541DiskImage::D64(disk) = disk else {
        return Err(invalid_data(format!("{label} requires a D64 image")).into());
    };
    extract_named_d64_file(disk, file_name)
}

fn extract_named_d64_file(
    disk: &D64DiskImage,
    expected_name: &[u8],
) -> Result<Vec<u8>, Box<dyn Error>> {
    if expected_name.len() > D64_DIRECTORY_FILE_NAME_LENGTH {
        return Err(invalid_input("D64 file name exceeds 16 bytes").into());
    }
    let mut visited = HashSet::new();
    let mut track = D64_DIRECTORY_TRACK;
    let mut sector = D64_DIRECTORY_FIRST_SECTOR;
    while track != 0 {
        if !visited.insert((track, sector)) {
            return Err(
                invalid_data(format!("D64 directory chain loops at {track}/{sector}")).into(),
            );
        }
        let directory_sector = disk.read_sector(track, sector)?;
        for entry_index in 0..D64_DIRECTORY_ENTRY_COUNT {
            let entry_offset = entry_index * D64_DIRECTORY_ENTRY_SIZE;
            if directory_sector[entry_offset + D64_DIRECTORY_FILE_TYPE_OFFSET] == 0
                || !d64_directory_name_matches(directory_sector, entry_offset, expected_name)
            {
                continue;
            }
            let first_track = directory_sector[entry_offset + D64_DIRECTORY_FIRST_TRACK_OFFSET];
            let first_sector = directory_sector[entry_offset + D64_DIRECTORY_FIRST_SECTOR_OFFSET];
            if first_track == 0 {
                return Err(invalid_data("D64 directory file has no first track").into());
            }
            return extract_d64_file(disk, first_track, first_sector);
        }
        track = directory_sector[0];
        sector = directory_sector[1];
    }
    Err(invalid_data(format!(
        "D64 directory does not contain {}",
        String::from_utf8_lossy(expected_name)
    ))
    .into())
}

fn d64_directory_name_matches(sector: &[u8], entry_offset: usize, expected: &[u8]) -> bool {
    (0..D64_DIRECTORY_FILE_NAME_LENGTH).all(|index| {
        let expected_byte = expected
            .get(index)
            .copied()
            .unwrap_or(D64_DIRECTORY_FILE_NAME_PADDING);
        sector[entry_offset + D64_DIRECTORY_FILE_NAME_OFFSET + index] == expected_byte
    })
}

fn extract_d64_file(
    disk: &D64DiskImage,
    mut track: u8,
    mut sector: u8,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut file = Vec::new();
    let mut visited = HashSet::new();
    while track != 0 {
        if !visited.insert((track, sector)) {
            return Err(invalid_data(format!("D64 file chain loops at {track}/{sector}")).into());
        }
        let file_sector = disk.read_sector(track, sector)?;
        let next_track = file_sector[0];
        let next_sector = file_sector[1];
        let data_length = if next_track == 0 {
            next_sector
                .checked_sub(1)
                .ok_or_else(|| invalid_data("D64 terminal sector has an invalid byte count"))?
        } else {
            254
        };
        let data_end = 2 + usize::from(data_length);
        file.extend_from_slice(&file_sector[2..data_end]);
        track = next_track;
        sector = next_sector;
    }
    Ok(file)
}

fn run_frame(core: &mut C64Core) -> Result<(), Box<dyn Error>> {
    run_frame_internal(core, None)
}

fn run_frame_observing_cpu_write(
    core: &mut C64Core,
    address: u16,
    observed_value: &mut Option<u8>,
) -> Result<(), Box<dyn Error>> {
    run_frame_internal(core, Some((address, observed_value)))
}

fn run_frame_internal(
    core: &mut C64Core,
    mut observer: Option<(u16, &mut Option<u8>)>,
) -> Result<(), Box<dyn Error>> {
    let target_generation = core.devices().vic().frame_generation().saturating_add(1);
    for _ in 0..MAXIMUM_CPU_SLOTS_PER_FRAME {
        let transaction_count = core.diagnostics().cpu_bus_transactions;
        core.run_cpu_slots(1)?;
        reject_jammed_cpus(core, "PAL frame")?;
        if let Some((observed_address, observed_value)) = observer.as_mut() {
            let diagnostics = core.diagnostics();
            if diagnostics.cpu_bus_transactions != transaction_count
                && let Some(transaction) = diagnostics.last_cpu_bus_transaction
                && transaction.kind == CpuBusAccessKind::Write
                && transaction.address == *observed_address
            {
                **observed_value = Some(transaction.value);
            }
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
