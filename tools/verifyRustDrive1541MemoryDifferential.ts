// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust 1541 内存地址译码差分
//
//   文件:       verifyRustDrive1541MemoryDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import { MOS_6522_REGISTER } from '../src/devices/Mos6522Registers';
import { Drive1541DiskVia } from '../src/peripherals/drive1541/Drive1541DiskVia';
import { Drive1541IecVia } from '../src/peripherals/drive1541/Drive1541IecVia';
import { Drive1541Mechanism } from '../src/peripherals/drive1541/Drive1541Mechanism';
import {
  DRIVE_1541_MEMORY_LAYOUT,
  Drive1541Memory,
} from '../src/peripherals/drive1541/Drive1541Memory';
import { IecBus } from '../src/peripherals/iec/IecBus';

const CONTROL_MOTOR_ON = 1 << 0;
const CONTROL_LED_ON = 1 << 1;
const CONTROL_READING = 1 << 2;
const CONTROL_BYTE_READY_ENABLED = 1 << 3;
const CONTROL_SYNC_FOUND = 1 << 4;
const CONTROL_WRITE_PROTECT_SENSOR = 1 << 5;
const INTERRUPT_IEC_VIA = 1 << 0;
const INTERRUPT_DISK_VIA = 1 << 1;

type Operation =
  | { readonly address: number; readonly kind: 'read' }
  | { readonly kind: 'readStack'; readonly stackPointer: number }
  | { readonly address: number; readonly kind: 'readWord' }
  | { readonly kind: 'reset' }
  | { readonly address: number; readonly kind: 'write'; readonly value: number }
  | { readonly kind: 'writeStack'; readonly stackPointer: number; readonly value: number }
  | { readonly address: number; readonly kind: 'writeWord'; readonly value: number };

interface DifferentialInput {
  readonly operations: readonly Operation[];
  readonly rom: readonly number[];
}

interface DifferentialOutput {
  readonly snapshots: readonly MemorySnapshot[];
}

interface MemorySnapshot {
  readonly diskPortAOutputPins: number;
  readonly diskPortBDataDirection: number;
  readonly diskPortBOutputLatch: number;
  readonly iecBusLowMask: number;
  readonly iecPortBDataDirection: number;
  readonly iecPortBOutputLatch: number;
  readonly interruptPendingMask: number;
  readonly lastDataBusValue: number;
  readonly lastRead: number | null;
  readonly mechanismControlMask: number;
  readonly mechanismHalfTrack: number;
  readonly mechanismSpeedZone: number;
  readonly mechanismWriteDataByte: number;
  readonly ram: readonly number[];
}

function buildInput(): DifferentialInput {
  const write = (address: number, value: number): Operation => ({
    address,
    kind: 'write',
    value,
  });
  return {
    operations: [
      { address: 0x8000, kind: 'read' },
      { address: 0xbfff, kind: 'read' },
      { address: 0xc000, kind: 'read' },
      { address: 0xffff, kind: 'read' },
      write(0xc000, 0xff),
      { address: 0xc000, kind: 'read' },
      write(0x0123, 0xa5),
      { address: 0x2123, kind: 'read' },
      { address: 0x4123, kind: 'read' },
      { address: 0x6123, kind: 'read' },
      write(0x0923, 0x5a),
      { address: 0x0923, kind: 'read' },
      { address: 0x0123, kind: 'read' },
      write(0x1800 + MOS_6522_REGISTER.dataDirectionB, 0x5a),
      write(0x1800 + MOS_6522_REGISTER.portB, 0x0a),
      write(0x1c00 + MOS_6522_REGISTER.dataDirectionA, 0xa5),
      { address: 0x3800 + MOS_6522_REGISTER.dataDirectionB, kind: 'read' },
      { address: 0x5c00 + MOS_6522_REGISTER.dataDirectionA, kind: 'read' },
      write(0x0800, 0x3c),
      { address: 0x17ff, kind: 'read' },
      { address: 0x07ff, kind: 'writeWord', value: 0xa55a },
      { kind: 'writeStack', stackPointer: 0xfe, value: 0x81 },
      { kind: 'readStack', stackPointer: 0xfe },
      { address: 0x01fe, kind: 'readWord' },
      write(0x1c00 + MOS_6522_REGISTER.dataDirectionB, 0x6f),
      write(0x1c00 + MOS_6522_REGISTER.portB, 3 | (1 << 2) | (1 << 3) | (2 << 5)),
      { kind: 'reset' },
      { address: 0x0800, kind: 'read' },
      { address: 0x0000, kind: 'read' },
    ],
    rom: Array.from(
      { length: DRIVE_1541_MEMORY_LAYOUT.rom.imageSize },
      (_unused, index) => index & 0xff,
    ),
  };
}

