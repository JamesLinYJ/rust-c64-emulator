// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Rust official EasyProg reference gate
//
//   File:       verifyRustEasyFlashReference.ts
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

import { unzipSync } from 'fflate';

import { loadPinnedReferenceAsset } from './reference/loadPinnedReferenceAsset';

const EASY_PROG_VERSION = '1.6.3';
const EASY_PROG_ENTRY = `easyprog-${EASY_PROG_VERSION}/easyprog-${EASY_PROG_VERSION}.prg`;
const EASY_PROG_SHA256 = '2866553213bb419ea1ae54aaf750e910ae3a1934f870079ea43eab8aefb87536';
const EASY_PROG_PATH = resolve(`output/reference/easyflash/easyprog-${EASY_PROG_VERSION}.prg`);
const EASY_PROG_ARCHIVE = {
  cachePath: resolve(`output/reference/easyflash/easyprog-${EASY_PROG_VERSION}.zip`),
  name: `EasyProg ${EASY_PROG_VERSION} archive`,
  sha256: '45f6ddc36504312d7de4b13b1d7b75f33c60cbbea10311a9a0de93b3e33c6df1',
  url: `https://skoe.de/easyflash/files/easyprog/easyprog-${EASY_PROG_VERSION}.zip`,
} as const;
const FIRMWARE_PATHS = [
  resolve('public/firmware/basic.901226-01.bin'),
  resolve('public/firmware/characters.901225-01.bin'),
  resolve('public/firmware/kernal.901227-03.bin'),
] as const;

interface RustEasyFlashReport {
  readonly boot_frames: number;
  readonly changed_high_bytes: number;
  readonly changed_low_bytes: number;
  readonly initialization_frames: number;
  readonly programming_frames: number;
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

function decodeReport(stdout: string): RustEasyFlashReport {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust EasyFlash runner produced no report.');
  const decoded: unknown = JSON.parse(line);
  if (typeof decoded !== 'object' || decoded === null || Array.isArray(decoded)) {
    throw new Error('Rust EasyFlash runner report is not an object.');
  }
  const record = decoded as Record<string, unknown>;
  for (const field of [
    'boot_frames',
    'changed_high_bytes',
    'changed_low_bytes',
    'initialization_frames',
    'programming_frames',
  ] as const) {
    if (!Number.isSafeInteger(record[field])) {
      throw new Error(`Rust EasyFlash runner returned invalid ${field}.`);
    }
  }
  return record as unknown as RustEasyFlashReport;
}

async function main(): Promise<void> {
  const archive = await loadPinnedReferenceAsset(EASY_PROG_ARCHIVE);
  const program = unzipSync(archive)[EASY_PROG_ENTRY];
  if (program === undefined) {
    throw new Error(`EasyProg archive does not contain ${EASY_PROG_ENTRY}.`);
  }
  const programHash = sha256(program);
  if (programHash !== EASY_PROG_SHA256) {
    throw new Error(`EasyProg PRG SHA-256 mismatch: received ${programHash}.`);
  }
  await mkdir(dirname(EASY_PROG_PATH), { recursive: true });
  await writeFile(EASY_PROG_PATH, program);

  execFileSync(
    'cargo',
    ['build', '--quiet', '--release', '--locked', '-p', 'c64-core', '--example', 'vice_easyflash'],
    { stdio: 'inherit' },
  );
  const executable = resolve(
    `target/release/examples/vice_easyflash${process.platform === 'win32' ? '.exe' : ''}`,
  );
  const result = spawnSync(executable, [...FIRMWARE_PATHS, EASY_PROG_PATH], {
    encoding: 'utf8',
    maxBuffer: 4 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(
      `Rust EasyFlash runner failed (${String(result.status)}):\n${result.stderr || result.stdout}`,
    );
  }
  const report = decodeReport(result.stdout);

  if (report.boot_frames !== 109) {
    throw new Error(`Rust C64 reached BASIC READY in ${report.boot_frames} frames; expected 109.`);
  }
  if (report.changed_low_bytes < 0x100 || report.changed_high_bytes < 0x100) {
    throw new Error(
      `Rust EasyProg programmed only ${report.changed_low_bytes} ROML and ` +
        `${report.changed_high_bytes} ROMH bytes.`,
    );
  }
  if (report.initialization_frames <= 0 || report.programming_frames <= 0) {
    throw new Error('Rust EasyProg did not advance through initialization and programming.');
  }

  console.log(
    `PASS Rust official EasyProg ${EASY_PROG_VERSION}: BASIC READY in ${report.boot_frames} ` +
      `frames, AM29F040B pair detected in ${report.initialization_frames} frames; its 6510 ` +
      `torture path programmed ${report.changed_low_bytes} ROML and ${report.changed_high_bytes} ` +
      `ROMH bytes after ${report.programming_frames} PAL frames; PRG SHA-256 ${programHash}.`,
  );
}

await main();
