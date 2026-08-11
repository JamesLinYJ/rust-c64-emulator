// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II Rust 像素管线差分验证
//
//   文件:       verifyRustVicPixelDifferential.ts
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';

import { C64_PALETTE } from '../src/devices/VicII';
import { VicCycleSequencer } from '../src/devices/VicCycleSequencer';
import {
  VicPixelPipeline,
  type VicPixelCollisionSink,
  type VicPixelDataSource,
  type VicPixelRegisters,
} from '../src/devices/VicPixelPipeline';
import { VicSprite } from '../src/devices/VicSprite';

interface TickOperation {
  readonly kind: 'tick';
  readonly backgroundColorIndices: readonly [number, number, number, number];
  readonly borderColorIndex: number;
  readonly borderPixelMask: number;
  readonly displayMode: number;
  readonly horizontalScroll: number;
  readonly screenVisible: boolean;
  readonly sourceSeed: number;
  readonly spriteDisplayMask: number;
  readonly spriteMulticolor0Index: number;
  readonly spriteMulticolor1Index: number;
  readonly spriteSeed: number;
}

interface ResetOperation {
  readonly kind: 'reset';
}

type Operation = ResetOperation | TickOperation;

interface Observation {
  readonly centerPixel: number;
  readonly firstPixel: number;
  readonly lastPixel: number;
  readonly lineHash: number;
  readonly spriteForegroundMask: number;
  readonly spriteSpriteMask: number;
}

const OPERATION_COUNT = 65_000;
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
  const random = fixedRandom(0x91ce_6569);
  const operations: Operation[] = [];
  for (let index = 0; index < OPERATION_COUNT; index += 1) {
    if ((random() & 0x03ff) === 0) {
      operations.push({ kind: 'reset' });
      continue;
    }
    operations.push({
      kind: 'tick',
      backgroundColorIndices: [random() & 0x0f, random() & 0x0f, random() & 0x0f, random() & 0x0f],
      borderColorIndex: random() & 0x0f,
      borderPixelMask: random() & 0xff,
      displayMode: random() & 0x07,
      horizontalScroll: random() & 0x07,
      screenVisible: (random() & 0x07) !== 0,
      sourceSeed: random(),
      spriteDisplayMask: random() & 0xff,
      spriteMulticolor0Index: random() & 0x0f,
      spriteMulticolor1Index: random() & 0x0f,
      spriteSeed: random(),
    });
  }
  return operations;
}

function mixWord(seed: number, index: number, salt: number): number {
  let value = (seed ^ Math.imul((index + 1) >>> 0, 0x9e37_79b9) ^ (salt >>> 0)) >>> 0;
  value = Math.imul((value ^ (value >>> 16)) >>> 0, 0x7feb_352d) >>> 0;
  value = Math.imul((value ^ (value >>> 15)) >>> 0, 0x846c_a68b) >>> 0;
  return (value ^ (value >>> 16)) >>> 0;
}

class PatternPixelSource implements VicPixelDataSource {
  sourceSeed = 0;
  spriteDisplayMask = 0;

  screenMatrixByte(column: number): number {
    return mixWord(this.sourceSeed, column, 0x1001) & 0xff;
  }

  colorMatrixNibble(column: number): number {
    return mixWord(this.sourceSeed, column, 0x1002) & 0x0f;
  }

  graphicsByte(column: number): number {
    return mixWord(this.sourceSeed, column, 0x1003) & 0xff;
  }

  spriteDataWord(spriteIndex: number): number {
    return mixWord(this.sourceSeed, spriteIndex, 0x1004) & 0x00ff_ffff;
  }
}

class CollisionSink implements VicPixelCollisionSink {
  spriteForegroundMask = 0;
  spriteSpriteMask = 0;

  reset(): void {
    this.spriteForegroundMask = 0;
    this.spriteSpriteMask = 0;
  }

  recordSpriteForegroundCollision(spriteMask: number): void {
    this.spriteForegroundMask |= spriteMask;
  }

  recordSpriteSpriteCollision(spriteMask: number): void {
    this.spriteSpriteMask |= spriteMask;
  }
}

function configureSprites(sprites: readonly VicSprite[], seed: number): void {
  for (let index = 0; index < sprites.length; index += 1) {
    const sprite = sprites[index];
    if (!sprite) continue;
    const value = mixWord(seed, index, 0x5350_5254);
    sprite.x = value & 0x01ff;
    sprite.color = C64_PALETTE[(value >>> 12) & 0x0f] ?? C64_PALETTE[0];
    sprite.foreground = (value & (1 << 9)) !== 0;
    sprite.multicolor = (value & (1 << 10)) !== 0;
    sprite.expandHorizontal = (value & (1 << 11)) !== 0;
  }
}

