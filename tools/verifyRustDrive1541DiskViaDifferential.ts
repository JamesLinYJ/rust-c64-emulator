// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust 1541 VIA2 差分
//
//   文件:       verifyRustDrive1541DiskViaDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { runRustJsonTraceSync } from './runRustJsonTrace';
import { isDeepStrictEqual } from 'node:util';

import { MOS_6522_PCR_CONTROL_MODE, MOS_6522_REGISTER } from '../src/devices/Mos6522Registers';
import { D64DiskImage, D64_LAYOUT, d64SectorCountThroughTrack } from '../src/media/D64DiskImage';
import {
  DRIVE_1541_DISK_PORT_B_BIT,
  Drive1541DiskVia,
} from '../src/peripherals/drive1541/Drive1541DiskVia';
import {
  DRIVE_1541_MECHANISM,
  Drive1541Mechanism,
} from '../src/peripherals/drive1541/Drive1541Mechanism';

type Operation =
  | { readonly cycles: number; readonly kind: 'clock' }
  | { readonly cycles: number; readonly kind: 'mechanismTick' }
  | { readonly kind: 'mountD64'; readonly writeProtected: boolean }
  | { readonly address: number; readonly kind: 'read' }
  | { readonly kind: 'reset' }
  | { readonly address: number; readonly kind: 'write'; readonly value: number };

interface DifferentialInput {
  readonly diskBytes: readonly number[];
  readonly operations: readonly Operation[];
}

interface DifferentialOutput {
  readonly snapshots: readonly DiskViaSnapshot[];
}

interface DiskViaSnapshot {
  readonly deviceNumber: number;
  readonly lastRead: number | null;
  readonly mechanism: {
    readonly byteReady: {
      readonly asserted: boolean;
      readonly edgeSequence: number;
      readonly enabled: boolean;
      readonly transitionSequence: number;
    };
    readonly control: {
      readonly ledOn: boolean;
      readonly motorOn: boolean;
      readonly reading: boolean;
    };
    readonly gcr: {
      readonly dataByte: number;
      readonly syncFound: boolean;
      readonly writeDataByte: number;
    };
    readonly media: {
      readonly dirtyHalfTracks: readonly number[];
      readonly diskPresent: boolean;
      readonly writeProtectSensorActive: boolean;
    };
    readonly position: {
      readonly angularBitOffset: number;
      readonly currentHalfTrack: number;
      readonly selectedSpeedZone: number;
    };
  };
  readonly via: {
    readonly interruptPending: boolean;
    readonly portAOutputPins: number;
    readonly portBDataDirection: number;
    readonly portBOutputLatch: number;
    readonly portBOutputPins: number;
  };
}

function d64Bytes(): Uint8Array {
  const sectorCount = d64SectorCountThroughTrack(35);
  const bytes = new Uint8Array(sectorCount * D64_LAYOUT.sectorSize);
  const directoryOffset = d64SectorCountThroughTrack(17) * D64_LAYOUT.sectorSize;
  bytes[directoryOffset + D64_LAYOUT.directoryHeader.diskId1Offset] = 0x4a;
  bytes[directoryOffset + D64_LAYOUT.directoryHeader.diskId2Offset] = 0x53;
  return bytes;
}

function buildInput(): DifferentialInput {
  const write = (address: number, value: number): Operation => ({
    address,
    kind: 'write',
    value,
  });
  return {
    diskBytes: [...d64Bytes()],
    operations: [
      { address: MOS_6522_REGISTER.portB, kind: 'read' },
      { kind: 'mountD64', writeProtected: false },
      { address: MOS_6522_REGISTER.portB, kind: 'read' },
      { cycles: DRIVE_1541_MECHANISM.diskChange.insertionCycles, kind: 'mechanismTick' },
      { address: MOS_6522_REGISTER.portB, kind: 'read' },
      write(MOS_6522_REGISTER.interruptEnable, 0x82),
      write(MOS_6522_REGISTER.dataDirectionB, 0x6f),
      write(
        MOS_6522_REGISTER.portB,
        3 | DRIVE_1541_DISK_PORT_B_BIT.motor | DRIVE_1541_DISK_PORT_B_BIT.led | (2 << 5),
      ),
      write(
        MOS_6522_REGISTER.portB,
        2 | DRIVE_1541_DISK_PORT_B_BIT.motor | DRIVE_1541_DISK_PORT_B_BIT.led | (2 << 5),
      ),
      { cycles: 34, kind: 'clock' },
      { address: MOS_6522_REGISTER.portB, kind: 'read' },
      { cycles: 138, kind: 'clock' },
      { address: MOS_6522_REGISTER.interruptFlags, kind: 'read' },
      { address: MOS_6522_REGISTER.portA, kind: 'read' },
      { address: MOS_6522_REGISTER.interruptFlags, kind: 'read' },
      write(MOS_6522_REGISTER.dataDirectionA, 0xf0),
      write(MOS_6522_REGISTER.portA, 0xa0),
      write(
        MOS_6522_REGISTER.peripheralControl,
        (MOS_6522_PCR_CONTROL_MODE.lowOutput << 1) | (MOS_6522_PCR_CONTROL_MODE.highOutput << 5),
      ),
      write(
        MOS_6522_REGISTER.peripheralControl,
        (MOS_6522_PCR_CONTROL_MODE.highOutput << 1) | (MOS_6522_PCR_CONTROL_MODE.lowOutput << 5),
      ),
      { cycles: 56, kind: 'clock' },
      { address: MOS_6522_REGISTER.portB, kind: 'read' },
      { cycles: 28, kind: 'clock' },
      write(MOS_6522_REGISTER.portAWithoutHandshake, 0x30),
      write(
        MOS_6522_REGISTER.peripheralControl,
        (MOS_6522_PCR_CONTROL_MODE.pulseOutput << 1) | (MOS_6522_PCR_CONTROL_MODE.pulseOutput << 5),
      ),
      { address: MOS_6522_REGISTER.portA, kind: 'read' },
      { address: MOS_6522_REGISTER.portB, kind: 'read' },
      { cycles: 1, kind: 'clock' },
      write(MOS_6522_REGISTER.timer1LatchLow, 0x02),
      write(MOS_6522_REGISTER.timer1CounterHigh, 0x00),
      write(MOS_6522_REGISTER.auxiliaryControl, 0xc0),
      { cycles: 8, kind: 'clock' },
      { kind: 'reset' },
      { cycles: 1, kind: 'clock' },
    ],
  };
}

