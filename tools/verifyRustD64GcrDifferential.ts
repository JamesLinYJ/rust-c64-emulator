// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust D64 GCR 固定数据差分
//
//   文件:       verifyRustD64GcrDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import {
  D64DiskImage,
  D64_ERROR_CODE,
  D64_LAYOUT,
  d64SectorCountThroughTrack,
  type D64ErrorCode,
} from '../src/media/D64DiskImage';
import {
  buildD64GcrTrack,
  decodeCommodoreGcr,
  decodeD64GcrTrack,
  encodeCommodoreGcr,
  encodeD64SectorToGcr,
} from '../src/peripherals/drive1541/CommodoreGcr';

interface SectorCase {
  readonly data: readonly number[];
  readonly errorCode: D64ErrorCode;
  readonly id1: number;
  readonly id2: number;
  readonly sector: number;
  readonly track: number;
}

interface DifferentialInput {
  readonly diskBytes: readonly number[];
  readonly groups: readonly (readonly number[])[];
  readonly sectors: readonly SectorCase[];
  readonly tracks: readonly number[];
}

interface DifferentialOutput {
  readonly groups: readonly {
    readonly decoded: readonly number[];
    readonly encoded: readonly number[];
  }[];
  readonly sectors: readonly (readonly number[])[];
  readonly tracks: readonly {
    readonly bytes: readonly number[];
    readonly decodedSectors: readonly {
      readonly data: readonly number[];
      readonly headerBitOffset: number;
      readonly id1: number;
      readonly id2: number;
      readonly sector: number;
      readonly track: number;
    }[];
    readonly issues: readonly { readonly bitOffset: number; readonly reason: string }[];
    readonly speedZone: number;
    readonly track: number;
    readonly transferBitsPerSecond: number;
  }[];
}

const REPRESENTATIVE_TRACKS = [1, 17, 18, 24, 25, 30, 31, 35] as const;
const REPRESENTABLE_ERROR_CODES = [
  D64_ERROR_CODE.ok,
  D64_ERROR_CODE.headerNotFound,
  D64_ERROR_CODE.syncNotFound,
  D64_ERROR_CODE.dataBlockNotFound,
  D64_ERROR_CODE.dataChecksum,
  D64_ERROR_CODE.headerChecksum,
  D64_ERROR_CODE.diskIdMismatch,
] as const;

function patternedBytes(length: number, seed: number): Uint8Array {
  return Uint8Array.from(
    { length },
    (_unused, index) => (seed + index * 73 + Math.trunc(index / 13) * 19) & 0xff,
  );
}

function buildInput(): DifferentialInput {
  const sectorCount = d64SectorCountThroughTrack(D64_LAYOUT.minimumTrackCount);
  const diskBytes = patternedBytes(sectorCount * D64_LAYOUT.sectorSize, 0x41);
  const directoryOffset = d64SectorCountThroughTrack(17) * D64_LAYOUT.sectorSize;
  diskBytes[directoryOffset + D64_LAYOUT.directoryHeader.diskId1Offset] = 0x4a;
  diskBytes[directoryOffset + D64_LAYOUT.directoryHeader.diskId2Offset] = 0x53;

  return {
    diskBytes: [...diskBytes],
    groups: [
      [...Uint8Array.of(0x00, 0x55, 0xaa, 0xff)],
      [...Uint8Array.from({ length: 256 }, (_unused, index) => index)],
      [...patternedBytes(1024, 0x97)],
    ],
    sectors: REPRESENTABLE_ERROR_CODES.map((errorCode, index) => ({
      data: [...patternedBytes(D64_LAYOUT.sectorSize, 0x20 + index * 17)],
      errorCode,
      id1: 0x4a,
      id2: 0x53,
      sector: index,
      track: REPRESENTATIVE_TRACKS[index],
    })),
    tracks: REPRESENTATIVE_TRACKS,
  };
}

function runTypeScript(input: DifferentialInput): DifferentialOutput {
  const image = new D64DiskImage(Uint8Array.from(input.diskBytes));
  return {
    groups: input.groups.map((source) => {
      const encoded = encodeCommodoreGcr(Uint8Array.from(source));
      return { decoded: [...decodeCommodoreGcr(encoded)], encoded: [...encoded] };
    }),
    sectors: input.sectors.map((sectorCase) => [
      ...encodeD64SectorToGcr(Uint8Array.from(sectorCase.data), sectorCase, sectorCase.errorCode),
    ]),
    tracks: input.tracks.map((trackNumber) => {
      const track = buildD64GcrTrack(image, trackNumber);
      const decoded = decodeD64GcrTrack(track.bytes);
      return {
        bytes: [...track.bytes],
        decodedSectors: decoded.sectors.map((sector) => ({
          ...sector,
          data: [...sector.data],
        })),
        issues: decoded.issues,
        speedZone: track.speedZone,
        track: track.track,
        transferBitsPerSecond: track.transferBitsPerSecond,
      };
    }),
  };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'd64_gcr_trace'],
    {
      encoding: 'utf8',
      input: JSON.stringify(input),
      maxBuffer: 128 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(`Rust D64 GCR adapter failed:\n${result.stderr || result.stdout}`);
  }
  return JSON.parse(result.stdout) as DifferentialOutput;
}

const input = buildInput();
const typescript = runTypeScript(input);
const rust = runRust(input);
if (!isDeepStrictEqual(typescript, rust)) {
  throw new Error(
    `Rust/TypeScript D64 GCR fixed-data mismatch.\n` +
      `TypeScript=${JSON.stringify(typescript)}\nRust=${JSON.stringify(rust)}`,
  );
}

console.log(
  `PASS Rust/TypeScript D64 GCR differential: ${input.groups.length} grouped codecs, ` +
    `${input.sectors.length} passive error encodings, ${input.tracks.length} complete raw tracks.`,
);
