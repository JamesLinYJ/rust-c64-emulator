// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust 1541 磁盘机构差分
//
//   文件:       verifyRustDrive1541MechanismDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { runRustJsonTraceSync } from './runRustJsonTrace';
import { isDeepStrictEqual } from 'node:util';

import { D64DiskImage, D64_LAYOUT, d64SectorCountThroughTrack } from '../src/media/D64DiskImage';
import { G64DiskImage, G64_LAYOUT } from '../src/media/G64DiskImage';
import {
  DRIVE_1541_MECHANISM,
  Drive1541Mechanism,
  type Drive1541SpeedZone,
} from '../src/peripherals/drive1541/Drive1541Mechanism';

type MediaInput =
  | { readonly bytes: readonly number[]; readonly kind: 'd64'; readonly writeProtected: boolean }
  | { readonly bytes: readonly number[]; readonly kind: 'g64'; readonly writeProtected: boolean };

type Operation =
  | { readonly kind: 'acknowledgeByteReady' }
  | {
      readonly kind: 'applyControl';
      readonly ledOn: boolean;
      readonly motorOn: boolean;
      readonly speedZone: Drive1541SpeedZone;
      readonly stepperPhase: number;
    }
  | { readonly kind: 'commitD64' }
  | { readonly kind: 'eject' }
  | { readonly kind: 'mount'; readonly mediaIndex: number }
  | { readonly kind: 'resetElectronics' }
  | { readonly enabled: boolean; readonly kind: 'setByteReadyEnabled' }
  | { readonly kind: 'setLedOn'; readonly ledOn: boolean }
  | { readonly kind: 'setMotorOn'; readonly motorOn: boolean }
  | { readonly kind: 'setReadMode'; readonly reading: boolean }
  | { readonly kind: 'setSpeedZone'; readonly speedZone: Drive1541SpeedZone }
  | { readonly kind: 'setWriteDataByte'; readonly value: number }
  | { readonly cycles: number; readonly kind: 'tick' };

interface RawTrackProbe {
  readonly byteCount: number;
  readonly halfTrack: number;
}

interface DifferentialInput {
  readonly media: readonly MediaInput[];
  readonly operations: readonly Operation[];
  readonly rawTrackProbes: readonly RawTrackProbe[];
}

interface DifferentialOutput {
  readonly snapshots: readonly MechanismSnapshot[];
}

interface MechanismSnapshot {
  readonly angularBitOffset: number;
  readonly byteReady: {
    readonly byteReadyAsserted: boolean;
    readonly byteReadyEdgeSequence: number;
    readonly byteReadyEnabled: boolean;
    readonly byteReadyTransitionSequence: number;
    readonly lastByteReadyEdgeData: number;
  };
  readonly control: {
    readonly ledOn: boolean;
    readonly motorOn: boolean;
    readonly reading: boolean;
  };
  readonly currentHalfTrack: number;
  readonly dataByte: number;
  readonly media: {
    readonly dirtyHalfTracks: readonly number[];
    readonly diskPresent: boolean;
    readonly kind: 'd64' | 'g64' | null;
    readonly rawTracks: readonly {
      readonly bytes: readonly number[] | null;
      readonly halfTrack: number;
    }[];
    readonly writeProtectSensorActive: boolean;
    readonly writeProtected: boolean;
  };
  readonly selectedSpeedZone: number;
  readonly syncFound: boolean;
  readonly writeDataByte: number;
}

function d64Bytes(): Uint8Array {
  const sectorCount = d64SectorCountThroughTrack(35);
  const bytes = new Uint8Array(sectorCount * D64_LAYOUT.sectorSize);
  const directoryOffset = d64SectorCountThroughTrack(17) * D64_LAYOUT.sectorSize;
  bytes[directoryOffset + D64_LAYOUT.directoryHeader.diskId1Offset] = 0x4a;
  bytes[directoryOffset + D64_LAYOUT.directoryHeader.diskId2Offset] = 0x53;
  return bytes;
}