function snapshot(
  via: Drive1541DiskVia,
  mechanism: Drive1541Mechanism,
  lastRead: number | null,
  byteReadyTransitionSequence: number,
  byteReadyEdgeSequence: number,
): DiskViaSnapshot {
  return {
    deviceNumber: via.deviceNumber,
    lastRead,
    mechanism: {
      byteReady: {
        asserted: mechanism.byteReadyAsserted,
        edgeSequence: byteReadyEdgeSequence,
        enabled: mechanism.byteReadyEnabled,
        transitionSequence: byteReadyTransitionSequence,
      },
      control: {
        ledOn: mechanism.ledOn,
        motorOn: mechanism.motorOn,
        reading: mechanism.reading,
      },
      gcr: {
        dataByte: mechanism.dataByte,
        syncFound: mechanism.syncFound,
        writeDataByte: mechanism.writeDataByte,
      },
      media: {
        dirtyHalfTracks: mechanism.dirtyHalfTracks,
        diskPresent: mechanism.diskPresent,
        writeProtectSensorActive: mechanism.writeProtectSensorActive,
      },
      position: {
        angularBitOffset: mechanism.angularBitOffset,
        currentHalfTrack: mechanism.currentHalfTrack,
        selectedSpeedZone: mechanism.selectedSpeedZone,
      },
    },
    via: {
      interruptPending: via.interruptPending,
      portAOutputPins: via.portAOutputPins,
      portBDataDirection: via.portBDataDirection,
      portBOutputLatch: via.portBOutputLatch,
      portBOutputPins: via.portBOutputPins,
    },
  };
}

function runTypeScript(input: DifferentialInput): DifferentialOutput {
  const mechanism = new Drive1541Mechanism();
  const via = new Drive1541DiskVia({ deviceNumber: 8, mechanism });
  let byteReadyTransitionSequence = 0;
  let byteReadyEdgeSequence = 0;
  mechanism.observeByteReady(() => {
    byteReadyTransitionSequence += 1;
  });
  mechanism.observeByteReadyEdge(() => {
    byteReadyEdgeSequence += 1;
  });
  const snapshots: DiskViaSnapshot[] = [snapshot(via, mechanism, null, 0, 0)];

  for (const operation of input.operations) {
    let lastRead: number | null = null;
    switch (operation.kind) {
      case 'clock':
        for (let cycle = 0; cycle < operation.cycles; cycle += 1) {
          mechanism.tick(1);
          via.tick(1);
        }
        break;
      case 'mechanismTick':
        mechanism.tick(operation.cycles);
        break;
      case 'mountD64':
        mechanism.mountDisk(
          new D64DiskImage(Uint8Array.from(input.diskBytes), {
            writeProtected: operation.writeProtected,
          }),
        );
        break;
      case 'read':
        lastRead = via.read(operation.address);
        break;
      case 'reset':
        via.reset();
        break;
      case 'write':
        via.write(operation.address, operation.value);
        break;
    }
    snapshots.push(
      snapshot(via, mechanism, lastRead, byteReadyTransitionSequence, byteReadyEdgeSequence),
    );
  }
  via.disconnect();
  return { snapshots };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  return runRustJsonTraceSync<DifferentialOutput>({
    example: 'drive1541_disk_via_trace',
    input,
    label: 'Rust 1541 VIA2 adapter',
    maximumOutputBytes: 128 * 1024 * 1024,
  });
}

const input = buildInput();
const typescript = runTypeScript(input);
const rust = runRust(input);
if (!isDeepStrictEqual(typescript, rust)) {
  const mismatch = typescript.snapshots.findIndex(
    (candidate, index) => !isDeepStrictEqual(candidate, rust.snapshots[index]),
  );
  throw new Error(
    `Rust/TypeScript 1541 VIA2 mismatch after operation ${mismatch - 1}.\n` +
      `Operation=${JSON.stringify(input.operations[mismatch - 1])}\n` +
      `TypeScript=${JSON.stringify(typescript.snapshots[mismatch])}\n` +
      `Rust=${JSON.stringify(rust.snapshots[mismatch])}`,
  );
}

console.log(
  `PASS Rust/TypeScript 1541 VIA2 differential: ${input.operations.length} operations, ` +
    `${rust.snapshots.length} exact board snapshots.`,
);
