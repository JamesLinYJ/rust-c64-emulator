// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - VICE SID noise/writeback 整机验证
//
//   文件:       verifyRustSidNoiseWritebackReference.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { resolve } from 'node:path';

import { SID_MODEL, type SidModel } from '../src/devices/SidModel';
import { loadPinnedReferenceAsset } from './reference/loadPinnedReferenceAsset';
import { runBoundedCommand } from './runRustJsonTrace';

interface ReferenceAsset {
  readonly cacheFileName: string;
  readonly directory: string;
  readonly fileName: string;
  readonly sha256: string;
}

interface TestCase {
  readonly expected: readonly number[];
  readonly label: string;
  readonly model: SidModel;
  readonly program: ReferenceAsset;
}

interface RunReport {
  readonly exit_code: number | null;
  readonly frames: number;
  readonly memory: readonly number[];
  readonly program_counter: number;
  readonly system_cycles: number;
}

const REVISION = 46_176;
const ROOT = `https://sourceforge.net/p/vice-emu/code/${REVISION}/tree/testprogs/SID`;
const MATRIX = [0xff, 0xfe, 0xfc, 0xfc, 0xfc, 0xf8, 0xf8, 0xf8, 0xf8, 0xf0, 0xf0] as const;
const FIRMWARE_PATHS = [
  resolve('public/firmware/basic.901226-01.bin'),
  resolve('public/firmware/characters.901225-01.bin'),
  resolve('public/firmware/kernal.901227-03.bin'),
] as const;

const test1: ReferenceAsset = {
  cacheFileName: 'sid-noise-writeback-test1.prg',
  directory: 'noisewriteback',
  fileName: 'noise_writeback_test1-old.prg',
  sha256: '4f85095b30b9b32260d0e993d03ba67bcdb215ac9bbb9b26ab922638ba7f93dc',
};
const test2Old: ReferenceAsset = {
  cacheFileName: 'sid-noise-writeback-test2-old.prg',
  directory: 'noisewriteback',
  fileName: 'noise_writeback_test2-old.prg',
  sha256: '9ee3ac86b997d65bbf7b6126a1ae336def638e754724fd9843320ef9b34d3b94',
};
const test2New: ReferenceAsset = {
  cacheFileName: 'sid-noise-writeback-test2-new.prg',
  directory: 'noisewriteback',
  fileName: 'noise_writeback_test2-new.prg',
  sha256: '39429fd6fd47e436b40adadac2fd2b65baf5a9c0ae616ddb090fbb17aee86e9e',
};
const matrix8: ReferenceAsset = {
  cacheFileName: 'sid-noise-writeback-8-to-8.prg',
  directory: 'wb_testsuite',
  fileName: 'noise_writeback_check_8_to_8_old.prg',
  sha256: '87c8204509171302e7ff6730ff07b06f3291cf11c6fa9aa9e49faba18db63f08',
};
const matrix9: ReferenceAsset = {
  cacheFileName: 'sid-noise-writeback-9-to-8.prg',
  directory: 'wb_testsuite',
  fileName: 'noise_writeback_check_9_to_8_old.prg',
  sha256: '6bd27be17f983a5501df1671ffe3bf6ba0b3741d6cc9849a8fda65f07bc70a68',
};

const cases: readonly TestCase[] = [
  { expected: [0xfe, 0xfe], label: 'test1-old', model: SID_MODEL.mos6581, program: test1 },
  { expected: [0xfe, 0xfe], label: 'test1-new', model: SID_MODEL.mos8580, program: test1 },
  { expected: [0x00, 0x14], label: 'test2-old', model: SID_MODEL.mos6581, program: test2Old },
  { expected: [0x00, 0x12], label: 'test2-new', model: SID_MODEL.mos8580, program: test2New },
  { expected: MATRIX, label: '8-to-8-old', model: SID_MODEL.mos6581, program: matrix8 },
  { expected: MATRIX, label: '8-to-8-new', model: SID_MODEL.mos8580, program: matrix8 },
  { expected: MATRIX, label: '9-to-8-old', model: SID_MODEL.mos6581, program: matrix9 },
  { expected: MATRIX, label: '9-to-8-new', model: SID_MODEL.mos8580, program: matrix9 },
];

function parseReport(stdout: string): RunReport {
  const line = stdout.trim().split(/\r?\n/u).at(-1);
  if (line === undefined) throw new Error('Rust VICE SID runner produced no report.');
  return JSON.parse(line) as RunReport;
}

async function main(): Promise<void> {
  const programs = new Map<ReferenceAsset, string>();
  for (const program of new Set(cases.map((test) => test.program))) {
    const path = resolve('output/reference', program.cacheFileName);
    await loadPinnedReferenceAsset({
      cachePath: path,
      name: `VICE ${program.fileName}`,
      sha256: program.sha256,
      url: `${ROOT}/${program.directory}/${program.fileName}?format=raw`,
    });
    programs.set(program, path);
  }

  await runBoundedCommand({
    arguments: [
      'build',
      '--quiet',
      '--release',
      '--locked',
      '-p',
      'c64-core',
      '--example',
      'vice_sid',
    ],
    command: 'cargo',
    label: 'Rust VICE SID runner build',
    maximumOutputBytes: 4 * 1024 * 1024,
  });
  const executable = resolve(
    `target/release/examples/vice_sid${process.platform === 'win32' ? '.exe' : ''}`,
  );

  const results: string[] = [];
  for (const test of cases) {
    const programPath = programs.get(test.program);
    if (programPath === undefined)
      throw new Error(`Missing pinned program ${test.program.fileName}.`);
    const stdout = await runBoundedCommand({
      arguments: [...FIRMWARE_PATHS, programPath, test.model, String(test.expected.length)],
      command: executable,
      label: `Rust VICE SID ${test.label}/${test.model}`,
      maximumOutputBytes: 4 * 1024 * 1024,
      timeoutMilliseconds: 120_000,
    });
    const report = parseReport(stdout);
    if (report.exit_code !== 0 || !isExact(report.memory, test.expected)) {
      throw new Error(
        `Rust ${test.label}/${test.model} failed: exit=${String(report.exit_code)}, ` +
          `OSC3=${report.memory.map(hexByte).join(' ')}, PC=$${report.program_counter.toString(16).padStart(4, '0')}.`,
      );
    }
    results.push(
      `${test.label}/${test.model} (${report.frames} frames, ${report.system_cycles.toLocaleString('en-US')} cycles)`,
    );
  }
  console.log(`PASS Rust VICE SID noise writeback revision ${REVISION}: ${results.join('; ')}.`);
}

function isExact(actual: readonly number[], expected: readonly number[]): boolean {
  return (
    actual.length === expected.length && actual.every((value, index) => value === expected[index])
  );
}

function hexByte(value: number): string {
  return value.toString(16).padStart(2, '0');
}

await main();
