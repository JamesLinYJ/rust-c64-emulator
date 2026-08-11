// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Rust 1541 VICE end-to-end reference gate
//
//   File:       verifyRustDrive1541Reference.ts
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';

import { loadPinnedReferenceAsset } from './reference/loadPinnedReferenceAsset';

interface RustDirectoryReport {
  readonly boot_frames: number;
  readonly directory_end_address: number;
  readonly directory_entry_found: boolean;
  readonly directory_frames: number;
  readonly directory_title_found: boolean;
  readonly drive_elapsed_cycles: number;
  readonly drive_half_track: number;
  readonly drive_lead_cycles: number;
  readonly drive_program_counter: number;
  readonly drive_target_cycles: number;
  readonly system_cycles: number;
}

interface RustLoadReport {
  readonly boot_frames: number;
  readonly directory_frames: number;
  readonly drive_elapsed_cycles: number;
  readonly drive_lead_cycles: number;
  readonly drive_target_cycles: number;
  readonly load_address: number;
  readonly load_end_address: number;
  readonly load_frames: number;
  readonly loaded_file_sha256: string;
  readonly loaded_payload_length: number;
}

interface RustSaveReport {
  readonly boot_frames: number;
  readonly committed_half_tracks: readonly number[];
  readonly directory_frames: number;
  readonly drive_elapsed_cycles: number;
  readonly drive_lead_cycles: number;
  readonly drive_target_cycles: number;
  readonly first_file_load_frames: number;
  readonly program_entry_frames: number;
  readonly reload_end_address: number;
  readonly reload_frames: number;
  readonly reloaded_file_sha256: string;
  readonly save_frames: number;
  readonly saved_file_sha256: string;
}

interface RustFormatReport {
  readonly boot_frames: number;
  readonly committed_track_count: number;
  readonly drive_elapsed_cycles: number;
  readonly drive_lead_cycles: number;
  readonly drive_target_cycles: number;
  readonly load_frames: number;
  readonly program_byte_length: number;
  readonly program_preserved: boolean;
  readonly result_code: number;
  readonly run_frames: number;
}

const VICE_TEST_REVISION = 46_176;
const REFERENCE_DIRECTORY = resolve('output/reference');
const FIRMWARE_PATHS = [
  resolve('public/firmware/basic.901226-01.bin'),
  resolve('public/firmware/characters.901225-01.bin'),
  resolve('public/firmware/kernal.901227-03.bin'),
] as const;
const DRIVE_ROM = {
  cachePath: resolve(REFERENCE_DIRECTORY, '1541-II.251968-03.bin'),
  name: 'Commodore 1541-II DOS ROM',
  sha256: '326c289c38753323d7e8167897447cf61ef35189d82eb8d75210ece949adda7c',
  url: 'https://www.zimmers.net/anonftp/pub/cbm/firmware/drives/new/1541/1541-II.251968-03.bin',
} as const;
const FRAMEWORK_DISK = {
  cachePath: resolve(REFERENCE_DIRECTORY, 'vice-1541-framework.d64'),
  name: `VICE 1541 framework.d64 revision ${VICE_TEST_REVISION}`,
  sha256: 'b094002c8b7d868a31fe4d93ab8ea027b2d7feaf6bce083b883c05d22affc128',
  url:
    `https://sourceforge.net/p/vice-emu/code/${VICE_TEST_REVISION}/tree/` +
    'testprogs/drive/1541-testsuite/disks/framework.d64?format=raw',
} as const;
const FORMAT_DISK = {
  cachePath: resolve(REFERENCE_DIRECTORY, 'vice-drive-format.d64'),
  name: `VICE drive/format/format.d64 revision ${VICE_TEST_REVISION}`,
  sha256: '7648898420e108b01167d8a04c605fec243e8114e85ad87cd9e537799e6cb54b',
  url:
    `https://sourceforge.net/p/vice-emu/code/${VICE_TEST_REVISION}/tree/` +
    'testprogs/drive/format/format.d64?format=raw',
} as const;

function decodeReport(stdout: string): Record<string, unknown> {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust VICE drive runner produced no report.');
  const decoded: unknown = JSON.parse(line);
  if (typeof decoded !== 'object' || decoded === null || Array.isArray(decoded)) {
    throw new Error('Rust VICE drive runner report is not an object.');
  }
  return decoded as Record<string, unknown>;
}

