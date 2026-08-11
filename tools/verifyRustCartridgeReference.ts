// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Rust VICE Ocean cartridge reference gate
//
//   File:       verifyRustCartridgeReference.ts
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { execFileSync, spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

import { loadPinnedReferenceAsset } from './reference/loadPinnedReferenceAsset';

const FIRMWARE_PATHS = [
  resolve('public/firmware/basic.901226-01.bin'),
  resolve('public/firmware/characters.901225-01.bin'),
  resolve('public/firmware/kernal.901227-03.bin'),
] as const;
const OCEAN_REFERENCE = {
  cachePath: resolve('output/reference/vice-testprogs/C64/carts/ocean/ocean.crt'),
  name: 'VICE revision 46176 ocean.crt',
  sha256: '426dd453d7702e0333c8b98df9e35ca2513a19184d8f2766b4acc2f8b4a684a5',
  url: 'https://sourceforge.net/p/vice-emu/code/46176/tree/testprogs/C64/carts/ocean/ocean.crt?format=raw',
} as const;
const EXPECTED_BANK_SHA256 = [
  '12c0b361abb12f552bae83fc4f1cd0b646ace5b510bbd53bf9fca371f914b64a',
  '9f1dcbc35c350d6027f98be0f5c8b43b42ca52b7604459c0c42be3aa88913d47',
  '9f1dcbc35c350d6027f98be0f5c8b43b42ca52b7604459c0c42be3aa88913d47',
  '9f1dcbc35c350d6027f98be0f5c8b43b42ca52b7604459c0c42be3aa88913d47',
] as const;
const EXPECTED_SCREEN_COPY_SHA256 =
  'c199b812bf803087360c2737b93b2ab39e7a6ea91af8b4b9a736b1a6308c0d5c';

interface RustCartridgeReport {
  readonly activity: number;
  readonly bank_sha256: readonly string[];
  readonly frames: number;
  readonly rom_pc_frame_samples: number;
  readonly screen_copy_sha256: string;
}

function decodeReport(stdout: string): RustCartridgeReport {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust cartridge runner produced no report.');
  const decoded: unknown = JSON.parse(line);
  if (typeof decoded !== 'object' || decoded === null || Array.isArray(decoded)) {
    throw new Error('Rust cartridge runner report is not an object.');
  }
  const record = decoded as Record<string, unknown>;
  for (const field of ['activity', 'frames', 'rom_pc_frame_samples'] as const) {
    if (!Number.isSafeInteger(record[field])) {
      throw new Error(`Rust cartridge runner returned invalid ${field}.`);
    }
  }
  if (
    !Array.isArray(record.bank_sha256) ||
    !record.bank_sha256.every((hash) => typeof hash === 'string' && /^[0-9a-f]{64}$/u.test(hash))
  ) {
    throw new Error('Rust cartridge runner returned invalid bank hashes.');
  }
  if (
    typeof record.screen_copy_sha256 !== 'string' ||
    !/^[0-9a-f]{64}$/u.test(record.screen_copy_sha256)
  ) {
    throw new Error('Rust cartridge runner returned an invalid screen-copy hash.');
  }
  return record as unknown as RustCartridgeReport;
}

async function main(): Promise<void> {
  await loadPinnedReferenceAsset(OCEAN_REFERENCE);
  execFileSync(
    'cargo',
    ['build', '--quiet', '--release', '--locked', '-p', 'c64-core', '--example', 'vice_cartridge'],
    { stdio: 'inherit' },
  );
  const executable = resolve(
    `target/release/examples/vice_cartridge${process.platform === 'win32' ? '.exe' : ''}`,
  );
  const result = spawnSync(executable, [...FIRMWARE_PATHS, OCEAN_REFERENCE.cachePath], {
    encoding: 'utf8',
    maxBuffer: 4 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(
      `Rust cartridge runner failed (${String(result.status)}):\n${result.stderr || result.stdout}`,
    );
  }
  const report = decodeReport(result.stdout);

  if (
    report.bank_sha256.length !== EXPECTED_BANK_SHA256.length ||
    report.bank_sha256.some((hash, index) => hash !== EXPECTED_BANK_SHA256[index])
  ) {
    throw new Error(`Rust Ocean bank hashes changed to ${report.bank_sha256.join(', ')}.`);
  }
  if (report.frames !== 120 || report.rom_pc_frame_samples < 20 || report.activity === 0) {
    throw new Error(
      `Rust Ocean execution changed: ${report.frames} frames, ` +
        `${report.rom_pc_frame_samples} ROM samples, activity ${report.activity}.`,
    );
  }
  if (report.screen_copy_sha256 !== EXPECTED_SCREEN_COPY_SHA256) {
    throw new Error(
      `Rust Ocean screen-copy SHA-256 changed to ${report.screen_copy_sha256}; ` +
        `expected ${EXPECTED_SCREEN_COPY_SHA256}.`,
    );
  }

  console.log(
    `PASS Rust VICE Ocean CRT revision 46176: ${report.bank_sha256.length} exact mirrored banks; ` +
      `${report.rom_pc_frame_samples} ROM PC frame samples and activity ` +
      `$${report.activity.toString(16).padStart(2, '0')} after ${report.frames} PAL frames; ` +
      `SHA-256 ${OCEAN_REFERENCE.sha256}.`,
  );
}

await main();
