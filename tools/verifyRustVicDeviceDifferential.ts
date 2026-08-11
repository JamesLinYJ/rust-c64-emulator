// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II Rust 整芯片差分验证
//
//   文件:       verifyRustVicDeviceDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { runRustJsonTraceSync } from './runRustJsonTrace';

import { VicII, type VicRasterLineSnapshot } from '../src/devices/VicII';
import type { VicFetchSnapshot } from '../src/devices/VicFetchPipeline';
import type { VicMemoryBus } from '../src/devices/VicMemoryBus';
import type { VicSprite } from '../src/devices/VicSprite';

interface TickOperation {
  readonly kind: 'tick';
  readonly cpuDataBusValue: number;
}

interface ReadOperation {
  readonly kind: 'read';
  readonly address: number;
}

interface WriteOperation {
  readonly kind: 'write';
  readonly address: number;
  readonly value: number;
}

interface SetLightPenOperation {
  readonly kind: 'setLightPen';
  readonly high: boolean;
}

interface LatchLightPenOperation {
  readonly kind: 'latchLightPen';
  readonly x: number;
  readonly y: number;
}

interface ResetOperation {
  readonly kind: 'reset';
}

type Operation =
  | LatchLightPenOperation
  | ReadOperation
  | ResetOperation
  | SetLightPenOperation
  | TickOperation
  | WriteOperation;

interface Observation {
  readonly borderHash: number;
  readonly currentFlags: number;
  readonly byteReads: number;
  readonly colorReads: number;
  readonly cycle: number;
  readonly displayState: number;
  readonly fetchHash: number;
  readonly interruptPending: boolean;
  readonly phi1DataBusValue: number;
  readonly pixelHash: number;
  readonly rasterLine: number;
  readonly readValue: number | null;
  readonly spriteDmaMask: number;
  readonly spriteDataWords: readonly number[];
  readonly spriteDisplayMask: number;
  readonly spriteFlags: readonly number[];
  readonly spriteHash: number;
  readonly stateHash: number;
}

