// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - TypeScript/Rust VIC timing differential test
//
//   File:       verifyRustVicTimingDifferential.ts
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';

import type { VicCycleResult } from '../src/devices/VicCycleSequencer';
import { VicCycleSequencer } from '../src/devices/VicCycleSequencer';
import { VicBorderController } from '../src/devices/VicBorderController';

interface TickOperation {
  readonly kind: 'tick';
  readonly columnSelect: boolean;
  readonly displayEnabled: boolean;
  readonly rowSelect: boolean;
  readonly spriteEnableMask: number;
  readonly spriteVerticalExpansionMask: number;
  readonly verticalScroll: number;
  readonly spriteY: readonly number[];
}

interface WriteExpansionOperation {
  readonly kind: 'writeExpansion';
  readonly value: number;
}

interface ResetOperation {
  readonly kind: 'reset';
}

type Operation = ResetOperation | TickOperation | WriteExpansionOperation;

interface Observation {
  readonly borderPixelMask: number | null;
  readonly resultFlags: number | null;
  readonly cycle: number;
  readonly rasterLine: number;
  readonly completedRasterLine: number | null;
  readonly lateReloadColumn: number | null;
  readonly matrixAccessCode: number | null;
  readonly phi1Code: number | null;
  readonly phi2Code: number | null;
  readonly spritePhi1Offset: number | null;
  readonly spritePhi2Offset: number | null;
  readonly spriteDisplayMask: number;
  readonly spriteDmaMask: number;
  readonly currentFlags: number;
}

const OPERATION_COUNT = 60_000;

function fixedRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return state >>> 0;
  };
}

function buildOperations(): readonly Operation[] {
  const random = fixedRandom(0x6569_2026);
  const spriteY = Array.from({ length: 8 }, () => 0);
  const operations: Operation[] = [];
  for (let index = 0; index < OPERATION_COUNT; index += 1) {
    const choice = random() % 128;
    if (choice === 0) {
      operations.push({ kind: 'reset' });
      continue;
    }
    if (choice <= 4) {
      operations.push({ kind: 'writeExpansion', value: random() & 0xff });
      continue;
    }
    if ((random() & 0x0f) === 0) spriteY[random() & 0x07] = random() & 0xff;
    operations.push({
      kind: 'tick',
      columnSelect: (random() & 1) !== 0,
      displayEnabled: (random() & 0x07) !== 0,
      rowSelect: (random() & 1) !== 0,
      spriteEnableMask: random() & 0xff,
      spriteVerticalExpansionMask: random() & 0xff,
      verticalScroll: random() & 0x07,
      spriteY: [...spriteY],
    });
  }
  return operations;
}

function encodePhi1(fetch: VicCycleResult['busSchedule']['phi1']): number {
  switch (fetch.kind) {
    case 'graphics':
      return 0;
    case 'idle':
      return 1;
    case 'refresh':
      return 2;
    case 'spriteData':
      return 3 | (fetch.spriteIndex << 8) | (fetch.byteIndex << 12);
    case 'spritePointer':
      return 4 | (fetch.spriteIndex << 8);
  }
}

function encodePhi2(fetch: NonNullable<VicCycleResult['busSchedule']['phi2']>): number {
  return fetch.kind === 'matrix' ? 0 : 1 | (fetch.spriteIndex << 8) | (fetch.byteIndex << 12);
}

function resultFlags(result: VicCycleResult): number {
  return (
    Number(result.aecLow) |
    (Number(result.baLow) << 1) |
    (Number(result.badLine) << 2) |
    (Number(result.badLineCondition) << 3) |
    (Number(result.enterDisplayState) << 4) |
    (Number(result.frameStarted) << 5) |
    (Number(result.lineStarted) << 6) |
    (Number(result.resetRowCounter) << 7)
  );
}

function currentFlags(sequencer: VicCycleSequencer): number {
  return (
    Number(sequencer.aecLow) | (Number(sequencer.baLow) << 1) | (Number(sequencer.badLine) << 2)
  );
}

