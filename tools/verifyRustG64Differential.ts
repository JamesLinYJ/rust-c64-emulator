// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust G64 固定操作差分
//
//   文件:       verifyRustG64Differential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import {
  G64DiskImage,
  G64_LAYOUT,
  type G64SpeedMap,
  type G64SpeedZone,
} from '../src/media/G64DiskImage';

interface Probe {
  readonly byteIndices: readonly number[];
  readonly halfTrack: number;
}

type Operation =
  | {
      readonly data: readonly number[];
      readonly halfTrack: number;
      readonly kind: 'setHalfTrack';
      readonly speedMap?: SpeedMapInput;
    }
  | {
      readonly byteIndex: number;
      readonly halfTrack: number;
      readonly kind: 'writeHalfTrackByte';
      readonly speedZone?: G64SpeedZone;
      readonly value: number;
    }
  | {
      readonly kind: 'setWriteProtected';
      readonly writeProtected: boolean;
    };

type SpeedMapInput =
  | { readonly kind: 'constant'; readonly zone: G64SpeedZone }
  | { readonly kind: 'variable'; readonly packedZones: readonly number[] };

interface DifferentialInput {
  readonly imageBytes: readonly number[];
  readonly operations: readonly Operation[];
  readonly probes: readonly Probe[];
}

interface DifferentialOutput {
  readonly snapshots: readonly ImageSnapshot[];
}

interface ImageSnapshot {
  readonly halfTrackCount: number;
  readonly lastHalfTrack: number;
  readonly maximumTrackLength: number;
  readonly probes: readonly {
    readonly bytes: readonly number[] | null;
    readonly halfTrack: number;
    readonly speedZones: readonly number[];
  }[];
  readonly serializedBytes: readonly number[];
  readonly writeProtected: boolean;
}

const HALF_TRACK_COUNT = 4;
const MAXIMUM_TRACK_LENGTH = 8;
const TABLE_END = G64_LAYOUT.headerSize + HALF_TRACK_COUNT * 8;
const TRACK_BLOCK_LENGTH = 2 + MAXIMUM_TRACK_LENGTH;
const SPEED_MAP_LENGTH = Math.ceil(MAXIMUM_TRACK_LENGTH / 4);

function fixture(): Uint8Array {
  const trackOffset = TABLE_END;
  const firstSpeedMapOffset = trackOffset + TRACK_BLOCK_LENGTH;
  const secondSpeedMapOffset = firstSpeedMapOffset + SPEED_MAP_LENGTH;
  const bytes = new Uint8Array(secondSpeedMapOffset + SPEED_MAP_LENGTH);
  bytes.set(Uint8Array.from(G64_LAYOUT.signature, (character) => character.charCodeAt(0)));
  bytes[G64_LAYOUT.versionOffset] = G64_LAYOUT.supportedVersion;
  bytes[G64_LAYOUT.trackCountOffset] = HALF_TRACK_COUNT;
  writeUint16(bytes, G64_LAYOUT.maximumTrackLengthOffset, MAXIMUM_TRACK_LENGTH);
  writeUint32(bytes, G64_LAYOUT.trackOffsetTableOffset, trackOffset);

  const speedTableOffset = G64_LAYOUT.trackOffsetTableOffset + HALF_TRACK_COUNT * 4;
  writeUint32(bytes, speedTableOffset, firstSpeedMapOffset);
  writeUint32(bytes, speedTableOffset + 4, secondSpeedMapOffset);
  writeUint32(bytes, speedTableOffset + 8, 2);
  writeUint32(bytes, speedTableOffset + 12, 1);

  const trackData = Uint8Array.of(0xff, 0x55, 0xa5, 0x00, 0x81);
  writeUint16(bytes, trackOffset, trackData.length);
  bytes.set(trackData, trackOffset + 2);
  bytes.set(Uint8Array.of(0b11_10_01_00, 0b01_01_01_01), firstSpeedMapOffset);
  bytes.set(Uint8Array.of(0b00_01_10_11, 0), secondSpeedMapOffset);
  return bytes;
}