function reportInteger(record: Record<string, unknown>, name: string): number {
  const value = record[name];
  if (typeof value !== 'number' || !Number.isSafeInteger(value)) {
    throw new Error(`Rust VICE drive runner returned an invalid ${name}.`);
  }
  return value;
}

function reportBoolean(record: Record<string, unknown>, name: string): boolean {
  const value = record[name];
  if (typeof value !== 'boolean') {
    throw new Error(`Rust VICE drive runner returned an invalid ${name}.`);
  }
  return value;
}

function reportSha256(record: Record<string, unknown>, name: string): string {
  const value = record[name];
  if (typeof value !== 'string' || !/^[0-9a-f]{64}$/u.test(value)) {
    throw new Error(`Rust VICE drive runner returned an invalid ${name}.`);
  }
  return value;
}

function reportIntegerArray(record: Record<string, unknown>, name: string): readonly number[] {
  const value = record[name];
  if (!Array.isArray(value) || !value.every((entry) => Number.isSafeInteger(entry))) {
    throw new Error(`Rust VICE drive runner returned an invalid ${name}.`);
  }
  return value as number[];
}

function requireDirectoryReport(stdout: string): RustDirectoryReport {
  const record = decodeReport(stdout);
  return {
    boot_frames: reportInteger(record, 'boot_frames'),
    directory_end_address: reportInteger(record, 'directory_end_address'),
    directory_entry_found: reportBoolean(record, 'directory_entry_found'),
    directory_frames: reportInteger(record, 'directory_frames'),
    directory_title_found: reportBoolean(record, 'directory_title_found'),
    drive_elapsed_cycles: reportInteger(record, 'drive_elapsed_cycles'),
    drive_half_track: reportInteger(record, 'drive_half_track'),
    drive_lead_cycles: reportInteger(record, 'drive_lead_cycles'),
    drive_program_counter: reportInteger(record, 'drive_program_counter'),
    drive_target_cycles: reportInteger(record, 'drive_target_cycles'),
    system_cycles: reportInteger(record, 'system_cycles'),
  };
}

function requireLoadReport(stdout: string): RustLoadReport {
  const record = decodeReport(stdout);
  return {
    boot_frames: reportInteger(record, 'boot_frames'),
    directory_frames: reportInteger(record, 'directory_frames'),
    drive_elapsed_cycles: reportInteger(record, 'drive_elapsed_cycles'),
    drive_lead_cycles: reportInteger(record, 'drive_lead_cycles'),
    drive_target_cycles: reportInteger(record, 'drive_target_cycles'),
    load_address: reportInteger(record, 'load_address'),
    load_end_address: reportInteger(record, 'load_end_address'),
    load_frames: reportInteger(record, 'load_frames'),
    loaded_file_sha256: reportSha256(record, 'loaded_file_sha256'),
    loaded_payload_length: reportInteger(record, 'loaded_payload_length'),
  };
}

function requireSaveReport(stdout: string): RustSaveReport {
  const record = decodeReport(stdout);
  return {
    boot_frames: reportInteger(record, 'boot_frames'),
    committed_half_tracks: reportIntegerArray(record, 'committed_half_tracks'),
    directory_frames: reportInteger(record, 'directory_frames'),
    drive_elapsed_cycles: reportInteger(record, 'drive_elapsed_cycles'),
    drive_lead_cycles: reportInteger(record, 'drive_lead_cycles'),
    drive_target_cycles: reportInteger(record, 'drive_target_cycles'),
    first_file_load_frames: reportInteger(record, 'first_file_load_frames'),
    program_entry_frames: reportInteger(record, 'program_entry_frames'),
    reload_end_address: reportInteger(record, 'reload_end_address'),
    reload_frames: reportInteger(record, 'reload_frames'),
    reloaded_file_sha256: reportSha256(record, 'reloaded_file_sha256'),
    save_frames: reportInteger(record, 'save_frames'),
    saved_file_sha256: reportSha256(record, 'saved_file_sha256'),
  };
}