function g64Bytes(): Uint8Array {
  const count = G64_LAYOUT.maximumHalfTrackCount;
  const maximumTrackLength = 16;
  const bytes = new Uint8Array(G64_LAYOUT.headerSize + count * 8);
  bytes.set(Uint8Array.from(G64_LAYOUT.signature, (character) => character.charCodeAt(0)));
  bytes[G64_LAYOUT.versionOffset] = G64_LAYOUT.supportedVersion;
  bytes[G64_LAYOUT.trackCountOffset] = count;
  bytes[G64_LAYOUT.maximumTrackLengthOffset] = maximumTrackLength;
  const image = new G64DiskImage(bytes);
  image.setHalfTrack(36, new Uint8Array(maximumTrackLength).fill(0xff), {
    kind: 'constant',
    zone: 0,
  });
  return image.toBytes();
}

function buildInput(): DifferentialInput {
  return {
    media: [
      { bytes: [...d64Bytes()], kind: 'd64', writeProtected: false },
      { bytes: [...g64Bytes()], kind: 'g64', writeProtected: false },
    ],
    operations: [
      { kind: 'mount', mediaIndex: 0 },
      { cycles: DRIVE_1541_MECHANISM.diskChange.insertionCycles, kind: 'tick' },
      { kind: 'setSpeedZone', speedZone: 2 },
      { kind: 'setMotorOn', motorOn: true },
      { cycles: 34, kind: 'tick' },
      { cycles: 138, kind: 'tick' },
      { kind: 'acknowledgeByteReady' },
      {
        kind: 'applyControl',
        ledOn: false,
        motorOn: true,
        speedZone: 0,
        stepperPhase: 3,
      },
      {
        kind: 'applyControl',
        ledOn: true,
        motorOn: true,
        speedZone: 0,
        stepperPhase: 2,
      },
      { kind: 'eject' },
      { kind: 'mount', mediaIndex: 0 },
      { cycles: DRIVE_1541_MECHANISM.diskChange.insertionCycles, kind: 'tick' },
      { kind: 'setWriteDataByte', value: 0xa5 },
      { kind: 'setReadMode', reading: false },
      { cycles: 64, kind: 'tick' },
      { kind: 'commitD64' },
      { kind: 'setReadMode', reading: true },
      { kind: 'eject' },
      { kind: 'mount', mediaIndex: 0 },
      { cycles: DRIVE_1541_MECHANISM.diskChange.removalCycles, kind: 'tick' },
      {
        cycles:
          DRIVE_1541_MECHANISM.diskChange.replacementGapCycles -
          DRIVE_1541_MECHANISM.diskChange.removalCycles,
        kind: 'tick',
      },
      {
        cycles:
          DRIVE_1541_MECHANISM.diskChange.insertionCycles -
          DRIVE_1541_MECHANISM.diskChange.replacementGapCycles,
        kind: 'tick',
      },
      { kind: 'eject' },
      { cycles: DRIVE_1541_MECHANISM.diskChange.removalCycles, kind: 'tick' },
      { kind: 'mount', mediaIndex: 1 },
      { cycles: DRIVE_1541_MECHANISM.diskChange.insertionCycles, kind: 'tick' },
      { kind: 'setSpeedZone', speedZone: 2 },
      { kind: 'setWriteDataByte', value: 0x3c },
      { kind: 'setReadMode', reading: false },
      { kind: 'setByteReadyEnabled', enabled: false },
      { cycles: 28, kind: 'tick' },
      { kind: 'setByteReadyEnabled', enabled: true },
      { cycles: 28, kind: 'tick' },
      { kind: 'resetElectronics' },
      { kind: 'setReadMode', reading: false },
      { cycles: 56, kind: 'tick' },
    ],
    rawTrackProbes: [
      { byteCount: 16, halfTrack: 36 },
      { byteCount: 16, halfTrack: 37 },
    ],
  };
}

function parseMedia(input: MediaInput): D64DiskImage | G64DiskImage {
  return input.kind === 'd64'
    ? new D64DiskImage(Uint8Array.from(input.bytes), { writeProtected: input.writeProtected })
    : new G64DiskImage(Uint8Array.from(input.bytes), { writeProtected: input.writeProtected });
}

