// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - VICE VIC-II 整机参考验证
//
//   文件:       verifyRustVicReference.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { execFileSync } from 'node:child_process';
import { mkdir, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { PNG } from 'pngjs';

import { loadPinnedReferenceAsset } from './reference/loadPinnedReferenceAsset';

interface ReferenceAsset {
  readonly cacheFileName?: string;
  readonly fileName: string;
  readonly sha256: string;
  readonly url: string;
}

interface PixelReferenceDefinition {
  readonly description: string;
  readonly entryPoint: number;
  readonly program: ReferenceAsset;
  readonly referenceImage: ReferenceAsset;
}

interface RustRunReport {
  readonly exit_code: number;
  readonly frames: number;
  readonly frame_generation: number;
  readonly program_counter: number;
  readonly raster_cycle: number;
  readonly raster_line: number;
  readonly system_cycles: number;
}

const VICE_TEST_REVISION = 46_176;
const VICE_TEST_ROOT = `https://sourceforge.net/p/vice-emu/code/${VICE_TEST_REVISION}/tree/testprogs/VICII`;
const REFERENCE_DIRECTORY = resolve('output/reference');
const FRAME_DIRECTORY = resolve('target/vice-vic-reference');
const FIRMWARE_PATHS = [
  resolve('public/firmware/basic.901226-01.bin'),
  resolve('public/firmware/characters.901225-01.bin'),
  resolve('public/firmware/kernal.901227-03.bin'),
] as const;

const RASTER_IRQ_PROGRAM: ReferenceAsset = {
  fileName: 'rasterirq_hold.prg',
  sha256: '2a1d02f6a70b1a8dd17373426493d5dc29378a1442bda6df15308b8ffd5e1a94',
  url: `${VICE_TEST_ROOT}/rasterirq/rasterirq_hold.prg?format=raw`,
};

const LIGHT_PEN_TIMING_PROGRAM: ReferenceAsset = {
  cacheFileName: 'vic-light-pen-test2.prg',
  fileName: 'test2.prg',
  sha256: 'b8beff034421415f419ccf9ee640c3afbf4b5aa7d03746a95275e1401429634c',
  url: `${VICE_TEST_ROOT}/lp-trigger/test2.prg?format=raw`,
};

const PIXEL_REFERENCE_DEFINITIONS = [
  {
    description: 'light-pen synchronized border-color dot phase',
    entryPoint: 0x080d,
    program: {
      cacheFileName: 'vic-light-pen-test1.prg',
      fileName: 'test1.prg',
      sha256: '8f41928e76d7d8e21271dbaa0fdc1dbce6c8481ed94d505c44f419aa521f3bad',
      url: `${VICE_TEST_ROOT}/lp-trigger/test1.prg?format=raw`,
    },
    referenceImage: {
      cacheFileName: 'vic-light-pen-test1.prg.png',
      fileName: 'test1.prg.png',
      sha256: 'c5f2d56fe7b6810c8971c0455905ccbbc9e9eb7365c7c9545ea1e64c3cde24a7',
      url: `${VICE_TEST_ROOT}/lp-trigger/references/test1.prg.png?format=raw`,
    },
  },
  {
    description: 'dynamic bad-line DMA',
    entryPoint: 0x080d,
    program: {
      fileName: 'test3-28-07.prg',
      sha256: '28297d89f31b18a432006e156df380b8677b074d5650556932d6ace2285d1847',
      url: `${VICE_TEST_ROOT}/dmadelay/test3-28-07.prg?format=raw`,
    },
    referenceImage: {
      fileName: 'test3-28-07.prg.png',
      sha256: 'de22e3d775444c915a76092237bd9a41885e1c681d3dcba39b1ac1ea7a53f655',
      url: `${VICE_TEST_ROOT}/dmadelay/references/test3-28-07.prg.png?format=raw`,
    },
  },
  {
    description: 'hires and multicolor sprite priority',
    entryPoint: 0x080d,
    program: {
      cacheFileName: 'vic-sprite-priorities-test1.prg',
      fileName: 'test1.prg',
      sha256: 'a818d5f27a75bb385cef91e0b290b892dcdc98cc23fb9d8f9087fee442a68b36',
      url: `${VICE_TEST_ROOT}/spritepriorities/test1.prg?format=raw`,
    },
    referenceImage: {
      cacheFileName: 'vic-sprite-priorities-test1.prg.png',
      fileName: 'test1.prg.png',
      sha256: '6b6aa40003904789d5140c23e78f2b4c6a14a4b08c65fae6d18cc51e6f25e7b2',
      url: `${VICE_TEST_ROOT}/spritepriorities/references/test1.prg.png?format=raw`,
    },
  },
  {
    description: 'sprite vertical-expansion DMA transition at raster 54',
    entryPoint: 0x080d,
    program: {
      fileName: 'd017-54.prg',
      sha256: '530bbfd4398e6c2953854e2c6c3a9a209b9a3a90a2c4a8f898bfd2923f2847d7',
      url: `${VICE_TEST_ROOT}/spritedma/d017-54.prg?format=raw`,
    },
    referenceImage: {
      fileName: 'd017-54.prg.png',
      sha256: '2cbdbc959ac07d7d539555318b60ea55893784898bafd0b13f5e846ee36ed9ae',
      url: `${VICE_TEST_ROOT}/spritedma/references/d017-54.prg.png?format=raw`,
    },
  },
  {
    description: 'sprite vertical-expansion DMA transition at raster 57',
    entryPoint: 0x080d,
    program: {
      fileName: 'd017-57.prg',
      sha256: 'a0f8773762192a690aec0c56c4946a70c5ff8c4959089cd3d25f0d025ac75a57',
      url: `${VICE_TEST_ROOT}/spritedma/d017-57.prg?format=raw`,
    },
    referenceImage: {
      fileName: 'd017-57.prg.png',
      sha256: '2cbdbc959ac07d7d539555318b60ea55893784898bafd0b13f5e846ee36ed9ae',
      url: `${VICE_TEST_ROOT}/spritedma/references/d017-57.prg.png?format=raw`,
    },
  },
] as const satisfies readonly PixelReferenceDefinition[];

const VICE_COLODORE_RGB_PALETTE = [
  0x000000, 0xffffff, 0x68372b, 0x70a4b2, 0x6f3d86, 0x588d43, 0x352879, 0xb8c76f, 0x6f4f25,
  0x433900, 0x9a6759, 0x444444, 0x6c6c6c, 0x9ad284, 0x6c5eb5, 0x959595,
] as const;

const PAL_OUTPUT = {
  height: 284,
  width: 403,
} as const;

const VICE_VIEWPORT = {
  height: 272,
  sourceX: 16,
  sourceY: 0,
  width: 384,
} as const;

function cachePath(asset: ReferenceAsset): string {
  return resolve(REFERENCE_DIRECTORY, asset.cacheFileName ?? asset.fileName);
}

async function ensureAsset(asset: ReferenceAsset): Promise<string> {
  const path = cachePath(asset);
  await loadPinnedReferenceAsset({
    cachePath: path,
    name: `VICE ${asset.fileName}`,
    sha256: asset.sha256,
    url: asset.url,
  });
  return path;
}

function requireNumericField(record: Record<string, unknown>, field: string): number {
  const value = record[field];
  if (typeof value !== 'number' || !Number.isSafeInteger(value)) {
    throw new Error(`Rust VICE runner returned an invalid ${field}.`);
  }
  return value;
}

function parseReport(stdout: string): RustRunReport {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust VICE runner produced no report.');
  const decoded: unknown = JSON.parse(line);
  if (typeof decoded !== 'object' || decoded === null || Array.isArray(decoded)) {
    throw new Error('Rust VICE runner report is not an object.');
  }
  const record = decoded as Record<string, unknown>;
  return {
    exit_code: requireNumericField(record, 'exit_code'),
    frames: requireNumericField(record, 'frames'),
    frame_generation: requireNumericField(record, 'frame_generation'),
    program_counter: requireNumericField(record, 'program_counter'),
    raster_cycle: requireNumericField(record, 'raster_cycle'),
    raster_line: requireNumericField(record, 'raster_line'),
    system_cycles: requireNumericField(record, 'system_cycles'),
  };
}

function runRustReference(
  executable: string,
  programPath: string,
  entryPoint: number,
  framePath: string,
): RustRunReport {
  const stdout = execFileSync(
    executable,
    [...FIRMWARE_PATHS, programPath, entryPoint.toString(16).padStart(4, '0'), framePath],
    { encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 },
  );
  return parseReport(stdout);
}

function assertPassed(label: string, report: RustRunReport): void {
  if (report.exit_code !== 0) {
    throw new Error(
      `${label} failed with exit code $${report.exit_code.toString(16).padStart(2, '0')} ` +
        `after ${report.frames} PAL frames (PC=$${report.program_counter.toString(16).padStart(4, '0')}, ` +
        `raster=${report.raster_line}/${report.raster_cycle}).`,
    );
  }
}

function createReferencePaletteIndex(): ReadonlyMap<number, number> {
  return new Map(VICE_COLODORE_RGB_PALETTE.map((rgb, index) => [rgb, index]));
}

function comparePaletteIndices(
  label: string,
  actual: Uint8Array,
  referenceBytes: Uint8Array,
): number {
  const expectedLength = PAL_OUTPUT.width * PAL_OUTPUT.height;
  if (actual.length !== expectedLength) {
    throw new Error(`${label} Rust frame has ${actual.length} pixels; expected ${expectedLength}.`);
  }
  const reference = PNG.sync.read(Buffer.from(referenceBytes));
  if (reference.width !== VICE_VIEWPORT.width || reference.height !== VICE_VIEWPORT.height) {
    throw new Error(
      `${label} reference is ${reference.width}x${reference.height}; ` +
        `expected ${VICE_VIEWPORT.width}x${VICE_VIEWPORT.height}.`,
    );
  }
  const palette = createReferencePaletteIndex();
  let mismatchCount = 0;
  let firstMismatch = '';
  for (let y = 0; y < reference.height; y += 1) {
    for (let x = 0; x < reference.width; x += 1) {
      const referenceOffset = (y * reference.width + x) * 4;
      const referenceRgb =
        (reference.data[referenceOffset] << 16) |
        (reference.data[referenceOffset + 1] << 8) |
        reference.data[referenceOffset + 2];
      const expected = palette.get(referenceRgb);
      if (expected === undefined) {
        throw new Error(
          `${label} reference uses non-palette RGB $${referenceRgb.toString(16).padStart(6, '0')} ` +
            `at (${x}, ${y}).`,
        );
      }
      const actualX = x + VICE_VIEWPORT.sourceX;
      const actualY = y + VICE_VIEWPORT.sourceY;
      const received = actual[actualY * PAL_OUTPUT.width + actualX];
      if (received !== expected) {
        mismatchCount += 1;
        if (firstMismatch.length === 0) {
          firstMismatch = `first mismatch at (${x}, ${y}): expected ${expected}, received ${received}`;
        }
      }
    }
  }
  if (mismatchCount !== 0) {
    throw new Error(
      `${label} Rust frame differs at ${mismatchCount} of ` +
        `${reference.width * reference.height} pixels; ${firstMismatch}.`,
    );
  }
  return reference.width * reference.height;
}

async function main(): Promise<void> {
  await mkdir(FRAME_DIRECTORY, { recursive: true });
  execFileSync(
    'cargo',
    ['build', '--quiet', '--release', '--locked', '-p', 'c64-core', '--example', 'vice_vic'],
    { stdio: 'inherit' },
  );
  const executable = resolve(
    `target/release/examples/vice_vic${process.platform === 'win32' ? '.exe' : ''}`,
  );

  const rasterProgramPath = await ensureAsset(RASTER_IRQ_PROGRAM);
  const rasterReport = runRustReference(
    executable,
    rasterProgramPath,
    0x0815,
    resolve(FRAME_DIRECTORY, 'rasterirq.frame'),
  );
  assertPassed('Rust VICE rasterirq_hold.prg', rasterReport);
  console.log(
    `PASS Rust VICE rasterirq_hold.prg: ${rasterReport.frames} frames, ` +
      `${rasterReport.system_cycles.toLocaleString('en-US')} system cycles.`,
  );

  const lightPenProgramPath = await ensureAsset(LIGHT_PEN_TIMING_PROGRAM);
  const lightPenReport = runRustReference(
    executable,
    lightPenProgramPath,
    0x080d,
    resolve(FRAME_DIRECTORY, 'light-pen-timing.frame'),
  );
  assertPassed('Rust VICE lp-trigger/test2.prg', lightPenReport);
  console.log(
    `PASS Rust VICE lp-trigger/test2.prg: ${lightPenReport.frames} frames, ` +
      `${lightPenReport.system_cycles.toLocaleString('en-US')} system cycles.`,
  );

  for (const [index, definition] of PIXEL_REFERENCE_DEFINITIONS.entries()) {
    const programPath = await ensureAsset(definition.program);
    const referenceBytes = await loadPinnedReferenceAsset({
      cachePath: cachePath(definition.referenceImage),
      name: `VICE ${definition.referenceImage.fileName}`,
      sha256: definition.referenceImage.sha256,
      url: definition.referenceImage.url,
    });
    const framePath = resolve(FRAME_DIRECTORY, `pixel-${index}.frame`);
    const report = runRustReference(executable, programPath, definition.entryPoint, framePath);
    const label = `Rust VICE ${definition.program.fileName}`;
    assertPassed(label, report);
    const comparedPixels = comparePaletteIndices(
      label,
      new Uint8Array(await readFile(framePath)),
      referenceBytes,
    );
    console.log(
      `PASS ${label} (${definition.description}): ` +
        `${comparedPixels.toLocaleString('en-US')} exact palette-index pixels, ` +
        `${report.frames} frames.`,
    );
  }

  console.log(
    `PASS Rust VICE VIC-II suite revision ${VICE_TEST_REVISION}: ` +
      `${PIXEL_REFERENCE_DEFINITIONS.length} exact PAL frame references.`,
  );
}

await main();