function requireFormatReport(stdout: string): RustFormatReport {
  const record = decodeReport(stdout);
  return {
    boot_frames: reportInteger(record, 'boot_frames'),
    committed_track_count: reportInteger(record, 'committed_track_count'),
    drive_elapsed_cycles: reportInteger(record, 'drive_elapsed_cycles'),
    drive_lead_cycles: reportInteger(record, 'drive_lead_cycles'),
    drive_target_cycles: reportInteger(record, 'drive_target_cycles'),
    load_frames: reportInteger(record, 'load_frames'),
    program_byte_length: reportInteger(record, 'program_byte_length'),
    program_preserved: reportBoolean(record, 'program_preserved'),
    result_code: reportInteger(record, 'result_code'),
    run_frames: reportInteger(record, 'run_frames'),
  };
}

function runScenario(
  executable: string,
  scenario: string,
  diskPath = FRAMEWORK_DISK.cachePath,
): string {
  return execFileSync(executable, [...FIRMWARE_PATHS, DRIVE_ROM.cachePath, diskPath, scenario], {
    encoding: 'utf8',
    maxBuffer: 4 * 1024 * 1024,
  });
}

async function main(): Promise<void> {
  await Promise.all([
    loadPinnedReferenceAsset(DRIVE_ROM),
    loadPinnedReferenceAsset(FRAMEWORK_DISK),
    loadPinnedReferenceAsset(FORMAT_DISK),
  ]);
  execFileSync(
    'cargo',
    ['build', '--quiet', '--release', '--locked', '-p', 'c64-core', '--example', 'vice_drive'],
    { stdio: 'inherit' },
  );
  const executable = resolve(
    `target/release/examples/vice_drive${process.platform === 'win32' ? '.exe' : ''}`,
  );
  const report = requireDirectoryReport(runScenario(executable, 'directory'));
  if (report.boot_frames !== 109) {
    throw new Error(`Rust C64 reached BASIC READY in ${report.boot_frames} frames; expected 109.`);
  }
  if (report.directory_frames !== 101) {
    throw new Error(
      `Rust 1541 loaded the framework directory in ${report.directory_frames} frames; expected 101.`,
    );
  }
  if (!report.directory_title_found || !report.directory_entry_found) {
    throw new Error('Rust 1541 directory load is missing the fixed title or first entry.');
  }
  if (report.directory_end_address <= 0x0801) {
    throw new Error('Rust 1541 directory load did not advance the BASIC text end pointer.');
  }
  if (
    report.drive_lead_cycles !== 0 ||
    report.drive_elapsed_cycles !== report.drive_target_cycles
  ) {
    throw new Error(
      `Rust 1541 clock is not synchronized: elapsed=${report.drive_elapsed_cycles}, ` +
        `target=${report.drive_target_cycles}, lead=${report.drive_lead_cycles}.`,
    );
  }

  const loadReport = requireLoadReport(runScenario(executable, 'load'));
  if (
    loadReport.boot_frames !== 109 ||
    loadReport.directory_frames !== 101 ||
    loadReport.load_frames !== 197
  ) {
    throw new Error(
      `Rust 1541 LOAD frame counts are ${loadReport.boot_frames}/` +
        `${loadReport.directory_frames}/${loadReport.load_frames}; expected 109/101/197.`,
    );
  }
  if (
    loadReport.load_address !== 0x0801 ||
    loadReport.load_end_address !== 0x0cd6 ||
    loadReport.loaded_payload_length !== 1_237
  ) {
    throw new Error(
      `Rust 1541 LOAD range is $${loadReport.load_address.toString(16)}..` +
        `$${loadReport.load_end_address.toString(16)} (${loadReport.loaded_payload_length} bytes); ` +
        'expected $0801..$0cd6 (1237 bytes).',
    );
  }
  if (
    loadReport.loaded_file_sha256 !==
    '3d992332728ed415b0958b719f3cd1be4410e28da5719c70ce5e51a8e4926063'
  ) {
    throw new Error(
      `Rust 1541 LOAD returned unexpected PRG SHA-256 ${loadReport.loaded_file_sha256}.`,
    );
  }
  if (
    loadReport.drive_lead_cycles !== 0 ||
    loadReport.drive_elapsed_cycles !== loadReport.drive_target_cycles
  ) {
    throw new Error(
      `Rust 1541 clock is not synchronized after LOAD: elapsed=${loadReport.drive_elapsed_cycles}, ` +
        `target=${loadReport.drive_target_cycles}, lead=${loadReport.drive_lead_cycles}.`,
    );
  }

  const saveReport = requireSaveReport(runScenario(executable, 'save'));
  if (
    saveReport.boot_frames !== 109 ||
    saveReport.directory_frames !== 101 ||
    saveReport.first_file_load_frames !== 197 ||
    saveReport.program_entry_frames !== 2 ||
    saveReport.save_frames !== 125 ||
    saveReport.reload_frames !== 33
  ) {
    throw new Error(
      'Rust 1541 SAVE/LOAD frame counts are ' +
        `${saveReport.boot_frames}/${saveReport.directory_frames}/` +
        `${saveReport.first_file_load_frames}/${saveReport.program_entry_frames}/` +
        `${saveReport.save_frames}/${saveReport.reload_frames}; expected 109/101/197/2/125/33.`,
    );
  }
  if (
    saveReport.committed_half_tracks.length !== 2 ||
    saveReport.committed_half_tracks[0] !== 34 ||
    saveReport.committed_half_tracks[1] !== 36
  ) {
    throw new Error(
      `Rust 1541 SAVE committed unexpected half-tracks ${saveReport.committed_half_tracks.join(', ')}.`,
    );
  }
  const expectedSavedFileSha256 =
    'dc207e83f8e1950a531b425e7e98a85bd4fb37fac9cb42d5316d8ca784c41e8f';
  if (
    saveReport.saved_file_sha256 !== expectedSavedFileSha256 ||
    saveReport.reloaded_file_sha256 !== expectedSavedFileSha256 ||
    saveReport.reload_end_address !== 0x0810
  ) {
    throw new Error(
      `Rust 1541 SAVE/LOAD content mismatch: disk=${saveReport.saved_file_sha256}, ` +
        `RAM=${saveReport.reloaded_file_sha256}, end=$${saveReport.reload_end_address.toString(16)}.`,
    );
  }
  if (
    saveReport.drive_lead_cycles !== 0 ||
    saveReport.drive_elapsed_cycles !== saveReport.drive_target_cycles
  ) {
    throw new Error(
      `Rust 1541 clock is not synchronized after SAVE/LOAD: ` +
        `elapsed=${saveReport.drive_elapsed_cycles}, target=${saveReport.drive_target_cycles}, ` +
        `lead=${saveReport.drive_lead_cycles}.`,
    );
  }

  const formatReport = requireFormatReport(
    runScenario(executable, 'format', FORMAT_DISK.cachePath),
  );
  if (
    formatReport.boot_frames !== 109 ||
    formatReport.load_frames !== 142 ||
    formatReport.run_frames !== 3_530
  ) {
    throw new Error(
      `Rust 1541 format frame counts are ${formatReport.boot_frames}/` +
        `${formatReport.load_frames}/${formatReport.run_frames}; expected 109/142/3530.`,
    );
  }
  if (
    formatReport.result_code !== 0 ||
    formatReport.committed_track_count !== 35 ||
    formatReport.program_byte_length !== 350 ||
    !formatReport.program_preserved
  ) {
    throw new Error(
      `Rust 1541 format result is code=$${formatReport.result_code.toString(16)}, ` +
        `tracks=${formatReport.committed_track_count}, bytes=${formatReport.program_byte_length}, ` +
        `preserved=${String(formatReport.program_preserved)}.`,
    );
  }
  if (
    formatReport.drive_lead_cycles !== 0 ||
    formatReport.drive_elapsed_cycles !== formatReport.drive_target_cycles
  ) {
    throw new Error(
      `Rust 1541 clock is not synchronized after format: ` +
        `elapsed=${formatReport.drive_elapsed_cycles}, target=${formatReport.drive_target_cycles}, ` +
        `lead=${formatReport.drive_lead_cycles}.`,
    );
  }
  console.log(
    `PASS Rust VICE 1541 directory reference revision ${VICE_TEST_REVISION}: BASIC READY in ` +
      `${report.boot_frames} PAL frames; directory in ${report.directory_frames}; ` +
      `first PRG (${loadReport.loaded_payload_length} bytes) in ${loadReport.load_frames}; ` +
      `SAVE in ${saveReport.save_frames}, reload in ${saveReport.reload_frames}; ` +
      `format in ${formatReport.run_frames}; ` +
      `drive PC=$${report.drive_program_counter.toString(16).padStart(4, '0')}, ` +
      `half-track ${report.drive_half_track}.`,
  );
}

await main();
