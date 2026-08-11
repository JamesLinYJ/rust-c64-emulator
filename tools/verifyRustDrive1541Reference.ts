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

function requireReport(stdout: string): RustDirectoryReport {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust VICE drive runner produced no report.');
  const decoded: unknown = JSON.parse(line);
  if (typeof decoded !== 'object' || decoded === null || Array.isArray(decoded)) {
    throw new Error('Rust VICE drive runner report is not an object.');
  }
  const record = decoded as Record<string, unknown>;
  const integer = (name: keyof RustDirectoryReport): number => {
    const value = record[name];
    if (typeof value !== 'number' || !Number.isSafeInteger(value)) {
      throw new Error(`Rust VICE drive runner returned an invalid ${name}.`);
    }
    return value;
  };
  const boolean = (name: keyof RustDirectoryReport): boolean => {
    const value = record[name];
    if (typeof value !== 'boolean') {
      throw new Error(`Rust VICE drive runner returned an invalid ${name}.`);
    }
    return value;
  };
  return {
    boot_frames: integer('boot_frames'),
    directory_end_address: integer('directory_end_address'),
    directory_entry_found: boolean('directory_entry_found'),
    directory_frames: integer('directory_frames'),
    directory_title_found: boolean('directory_title_found'),
    drive_elapsed_cycles: integer('drive_elapsed_cycles'),
    drive_half_track: integer('drive_half_track'),
    drive_lead_cycles: integer('drive_lead_cycles'),
    drive_program_counter: integer('drive_program_counter'),
    drive_target_cycles: integer('drive_target_cycles'),
    system_cycles: integer('system_cycles'),
  };
}

async function main(): Promise<void> {
  await Promise.all([
    loadPinnedReferenceAsset(DRIVE_ROM),
    loadPinnedReferenceAsset(FRAMEWORK_DISK),
  ]);
  execFileSync(
    'cargo',
    ['build', '--quiet', '--release', '--locked', '-p', 'c64-core', '--example', 'vice_drive'],
    { stdio: 'inherit' },
  );
  const executable = resolve(
    `target/release/examples/vice_drive${process.platform === 'win32' ? '.exe' : ''}`,
  );
  const stdout = execFileSync(
    executable,
    [...FIRMWARE_PATHS, DRIVE_ROM.cachePath, FRAMEWORK_DISK.cachePath, 'directory'],
    { encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 },
  );
  const report = requireReport(stdout);
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
  console.log(
    `PASS Rust VICE 1541 directory reference revision ${VICE_TEST_REVISION}: BASIC READY in ` +
      `${report.boot_frames} PAL frames; directory in ${report.directory_frames}; ` +
      `drive PC=$${report.drive_program_counter.toString(16).padStart(4, '0')}, ` +
      `half-track ${report.drive_half_track}.`,
  );
}

await main();
