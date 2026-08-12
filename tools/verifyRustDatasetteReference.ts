// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Rust Datasette VICE end-to-end reference gate
//
//   File:       verifyRustDatasetteReference.ts
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

import { createCommodoreRomTapeFixture } from './reference/CommodoreRomTapeFixture';
import { loadPinnedReferenceAsset } from './reference/loadPinnedReferenceAsset';

const FIRMWARE_PATHS = [
  resolve('public/firmware/basic.901226-01.bin'),
  resolve('public/firmware/characters.901225-01.bin'),
  resolve('public/firmware/kernal.901227-03.bin'),
] as const;
const LOAD_TAP_PATH = resolve('output/reference/rust-kernal-load.tap');
const EXPECTED_LOAD_TAP_SHA256 = '72434c68f55b078e9cabca5e6db55273c9fbf87f98d668d93c681bb65958f731';
const EXPECTED_SAVE_TAP_SHA256 = 'c7503b92224d157bd1ba05fa0b1c100a8ddca6c9ea679ec52a2dc517abcead02';
const EXPECTED_VICE_WRITE_PULSES = [0x20 * 8, 0x40 * 8, 0x60 * 8, 0x40 * 8] as const;
const VICE_TAPE_WRITE_ASSET = {
  cachePath: resolve('output/reference/vice-testprogs/tape/tap204060/tap204060once.prg'),
  name: 'VICE revision 46176 tap204060once.prg',
  sha256: '5311adc89f6296b9dee38166bf1b1e588c5343e7e3c8f41203b3ef1ea5e693a9',
  url: 'https://sourceforge.net/p/vice-emu/code/46176/tree/testprogs/tape/tap204060/tap204060once.prg?format=raw',
} as const;

interface RustTapeReport {
  readonly load: {
    readonly boot_frames: number;
    readonly end_address: number;
    readonly frames: number;
    readonly pulse_count: number;
    readonly status: number;
  };
  readonly save: {
    readonly line_entry_frames: number;
    readonly load_frames: number;
    readonly pulse_count: number;
    readonly save_frames: number;
    readonly serialized_sha256: string;
  };
  readonly vice_write: {
    readonly frames: number;
    readonly initial_pause_cycles: number;
    readonly serialized_sha256: string;
    readonly tail_pulses: readonly number[];
  };
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

function decodeReport(stdout: string): RustTapeReport {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust Datasette runner produced no report.');
  const decoded: unknown = JSON.parse(line);
  if (typeof decoded !== 'object' || decoded === null || Array.isArray(decoded)) {
    throw new Error('Rust Datasette runner report is not an object.');
  }
  const report = decoded as Partial<RustTapeReport>;
  requireRecord(report.load, 'load');
  requireRecord(report.save, 'save');
  requireRecord(report.vice_write, 'vice_write');
  for (const [record, fields] of [
    [report.load, ['boot_frames', 'end_address', 'frames', 'pulse_count', 'status']],
    [report.save, ['line_entry_frames', 'load_frames', 'pulse_count', 'save_frames']],
    [report.vice_write, ['frames', 'initial_pause_cycles']],
  ] as const) {
    for (const field of fields) requireInteger(record, field);
  }
  requireSha256(report.save, 'serialized_sha256');
  requireSha256(report.vice_write, 'serialized_sha256');
  if (
    !Array.isArray(report.vice_write.tail_pulses) ||
    !report.vice_write.tail_pulses.every((pulse) => Number.isSafeInteger(pulse))
  ) {
    throw new Error('Rust Datasette runner returned invalid VICE tail pulses.');
  }
  return report as RustTapeReport;
}

function requireRecord(value: unknown, name: string): asserts value is Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`Rust Datasette runner returned invalid ${name}.`);
  }
}

function requireInteger(record: Record<string, unknown>, field: string): void {
  if (!Number.isSafeInteger(record[field])) {
    throw new Error(`Rust Datasette runner returned invalid ${field}.`);
  }
}