function runTypeScript(input: DifferentialInput): DifferentialOutput {
  const mechanism = new Drive1541Mechanism();
  let byteReadyTransitionSequence = 0;
  let byteReadyEdgeSequence = 0;
  let lastByteReadyEdgeData = 0;
  mechanism.observeByteReady(() => {
    byteReadyTransitionSequence += 1;
  });
  mechanism.observeByteReadyEdge(({ dataByte }) => {
    byteReadyEdgeSequence += 1;
    lastByteReadyEdgeData = dataByte;
  });

  const snapshot = (): MechanismSnapshot => {
    const mounted = mechanism.mountedDisk;
    return {
      angularBitOffset: mechanism.angularBitOffset,
      byteReady: {
        byteReadyAsserted: mechanism.byteReadyAsserted,
        byteReadyEdgeSequence,
        byteReadyEnabled: mechanism.byteReadyEnabled,
        byteReadyTransitionSequence,
        lastByteReadyEdgeData,
      },
      control: {
        ledOn: mechanism.ledOn,
        motorOn: mechanism.motorOn,
        reading: mechanism.reading,
      },
      currentHalfTrack: mechanism.currentHalfTrack,
      dataByte: mechanism.dataByte,
      media: {
        dirtyHalfTracks: mechanism.dirtyHalfTracks,
        diskPresent: mechanism.diskPresent,
        kind: mounted === undefined ? null : mounted instanceof D64DiskImage ? 'd64' : 'g64',
        rawTracks: input.rawTrackProbes.map((probe) => ({
          bytes: mechanism.diskPresent
            ? [...mechanism.readRawHalfTrack(probe.halfTrack).slice(0, probe.byteCount)]
            : null,
          halfTrack: probe.halfTrack,
        })),
        writeProtectSensorActive: mechanism.writeProtectSensorActive,
        writeProtected: mechanism.writeProtected,
      },
      selectedSpeedZone: mechanism.selectedSpeedZone,
      syncFound: mechanism.syncFound,
      writeDataByte: mechanism.writeDataByte,
    };
  };

  const snapshots = [snapshot()];
  for (const operation of input.operations) {
    switch (operation.kind) {
      case 'acknowledgeByteReady':
        mechanism.acknowledgeByteReady();
        break;
      case 'applyControl':
        mechanism.applyControlState(operation);
        break;
      case 'commitD64':
        mechanism.commitRawTrackWritesToD64Sectors();
        break;
      case 'eject':
        mechanism.ejectDisk();
        break;
      case 'mount': {
        const media = input.media[operation.mediaIndex];
        if (media === undefined) throw new Error('Invalid mechanism media index.');
        mechanism.mountDisk(parseMedia(media));
        break;
      }
      case 'resetElectronics':
        mechanism.resetElectronics();
        break;
      case 'setByteReadyEnabled':
        mechanism.setByteReadyEnabled(operation.enabled);
        break;
      case 'setLedOn':
        mechanism.setLedOn(operation.ledOn);
        break;
      case 'setMotorOn':
        mechanism.setMotorOn(operation.motorOn);
        break;
      case 'setReadMode':
        mechanism.setReadMode(operation.reading);
        break;
      case 'setSpeedZone':
        mechanism.setSpeedZone(operation.speedZone);
        break;
      case 'setWriteDataByte':
        mechanism.setWriteDataByte(operation.value);
        break;
      case 'tick':
        mechanism.tick(operation.cycles);
        break;
    }
    snapshots.push(snapshot());
  }
  return { snapshots };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  return runRustJsonTraceSync<DifferentialOutput>({
    example: 'drive1541_mechanism_trace',
    input,
    label: 'Rust 1541 mechanism adapter',
    maximumOutputBytes: 128 * 1024 * 1024,
  });
}

const input = buildInput();
const typescript = runTypeScript(input);
const rust = runRust(input);
if (!isDeepStrictEqual(typescript, rust)) {
  const mismatch = typescript.snapshots.findIndex(
    (snapshot, index) => !isDeepStrictEqual(snapshot, rust.snapshots[index]),
  );
  throw new Error(
    `Rust/TypeScript 1541 mechanism mismatch after operation ${mismatch - 1}.\n` +
      `Operation=${JSON.stringify(input.operations[mismatch - 1])}\n` +
      `TypeScript=${JSON.stringify(typescript.snapshots[mismatch])}\n` +
      `Rust=${JSON.stringify(rust.snapshots[mismatch])}`,
  );
}

console.log(
  `PASS Rust/TypeScript 1541 mechanism differential: ${input.operations.length} operations, ` +
    `${input.rawTrackProbes.length} raw-track probes, ${rust.snapshots.length} exact snapshots.`,
);
