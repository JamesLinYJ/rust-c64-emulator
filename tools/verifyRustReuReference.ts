// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE QuickReu functional reference gate
//
//   File:       verifyRustReuReference.ts
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { execFileSync, spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

import {
  loadPinnedReferenceAsset,
  type PinnedReferenceAsset,
} from './reference/loadPinnedReferenceAsset';

const VICE_TEST_REVISION = 46_176;
const FIRMWARE_PATHS = [
  resolve('public/firmware/basic.901226-01.bin'),
  resolve('public/firmware/characters.901225-01.bin'),
  resolve('public/firmware/kernal.901227-03.bin'),
] as const;
const QUICK_REU_HASHES = [
  '52b36c94319d72bd152ffffb89405d32fc502859acdb9097c6b5874d79529cf9',
  '5df38a1b200bd7534a2c31d286d0c568370f584bc02e0ef02370c545d18ff2a8',
  '6b24606a7b6ddf86556311fa85da35077a28e7b95831620d0428ee1aa594e79d',
  '9b017a4a939b41ffca37b58739d29e72fe75eed5b5c1afcc0caffde0efa6c886',
  'a7edfec1d1dc9d0e7f545f954791b6a3f6946bef09d8eeeb7e013a2c1cd13f25',
  '55b78052712d301c6eabc7c4d053e765ed06098cbbe9734a7512abc8cc9539d1',
  'df1d63cf1217755d7b97694346255f509c561e122d017ca320e6ac236c30f34d',
  '18ad2e0f64fb4e21a63a1c57e46b270c791f60341f8baacfa94fe94304ef46a6',
] as const;
const EXPECTED_RESULT_FRAMES = [37, 37, 37, 38, 38, 37, 38, 37] as const;
const EXPECTED_DMA_BUS_CYCLES = 44_588;
const EXPECTED_DMA_SYSTEM_CYCLES = 44_589;

interface RustReuReport {
  readonly boot_frames: number;
  readonly dma_bus_cycles: number;
  readonly dma_system_cycles: number;
  readonly dma_vic_stall_cycles: number;
  readonly result_frames: number;
}

function quickReuAsset(index: number): PinnedReferenceAsset {
  const testNumber = index + 1;
  const fileName = `quickreu-test${testNumber}.prg`;
  return {
    cachePath: resolve(`output/reference/vice-testprogs/REU/QuickReuTest-1.1.1/${fileName}`),
    name: `VICE revision ${VICE_TEST_REVISION} ${fileName}`,
    sha256: QUICK_REU_HASHES[index] ?? '',
    url:
      `https://sourceforge.net/p/vice-emu/code/${VICE_TEST_REVISION}/tree/` +
      `testprogs/REU/QuickReuTest-1.1.1/${fileName}?format=raw`,
  };
}

function decodeReport(stdout: string): RustReuReport {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust QuickReu runner produced no report.');
  const decoded: unknown = JSON.parse(line);
  if (typeof decoded !== 'object' || decoded === null || Array.isArray(decoded)) {
    throw new Error('Rust QuickReu runner report is not an object.');
  }
  const record = decoded as Record<string, unknown>;
  for (const field of [
    'boot_frames',
    'dma_bus_cycles',
    'dma_system_cycles',
    'dma_vic_stall_cycles',
    'result_frames',
  ] as const) {
    if (!Number.isSafeInteger(record[field])) {
      throw new Error(`Rust QuickReu runner returned invalid ${field}.`);
    }
  }
  return record as unknown as RustReuReport;
}

async function main(): Promise<void> {
  const assets = QUICK_REU_HASHES.map((_, index) => quickReuAsset(index));
  await Promise.all(assets.map(async (asset) => loadPinnedReferenceAsset(asset)));
  execFileSync(
    'cargo',
    ['build', '--quiet', '--release', '--locked', '-p', 'c64-core', '--example', 'vice_reu'],
    { stdio: 'inherit' },
  );
  const executable = resolve(
    `target/release/examples/vice_reu${process.platform === 'win32' ? '.exe' : ''}`,
  );

  for (const [index, asset] of assets.entries()) {
    const result = spawnSync(executable, [...FIRMWARE_PATHS, asset.cachePath], {
      encoding: 'utf8',
      maxBuffer: 4 * 1024 * 1024,
    });
    if (result.status !== 0) {
      throw new Error(
        `Rust QuickReu test ${index + 1} failed (${String(result.status)}):\n` +
          `${result.stderr || result.stdout}`,
      );
    }
    const report = decodeReport(result.stdout);
    if (
      report.boot_frames !== 109 ||
      report.result_frames !== EXPECTED_RESULT_FRAMES[index] ||
      report.dma_bus_cycles !== EXPECTED_DMA_BUS_CYCLES ||
      report.dma_system_cycles !== EXPECTED_DMA_SYSTEM_CYCLES ||
      report.dma_vic_stall_cycles !== 0
    ) {
      throw new Error(`Rust QuickReu test ${index + 1} changed: ${JSON.stringify(report)}.`);
    }
  }

  console.log(
    `PASS Rust VICE QuickReu 1.1.1 revision ${VICE_TEST_REVISION}: ` +
      `${assets.length} functional PRGs, ${EXPECTED_DMA_BUS_CYCLES} DMA bus cycles per run, ` +
      'all reported zero failed classes.',
  );
}

await main();