function encodeResult(result: VicCycleResult, borderPixelMask: number): Observation {
  return {
    borderPixelMask,
    resultFlags: resultFlags(result),
    cycle: result.cycle,
    rasterLine: result.rasterLine,
    completedRasterLine: result.completedRasterLine ?? null,
    lateReloadColumn: result.lateVideoCounterReloadColumn ?? null,
    matrixAccessCode:
      result.matrixAccess === undefined
        ? null
        : result.matrixAccess.column |
          ((result.matrixAccess.source === 'videoMemory' ? 1 : 0) << 8),
    phi1Code: encodePhi1(result.busSchedule.phi1),
    phi2Code: result.busSchedule.phi2 === undefined ? null : encodePhi2(result.busSchedule.phi2),
    spritePhi1Offset: result.spriteDataOffsets.phi1 ?? null,
    spritePhi2Offset: result.spriteDataOffsets.phi2 ?? null,
    spriteDisplayMask: result.spriteDisplayMask,
    spriteDmaMask: result.spriteDmaMask,
    currentFlags:
      Number(result.aecLow) | (Number(result.baLow) << 1) | (Number(result.badLine) << 2),
  };
}

function observeWithoutTick(sequencer: VicCycleSequencer): Observation {
  return {
    borderPixelMask: null,
    resultFlags: null,
    cycle: sequencer.cycle,
    rasterLine: sequencer.rasterLine,
    completedRasterLine: null,
    lateReloadColumn: null,
    matrixAccessCode: null,
    phi1Code: null,
    phi2Code: null,
    spritePhi1Offset: null,
    spritePhi2Offset: null,
    spriteDisplayMask: sequencer.spriteDisplayMask,
    spriteDmaMask: sequencer.spriteDmaMask,
    currentFlags: currentFlags(sequencer),
  };
}

function runTypeScript(operations: readonly Operation[]): readonly Observation[] {
  const sequencer = new VicCycleSequencer();
  const border = new VicBorderController();
  return operations.map((operation) => {
    if (operation.kind === 'reset') {
      sequencer.reset();
      border.reset();
      return observeWithoutTick(sequencer);
    }
    if (operation.kind === 'writeExpansion') {
      sequencer.writeSpriteVerticalExpansionRegister(operation.value);
      return observeWithoutTick(sequencer);
    }
    const spriteY = operation.spriteY;
    const result = sequencer.tick({
      displayEnabled: operation.displayEnabled,
      spriteEnableMask: operation.spriteEnableMask,
      spriteVerticalExpansionMask: operation.spriteVerticalExpansionMask,
      verticalScroll: operation.verticalScroll,
      spriteY: (spriteIndex) => spriteY[spriteIndex] ?? 0,
    });
    return encodeResult(
      result,
      border.tickCycle(
        operation.columnSelect,
        operation.displayEnabled,
        result.cycle,
        result.rasterLine,
        operation.rowSelect,
      ),
    );
  });
}

function runRust(operations: readonly Operation[]): readonly Observation[] {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'vic_timing_trace'],
    {
      cwd: process.cwd(),
      encoding: 'utf8',
      input: JSON.stringify(operations),
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Rust VIC timing adapter failed (${String(result.status)}):\n${result.stderr}`);
  }
  return JSON.parse(result.stdout) as readonly Observation[];
}

const operations = buildOperations();
const expected = runTypeScript(operations);
const actual = runRust(operations);
if (actual.length !== expected.length) throw new Error('Rust VIC timing trace length mismatch.');
for (let index = 0; index < expected.length; index += 1) {
  if (JSON.stringify(actual[index]) !== JSON.stringify(expected[index])) {
    const start = Math.max(0, index - 32);
    throw new Error(
      `VIC timing mismatch after operation ${index} ${JSON.stringify(operations[index])}: ` +
        `expected ${JSON.stringify(expected[index])}, received ${JSON.stringify(actual[index])}. ` +
        `Recent operations: ${JSON.stringify(operations.slice(start, index + 1))}.`,
    );
  }
}

console.log(
  `PASS Rust/TypeScript VIC timing differential: ${operations.length} deterministic ` +
    'raster/bad-line/border/sprite-DMA operations with exact BA/AEC and pixel masks.',
);
