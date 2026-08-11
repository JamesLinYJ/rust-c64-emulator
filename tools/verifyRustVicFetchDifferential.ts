// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II Rust 取数流水线差分验证
//
//   文件:       verifyRustVicFetchDifferential.ts
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';

import { VicCycleSequencer } from '../src/devices/VicCycleSequencer';
import {
  VicFetchPipeline,
  type VicFetchRegisters,
  type VicFetchSnapshot,
} from '../src/devices/VicFetchPipeline';
import type { VicMemoryBus } from '../src/devices/VicMemoryBus';

interface TickOperation extends VicFetchRegisters {
  readonly kind: 'tick';
  readonly cpuDataBusValue: number;
  readonly displayEnabled: boolean;
  readonly spriteEnableMask: number;
  readonly spriteVerticalExpansionMask: number;
  readonly spriteY: readonly number[];
  readonly verticalScroll: number;
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
  readonly byteReads: number;
  readonly colorReads: number;
  readonly phi1DataBusValue: number;
  readonly spriteDisplayMask: number;
  readonly stateHash: number;
}

const OPERATION_COUNT = 70_000;
const FNV_OFFSET_BASIS = 0x811c9dc5;
const FNV_PRIME = 0x01000193;

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
  const random = fixedRandom(0xf37c_6569);
  const spriteY = Array.from({ length: 8 }, () => 0);
  const operations: Operation[] = [];
  for (let index = 0; index < OPERATION_COUNT; index += 1) {
    const choice = random() & 0xff;
    if (choice === 0) {
      operations.push({ kind: 'reset' });
      continue;
    }
    if (choice <= 3) {
      operations.push({ kind: 'writeExpansion', value: random() & 0xff });
      continue;
    }
    if ((random() & 0x0f) === 0) spriteY[random() & 0x07] = random() & 0xff;
    operations.push({
      kind: 'tick',
      bitmapMemoryAddress: (random() & 1) << 13,
      bitmapMode: (random() & 1) !== 0,
      characterMemoryAddress: (random() & 0x07) << 11,
      cpuDataBusValue: random() & 0xff,
      displayEnabled: (random() & 0x07) !== 0,
      extendedBackgroundMode: (random() & 1) !== 0,
      screenMemoryAddress: (random() & 0x0f) << 10,
      spriteEnableMask: random() & 0xff,
      spriteVerticalExpansionMask: random() & 0xff,
      spriteY: [...spriteY],
      verticalScroll: random() & 0x07,
    });
  }
  return operations;
}

function mixByte(address: number, counter: number, salt: number): number {
  let value =
    (Math.imul(address >>> 0, 0x045d9f3b) + Math.imul(counter >>> 0, 0x119de1f3) + salt) >>> 0;
  value = (value ^ (value >>> 16)) >>> 0;
  value = Math.imul(value, 0x27d4eb2d) >>> 0;
  return (value ^ (value >>> 15)) & 0xff;
}

class PatternMemory implements VicMemoryBus {
  cpuDataBusValue = 0;
  byteReads = 0;
  colorReads = 0;

  readVicByte(addressInBank: number): number {
    this.byteReads = (this.byteReads + 1) >>> 0;
    return mixByte(addressInBank & 0x3fff, this.byteReads, 0x6569_0001);
  }

  readVicColor(index: number): number {
    this.colorReads = (this.colorReads + 1) >>> 0;
    return mixByte(index & 0x03ff, this.colorReads, 0x6569_0002) & 0x0f;
  }

  reset(): void {
    this.cpuDataBusValue = 0;
    this.byteReads = 0;
    this.colorReads = 0;
  }
}

function hashByte(hash: number, value: number): number {
  return Math.imul((hash ^ (value & 0xff)) >>> 0, FNV_PRIME) >>> 0;
}

function hashU16(hash: number, value: number): number {
  return hashByte(hashByte(hash, value), value >>> 8);
}

function hashU32(hash: number, value: number): number {
  return hashU16(hashU16(hash, value), value >>> 16);
}

function hashBytes(hash: number, values: Uint8Array): number {
  let result = hash;
  for (const value of values) result = hashByte(result, value);
  return result;
}