const OPERATION_COUNT = 45_000;
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
  const random = fixedRandom(0xd3a1_6569);
  const operations: Operation[] = [];
  for (let index = 0; index < OPERATION_COUNT; index += 1) {
    const choice = random() & 0x07ff;
    if (choice === 0) {
      operations.push({ kind: 'reset' });
    } else if (choice < 1_680) {
      operations.push({ kind: 'tick', cpuDataBusValue: random() & 0xff });
    } else if (choice < 1_860) {
      operations.push({
        kind: 'write',
        address: 0xd000 | (random() & 0x03ff),
        value: random() & 0xff,
      });
    } else if (choice < 2_000) {
      operations.push({ kind: 'read', address: 0xd000 | (random() & 0x03ff) });
    } else if (choice < 2_024) {
      operations.push({ kind: 'setLightPen', high: (random() & 1) !== 0 });
    } else {
      operations.push({
        kind: 'latchLightPen',
        x: random() & 0x03ff,
        y: random() & 0x03ff,
      });
    }
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
    return mixByte(addressInBank & 0x3fff, this.byteReads, 0x6569_1001);
  }

  readVicColor(index: number): number {
    this.colorReads = (this.colorReads + 1) >>> 0;
    return mixByte(index & 0x03ff, this.colorReads, 0x6569_1002) & 0x0f;
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

function hashFetch(hash: number, snapshot: VicFetchSnapshot): number {
  let result = hashBytes(hash, snapshot.colorMatrix);
  result = hashBytes(result, snapshot.graphics);
  result = hashByte(result, Number(snapshot.idleState));
  result = hashByte(result, snapshot.lastPhi1Byte);
  result = hashByte(result, snapshot.lastPhi2Byte);
  result = hashWords(result, snapshot.spriteData);
  result = hashByte(result, snapshot.spriteDisplayMask);
  result = hashBytes(result, snapshot.spritePointers);
  result = hashByte(result, snapshot.matrixIndex);
  result = hashWords(result, snapshot.pendingSpriteData);
  result = hashBytes(result, snapshot.pendingSpritePointers);
  result = hashByte(result, snapshot.refreshCounter);
  result = hashByte(result, snapshot.rowCounter);
  result = hashBytes(result, snapshot.screenMatrix);
  result = hashU16(result, snapshot.videoCounter);
  return hashU16(result, snapshot.videoCounterBase);
}

function hashSprites(hash: number, sprites: readonly VicSprite[]): number {
  let result = hash;
  for (const sprite of sprites) {
    result = hashU16(result, sprite.x);
    result = hashByte(result, sprite.y);
    result = hashU32(result, sprite.color);
    result = hashByte(result, Number(sprite.enabled));
    result = hashByte(result, Number(sprite.foreground));
    result = hashByte(result, Number(sprite.multicolor));
    result = hashByte(result, Number(sprite.expandVertical));
    result = hashByte(result, Number(sprite.expandHorizontal));
    result = hashByte(result, Number(sprite.collisionWithSprite));
    result = hashByte(result, Number(sprite.collisionWithForeground));
  }
  return result;
}

function stateHash(snapshot: VicRasterLineSnapshot, sprites: readonly VicSprite[]): number {
  let hash = hashWords(FNV_OFFSET_BASIS, snapshot.borderColors);
  hash = hashBytes(hash, snapshot.borderPixelMasks);
  hash = hashFetch(hash, snapshot.fetchState);
  hash = hashWords(hash, snapshot.pixels);
  return hashSprites(hash, sprites);
}

function observe(vic: VicII, memory: PatternMemory, readValue: number | null): Observation {
  const snapshot = vic.captureRasterLineState();
  return {
    borderHash: hashBytes(
      hashWords(FNV_OFFSET_BASIS, snapshot.borderColors),
      snapshot.borderPixelMasks,
    ),
    currentFlags: Number(vic.aecLow) | (Number(vic.baLow) << 1) | (Number(vic.badLine) << 2),
    byteReads: memory.byteReads,
    colorReads: memory.colorReads,
    cycle: vic.currentRasterCycle,
    displayState:
      Number(vic.screenVisible) |
      (vic.displayMode << 1) |
      (vic.horizontalScroll << 4) |
      (Number(vic.screenWidth === 40) << 7) |
      (Number(vic.screenHeight === 25) << 8),
    fetchHash: hashFetch(FNV_OFFSET_BASIS, snapshot.fetchState),
    interruptPending: vic.interruptPending,
    phi1DataBusValue: vic.phi1DataBusValue,
    pixelHash: hashWords(FNV_OFFSET_BASIS, snapshot.pixels),
    rasterLine: vic.currentRasterLine,
    readValue,
    spriteDmaMask: vic.spriteDmaMask,
    spriteDataWords: [...snapshot.fetchState.spriteData],
    spriteDisplayMask: snapshot.fetchState.spriteDisplayMask,
    spriteFlags: vic.sprites.map(
      (sprite) =>
        Number(sprite.enabled) |
        (Number(sprite.foreground) << 1) |
        (Number(sprite.multicolor) << 2) |
        (Number(sprite.expandVertical) << 3) |
        (Number(sprite.expandHorizontal) << 4) |
        (Number(sprite.collisionWithSprite) << 5) |
        (Number(sprite.collisionWithForeground) << 6),
    ),
    spriteHash: hashSprites(FNV_OFFSET_BASIS, vic.sprites),
    stateHash: stateHash(snapshot, vic.sprites),
  };
}

function runTypeScript(operations: readonly Operation[]): readonly Observation[] {
  const vic = new VicII();
  const memory = new PatternMemory();
  return operations.map((operation) => {
    let readValue: number | null = null;
    switch (operation.kind) {
      case 'tick':
        memory.cpuDataBusValue = operation.cpuDataBusValue;
        vic.clockCycle(memory);
        break;
      case 'read':
        readValue = vic.read(operation.address);
        break;
      case 'write':
        vic.write(operation.address, operation.value);
        break;
      case 'setLightPen':
        vic.setLightPenInputHigh(operation.high);
        break;
      case 'latchLightPen':
        vic.latchLightPen(operation.x, operation.y);
        break;
      case 'reset':
        vic.reset();
        break;
    }
    return observe(vic, memory, readValue);
  });
}

function runRust(operations: readonly Operation[]): readonly Observation[] {
  return runRustJsonTraceSync<readonly Observation[]>({
    example: 'vic_device_trace',
    input: operations,
    label: 'Rust VIC device adapter',
    maximumOutputBytes: 64 * 1024 * 1024,
  });
}

const operations = buildOperations();
const expected = runTypeScript(operations);
const actual = runRust(operations);
if (actual.length !== expected.length) throw new Error('Rust VIC device trace length mismatch.');
for (let index = 0; index < expected.length; index += 1) {
  if (JSON.stringify(actual[index]) !== JSON.stringify(expected[index])) {
    const start = Math.max(0, index - 24);
    throw new Error(
      `VIC device mismatch after operation ${index} ${JSON.stringify(operations[index])}: ` +
        `expected ${JSON.stringify(expected[index])}, received ${JSON.stringify(actual[index])}. ` +
        `Recent operations: ${JSON.stringify(operations.slice(start, index + 1))}.`,
    );
  }
}

console.log(
  `PASS Rust/TypeScript VIC device differential: ${operations.length} deterministic ` +
    'ticks, mirrored register accesses, IRQ/collision/light-pen events and exact raster hashes.',
);