function hashLine(pixels: Uint32Array): number {
  let hash = FNV_OFFSET_BASIS;
  for (const pixel of pixels) {
    for (let shift = 0; shift < 32; shift += 8) {
      hash = Math.imul((hash ^ ((pixel >>> shift) & 0xff)) >>> 0, FNV_PRIME) >>> 0;
    }
  }
  return hash;
}

function observe(pipeline: VicPixelPipeline, collisions: CollisionSink): Observation {
  const pixels = pipeline.snapshot();
  return {
    centerPixel: pixels[201] ?? 0,
    firstPixel: pixels[0] ?? 0,
    lastPixel: pixels[402] ?? 0,
    lineHash: hashLine(pixels),
    spriteForegroundMask: collisions.spriteForegroundMask & 0xff,
    spriteSpriteMask: collisions.spriteSpriteMask & 0xff,
  };
}

function paletteColor(index: number): number {
  return C64_PALETTE[index & 0x0f] ?? C64_PALETTE[0];
}

function runTypeScript(operations: readonly Operation[]): readonly Observation[] {
  const sequencer = new VicCycleSequencer();
  const pipeline = new VicPixelPipeline();
  const source = new PatternPixelSource();
  const collisions = new CollisionSink();
  const sprites = C64_PALETTE.slice(0, 8).map((color) => new VicSprite(color));
  pipeline.reset(C64_PALETTE[0]);

  return operations.map((operation) => {
    collisions.reset();
    if (operation.kind === 'reset') {
      sequencer.reset();
      pipeline.reset(C64_PALETTE[0]);
      return observe(pipeline, collisions);
    }

    source.sourceSeed = operation.sourceSeed;
    source.spriteDisplayMask = operation.spriteDisplayMask;
    configureSprites(sprites, operation.spriteSeed);
    const registers: VicPixelRegisters = {
      backgroundColors: operation.backgroundColorIndices.map(paletteColor),
      bitmapMode: (operation.displayMode & 0x02) !== 0,
      borderColor: paletteColor(operation.borderColorIndex),
      displayModeValid: operation.displayMode <= 4,
      extendedBackgroundMode: (operation.displayMode & 0x04) !== 0,
      horizontalScroll: operation.horizontalScroll,
      multicolorMode: (operation.displayMode & 0x01) !== 0,
      palette: C64_PALETTE,
      screenVisible: operation.screenVisible,
      spriteMulticolor0: paletteColor(operation.spriteMulticolor0Index),
      spriteMulticolor1: paletteColor(operation.spriteMulticolor1Index),
      sprites,
    };
    const cycle = sequencer.tick({
      displayEnabled: false,
      spriteEnableMask: 0,
      spriteVerticalExpansionMask: 0,
      spriteY: () => 0,
      verticalScroll: 0,
    });
    pipeline.clockCycle(cycle, operation.borderPixelMask, registers, source, collisions);
    return observe(pipeline, collisions);
  });
}

function runRust(operations: readonly Operation[]): readonly Observation[] {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'vic_pixel_trace'],
    {
      cwd: process.cwd(),
      encoding: 'utf8',
      input: JSON.stringify(operations),
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Rust VIC pixel adapter failed (${String(result.status)}):\n${result.stderr}`);
  }
  return JSON.parse(result.stdout) as readonly Observation[];
}

const operations = buildOperations();
const expected = runTypeScript(operations);
const actual = runRust(operations);
if (actual.length !== expected.length) throw new Error('Rust VIC pixel trace length mismatch.');
for (let index = 0; index < expected.length; index += 1) {
  if (JSON.stringify(actual[index]) !== JSON.stringify(expected[index])) {
    const start = Math.max(0, index - 24);
    throw new Error(
      `VIC pixel mismatch after operation ${index} ${JSON.stringify(operations[index])}: ` +
        `expected ${JSON.stringify(expected[index])}, received ${JSON.stringify(actual[index])}. ` +
        `Recent operations: ${JSON.stringify(operations.slice(start, index + 1))}.`,
    );
  }
}

console.log(
  `PASS Rust/TypeScript VIC pixel differential: ${operations.length} deterministic ` +
    'display-mode, scroll, border, sprite-priority and collision operations with exact line hashes.',
);