function buildInput(): DifferentialInput {
  return {
    imageBytes: [...fixture()],
    operations: [
      {
        data: [0x55, 0xaa],
        halfTrack: 3,
        kind: 'setHalfTrack',
        speedMap: { kind: 'constant', zone: 2 },
      },
      {
        byteIndex: 1,
        halfTrack: 3,
        kind: 'writeHalfTrackByte',
        speedZone: 3,
        value: 0x7e,
      },
      {
        data: [0xa5],
        halfTrack: 36,
        kind: 'setHalfTrack',
        speedMap: { kind: 'constant', zone: 2 },
      },
      {
        data: [0x96, 0x69, 0x3c],
        halfTrack: 4,
        kind: 'setHalfTrack',
        speedMap: { kind: 'variable', packedZones: [0b11_00_10_01, 0] },
      },
      { kind: 'setWriteProtected', writeProtected: true },
    ],
    probes: [2, 3, 4, 5, 36].map((halfTrack) => ({
      byteIndices: Array.from({ length: MAXIMUM_TRACK_LENGTH }, (_unused, index) => index),
      halfTrack,
    })),
  };
}

function toSpeedMap(input: SpeedMapInput): G64SpeedMap {
  return input.kind === 'constant'
    ? { kind: 'constant', zone: input.zone }
    : { kind: 'variable', packedZones: Uint8Array.from(input.packedZones) };
}

function snapshot(image: G64DiskImage, probes: readonly Probe[]): ImageSnapshot {
  return {
    halfTrackCount: image.halfTrackCount,
    lastHalfTrack: image.lastHalfTrack,
    maximumTrackLength: image.maximumTrackLength,
    probes: probes
      .map((probe) => ({
        bytes: image.readHalfTrack(probe.halfTrack)?.bytes ?? null,
        halfTrack: probe.halfTrack,
        speedZones: probe.byteIndices.map((byteIndex) =>
          image.speedZoneAtByte(probe.halfTrack, byteIndex),
        ),
      }))
      .map((probe) => ({ ...probe, bytes: probe.bytes === null ? null : [...probe.bytes] })),
    serializedBytes: [...image.toBytes()],
    writeProtected: image.writeProtected,
  };
}

function runTypeScript(input: DifferentialInput): DifferentialOutput {
  const image = new G64DiskImage(Uint8Array.from(input.imageBytes));
  const snapshots = [snapshot(image, input.probes)];
  for (const operation of input.operations) {
    switch (operation.kind) {
      case 'setHalfTrack':
        image.setHalfTrack(
          operation.halfTrack,
          Uint8Array.from(operation.data),
          operation.speedMap === undefined ? undefined : toSpeedMap(operation.speedMap),
        );
        break;
      case 'writeHalfTrackByte':
        image.writeHalfTrackByte(
          operation.halfTrack,
          operation.byteIndex,
          operation.value,
          operation.speedZone,
        );
        break;
      case 'setWriteProtected':
        image.writeProtected = operation.writeProtected;
        break;
    }
    snapshots.push(snapshot(image, input.probes));
  }
  return { snapshots };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'g64_trace'],
    {
      encoding: 'utf8',
      input: JSON.stringify(input),
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(`Rust G64 adapter failed:\n${result.stderr || result.stdout}`);
  }
  return JSON.parse(result.stdout) as DifferentialOutput;
}

const input = buildInput();
const typescript = runTypeScript(input);
const rust = runRust(input);
if (!isDeepStrictEqual(typescript, rust)) {
  throw new Error(
    `Rust/TypeScript G64 mismatch.\n` +
      `TypeScript=${JSON.stringify(typescript)}\nRust=${JSON.stringify(rust)}`,
  );
}

console.log(
  `PASS Rust/TypeScript G64 differential: ${input.operations.length} mutations, ` +
    `${input.probes.length} half-track probes, ${rust.snapshots.length} canonical images.`,
);

function writeUint16(bytes: Uint8Array, offset: number, value: number): void {
  bytes[offset] = value & 0xff;
  bytes[offset + 1] = (value >>> 8) & 0xff;
}

function writeUint32(bytes: Uint8Array, offset: number, value: number): void {
  bytes[offset] = value & 0xff;
  bytes[offset + 1] = (value >>> 8) & 0xff;
  bytes[offset + 2] = (value >>> 16) & 0xff;
  bytes[offset + 3] = (value >>> 24) & 0xff;
}