function requireSha256(record: Record<string, unknown>, field: string): void {
  if (typeof record[field] !== 'string' || !/^[0-9a-f]{64}$/u.test(record[field])) {
    throw new Error(`Rust Datasette runner returned invalid ${field}.`);
  }
}

async function main(): Promise<void> {
  await loadPinnedReferenceAsset(VICE_TAPE_WRITE_ASSET);
  const loadTap = createCommodoreRomTapeFixture({
    fileName: 'CODEX TAPE',
    loadAddress: 0xc000,
    payload: Uint8Array.from({ length: 64 }, (_, index) => (index * 73 + 0x35) & 0xff),
  });
  const loadTapHash = sha256(loadTap);
  if (loadTapHash !== EXPECTED_LOAD_TAP_SHA256) {
    throw new Error(`Rust KERNAL LOAD fixture SHA-256 changed to ${loadTapHash}.`);
  }
  await mkdir(dirname(LOAD_TAP_PATH), { recursive: true });
  await writeFile(LOAD_TAP_PATH, loadTap);

  execFileSync(
    'cargo',
    ['build', '--quiet', '--release', '--locked', '-p', 'c64-core', '--example', 'vice_tape'],
    { stdio: 'inherit' },
  );
  const executable = resolve(
    `target/release/examples/vice_tape${process.platform === 'win32' ? '.exe' : ''}`,
  );
  const result = spawnSync(
    executable,
    [...FIRMWARE_PATHS, LOAD_TAP_PATH, VICE_TAPE_WRITE_ASSET.cachePath],
    { encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 },
  );
  if (result.status !== 0) {
    throw new Error(
      `Rust Datasette runner failed (${String(result.status)}):\n${result.stderr || result.stdout}`,
    );
  }
  const report = decodeReport(result.stdout);

  if (report.load.boot_frames !== 109) {
    throw new Error(
      `Rust C64 reached BASIC READY in ${report.load.boot_frames} frames; expected 109.`,
    );
  }
  if (report.load.end_address !== 0xc040 || report.load.status !== 0) {
    throw new Error(
      `Rust KERNAL tape LOAD ended at $${report.load.end_address.toString(16)} with status ` +
        `$${report.load.status.toString(16)}.`,
    );
  }
  if (report.load.pulse_count === 0) {
    throw new Error('Rust KERNAL tape LOAD consumed no physical pulses.');
  }
  if (report.save.serialized_sha256 !== EXPECTED_SAVE_TAP_SHA256) {
    throw new Error(
      `Rust KERNAL tape SAVE SHA-256 changed to ${report.save.serialized_sha256}; ` +
        `expected TypeScript oracle ${EXPECTED_SAVE_TAP_SHA256}.`,
    );
  }
  if (
    report.vice_write.tail_pulses.length !== EXPECTED_VICE_WRITE_PULSES.length ||
    report.vice_write.tail_pulses.some(
      (pulse, index) => pulse !== EXPECTED_VICE_WRITE_PULSES[index],
    )
  ) {
    throw new Error(
      `Rust VICE tape WRITE tail changed to ${report.vice_write.tail_pulses.join('/')}.`,
    );
  }
  if (report.vice_write.initial_pause_cycles < 985_248) {
    throw new Error('Rust VICE tape WRITE initial pause is shorter than one PAL second.');
  }

  console.log(
    `PASS Rust Datasette KERNAL LOAD: ${report.load.pulse_count.toLocaleString('en-US')} ` +
      `READ pulses in ${report.load.frames} PAL frames; TAP SHA-256 ${loadTapHash}.`,
  );
  console.log(
    `PASS Rust Datasette VICE WRITE: ${report.vice_write.tail_pulses.join('/')} in ` +
      `${report.vice_write.frames} PAL frames; TAP SHA-256 ` +
      `${report.vice_write.serialized_sha256}.`,
  );
  console.log(
    `PASS Rust Datasette KERNAL SAVE/LOAD: ${report.save.pulse_count.toLocaleString('en-US')} ` +
      `pulses, save ${report.save.save_frames} frames, reload ${report.save.load_frames} frames; ` +
      `TAP SHA-256 ${report.save.serialized_sha256}.`,
  );
}

await main();