function hashWords(hash: number, values: Uint32Array): number {
  let result = hash;
  for (const value of values) result = hashU32(result, value);
  return result;
}

function stateHash(snapshot: VicFetchSnapshot): number {
  let hash = FNV_OFFSET_BASIS;
  hash = hashBytes(hash, snapshot.colorMatrix);
  hash = hashBytes(hash, snapshot.graphics);
  hash = hashByte(hash, Number(snapshot.idleState));
  hash = hashByte(hash, snapshot.lastPhi1Byte);
  hash = hashByte(hash, snapshot.lastPhi2Byte);
  hash = hashWords(hash, snapshot.spriteData);
  hash = hashByte(hash, snapshot.spriteDisplayMask);
  hash = hashBytes(hash, snapshot.spritePointers);
  hash = hashByte(hash, snapshot.matrixIndex);
  hash = hashWords(hash, snapshot.pendingSpriteData);
  hash = hashBytes(hash, snapshot.pendingSpritePointers);
  hash = hashByte(hash, snapshot.refreshCounter);
  hash = hashByte(hash, snapshot.rowCounter);
  hash = hashBytes(hash, snapshot.screenMatrix);
  hash = hashU16(hash, snapshot.videoCounter);
  return hashU16(hash, snapshot.videoCounterBase);
}

function observe(fetch: VicFetchPipeline, memory: PatternMemory): Observation {
  return {
    byteReads: memory.byteReads,
    colorReads: memory.colorReads,
    phi1DataBusValue: fetch.phi1DataBusValue,
    spriteDisplayMask: fetch.spriteDisplayMask,
    stateHash: stateHash(fetch.snapshot()),
  };
}

function runTypeScript(operations: readonly Operation[]): readonly Observation[] {
  const sequencer = new VicCycleSequencer();
  const fetch = new VicFetchPipeline();
  const memory = new PatternMemory();
  return operations.map((operation) => {
    if (operation.kind === 'reset') {
      sequencer.reset();
      fetch.reset();
      memory.reset();
      return observe(fetch, memory);
    }
    if (operation.kind === 'writeExpansion') {
      sequencer.writeSpriteVerticalExpansionRegister(operation.value);
      return observe(fetch, memory);
    }

    memory.cpuDataBusValue = operation.cpuDataBusValue;
    const cycle = sequencer.tick({
      displayEnabled: operation.displayEnabled,
      spriteEnableMask: operation.spriteEnableMask,
      spriteVerticalExpansionMask: operation.spriteVerticalExpansionMask,
      verticalScroll: operation.verticalScroll,
      spriteY: (spriteIndex) => operation.spriteY[spriteIndex] ?? 0,
    });
    fetch.executeCycle(cycle, operation, memory);
    return observe(fetch, memory);
  });
}

function runRust(operations: readonly Operation[]): readonly Observation[] {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'vic_fetch_trace'],
    {
      cwd: process.cwd(),
      encoding: 'utf8',
      input: JSON.stringify(operations),
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Rust VIC fetch adapter failed (${String(result.status)}):\n${result.stderr}`);
  }
  return JSON.parse(result.stdout) as readonly Observation[];
}

const operations = buildOperations();
const expected = runTypeScript(operations);
const actual = runRust(operations);
if (actual.length !== expected.length) throw new Error('Rust VIC fetch trace length mismatch.');
for (let index = 0; index < expected.length; index += 1) {
  if (JSON.stringify(actual[index]) !== JSON.stringify(expected[index])) {
    const start = Math.max(0, index - 24);
    throw new Error(
      `VIC fetch mismatch after operation ${index} ${JSON.stringify(operations[index])}: ` +
        `expected ${JSON.stringify(expected[index])}, received ${JSON.stringify(actual[index])}. ` +
        `Recent operations: ${JSON.stringify(operations.slice(start, index + 1))}.`,
    );
  }
}

console.log(
  `PASS Rust/TypeScript VIC fetch differential: ${operations.length} deterministic ` +
    'phi1/phi2 matrix, graphics, refresh and sprite transactions with exact state hashes.',
);