function snapshot(
  memory: Drive1541Memory,
  mechanism: Drive1541Mechanism,
  iecBus: IecBus,
  lastRead: number | null,
): MemorySnapshot {
  const state = iecBus.state;
  const iecBusLowMask =
    (state.attentionHigh ? 0 : 1 << 0) |
    (state.clockHigh ? 0 : 1 << 1) |
    (state.dataHigh ? 0 : 1 << 2) |
    (state.resetHigh ? 0 : 1 << 3) |
    (state.serviceRequestHigh ? 0 : 1 << 4);
  const mechanismControlMask =
    (mechanism.motorOn ? CONTROL_MOTOR_ON : 0) |
    (mechanism.ledOn ? CONTROL_LED_ON : 0) |
    (mechanism.reading ? CONTROL_READING : 0) |
    (mechanism.byteReadyEnabled ? CONTROL_BYTE_READY_ENABLED : 0) |
    (mechanism.syncFound ? CONTROL_SYNC_FOUND : 0) |
    (mechanism.writeProtectSensorActive ? CONTROL_WRITE_PROTECT_SENSOR : 0);
  const interruptPendingMask =
    (memory.iecVia.interruptPending ? INTERRUPT_IEC_VIA : 0) |
    (memory.diskVia.interruptPending ? INTERRUPT_DISK_VIA : 0);
  return {
    diskPortAOutputPins: memory.diskVia.portAOutputPins,
    diskPortBDataDirection: memory.diskVia.portBDataDirection,
    diskPortBOutputLatch: memory.diskVia.portBOutputLatch,
    iecBusLowMask,
    iecPortBDataDirection: memory.iecVia.portBDataDirection,
    iecPortBOutputLatch: memory.iecVia.portBOutputLatch,
    interruptPendingMask,
    lastDataBusValue: memory.lastDataBusValue,
    lastRead,
    mechanismControlMask,
    mechanismHalfTrack: mechanism.currentHalfTrack,
    mechanismSpeedZone: mechanism.selectedSpeedZone,
    mechanismWriteDataByte: mechanism.writeDataByte,
    ram: [...memory.ram],
  };
}

function runTypeScript(input: DifferentialInput): DifferentialOutput {
  const iecBus = new IecBus();
  const mechanism = new Drive1541Mechanism();
  const iecVia = new Drive1541IecVia({ deviceNumber: 8, iecBus });
  const diskVia = new Drive1541DiskVia({ deviceNumber: 8, mechanism });
  const memory = new Drive1541Memory(Uint8Array.from(input.rom), { diskVia, iecVia });
  const snapshots: MemorySnapshot[] = [snapshot(memory, mechanism, iecBus, null)];

  for (const operation of input.operations) {
    let lastRead: number | null = null;
    switch (operation.kind) {
      case 'read':
        lastRead = memory.read(operation.address);
        break;
      case 'readStack':
        lastRead = memory.readStack(operation.stackPointer);
        break;
      case 'readWord':
        lastRead = memory.readWord(operation.address);
        break;
      case 'reset':
        memory.resetHardware();
        break;
      case 'write':
        memory.write(operation.address, operation.value);
        break;
      case 'writeStack':
        memory.writeStack(operation.stackPointer, operation.value);
        break;
      case 'writeWord':
        memory.writeWord(operation.address, operation.value);
        break;
    }
    snapshots.push(snapshot(memory, mechanism, iecBus, lastRead));
  }
  diskVia.disconnect();
  iecVia.disconnect();
  return { snapshots };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'drive1541_memory_trace'],
    {
      encoding: 'utf8',
      input: JSON.stringify(input),
      maxBuffer: 128 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(`Rust 1541 memory adapter failed:\n${result.stderr || result.stdout}`);
  }
  return JSON.parse(result.stdout) as DifferentialOutput;
}

const input = buildInput();
const typescript = runTypeScript(input);
const rust = runRust(input);
if (!isDeepStrictEqual(typescript, rust)) {
  const mismatch = typescript.snapshots.findIndex(
    (candidate, index) => !isDeepStrictEqual(candidate, rust.snapshots[index]),
  );
  throw new Error(
    `Rust/TypeScript 1541 memory mismatch after operation ${mismatch - 1}.\n` +
      `Operation=${JSON.stringify(input.operations[mismatch - 1])}\n` +
      `TypeScript=${JSON.stringify(typescript.snapshots[mismatch])}\n` +
      `Rust=${JSON.stringify(rust.snapshots[mismatch])}`,
  );
}

console.log(
  `PASS Rust/TypeScript 1541 memory differential: ${input.operations.length} operations, ` +
    `${rust.snapshots.length} exact decoder snapshots.`,
);
