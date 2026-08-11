// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust 1541 machine 差分
//
//   文件:       verifyRustDrive1541MachineDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import { Cpu6502 } from '../src/core/cpu/Cpu6502';
import { MOS_6522_INTERRUPT_BIT, MOS_6522_REGISTER } from '../src/devices/Mos6522Registers';
import { D64DiskImage, D64_LAYOUT, d64SectorCountThroughTrack } from '../src/media/D64DiskImage';
import { Drive1541DiskVia } from '../src/peripherals/drive1541/Drive1541DiskVia';
import { Drive1541IecVia } from '../src/peripherals/drive1541/Drive1541IecVia';
import { Drive1541Machine } from '../src/peripherals/drive1541/Drive1541Machine';
import {
  DRIVE_1541_MECHANISM,
  Drive1541Mechanism,
  type Drive1541SpeedZone,
} from '../src/peripherals/drive1541/Drive1541Mechanism';
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
const CONTROL_BYTE_READY_ASSERTED = 1 << 5;
const INTERRUPT_IEC_VIA = 1 << 0;
const INTERRUPT_DISK_VIA = 1 << 1;

type Operation =
  | { readonly cycles: number; readonly kind: 'advanceHardware' }
  | { readonly cycles: number; readonly kind: 'clockCycles' }
  | { readonly kind: 'executeInstruction' }
  | { readonly cycles: number; readonly kind: 'mechanismTick' }
  | { readonly kind: 'mountD64' }
  | { readonly address: number; readonly kind: 'readMemory' }
  | { readonly kind: 'resetCpu' }
  | { readonly kind: 'resetTiming' }
  | {
      readonly kind: 'setMechanism';
      readonly motorOn: boolean;
      readonly reading: boolean;
      readonly speedZone: Drive1541SpeedZone;
      readonly writeDataByte: number;
    }
  | { readonly address: number; readonly kind: 'writeMemory'; readonly value: number };

interface DifferentialInput {
  readonly diskBytes: readonly number[];
  readonly operations: readonly Operation[];
  readonly rom: readonly number[];
}

interface DifferentialOutput {
  readonly snapshots: readonly MachineSnapshot[];
}

interface MachineSnapshot {
  readonly byteReadyEdgeSequence: number;
  readonly byteReadyTransitionSequence: number;
  readonly cpu: {
    readonly atInstructionBoundary: boolean;
    readonly jammed: boolean;
    readonly registers: {
      readonly accumulator: number;
      readonly indexX: number;
      readonly indexY: number;
      readonly programCounter: number;
      readonly stackPointer: number;
      readonly status: number;
    };
  };
  readonly elapsedCycles: number;
  readonly iecBusLowMask: number;
  readonly interruptPendingMask: number;
  readonly lastDataBusValue: number;
  readonly lastResult: number | null;
  readonly mechanismAngularBitOffset: number;
  readonly mechanismControlMask: number;
  readonly mechanismHalfTrack: number;
  readonly mechanismSpeedZone: number;
  readonly mechanismWriteDataByte: number;
  readonly ram: readonly number[];
}

function buildRom(): Uint8Array {
  const rom = new Uint8Array(DRIVE_1541_MEMORY_LAYOUT.rom.imageSize).fill(0xea);
  rom.set([0xb8, 0xa9, 0x42, 0x85, 0x00, 0x58, 0xea, 0xea]);
  rom[0x3ffc] = 0x00;
  rom[0x3ffd] = 0xc0;
  rom[0x3ffe] = 0x00;
  rom[0x3fff] = 0xc1;
  return rom;
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
  return {
    diskBytes: [...d64Bytes()],
    operations: [
      { kind: 'mountD64' },
      { cycles: DRIVE_1541_MECHANISM.diskChange.insertionCycles, kind: 'mechanismTick' },
      {
        kind: 'setMechanism',
        motorOn: true,
        reading: false,
        speedZone: 0,
        writeDataByte: 0xff,
      },
      { cycles: 32, kind: 'advanceHardware' },
      { kind: 'executeInstruction' },
      { cycles: 30, kind: 'advanceHardware' },
      { kind: 'resetCpu' },
      { kind: 'resetTiming' },
      { kind: 'executeInstruction' },
      { kind: 'executeInstruction' },
      { kind: 'executeInstruction' },
      { address: 0x0000, kind: 'readMemory' },
      {
        address: 0x1c00 + MOS_6522_REGISTER.interruptEnable,
        kind: 'writeMemory',
        value: MOS_6522_INTERRUPT_BIT.any | MOS_6522_INTERRUPT_BIT.ca1,
      },
      { address: 0x1c00 + MOS_6522_REGISTER.portA, kind: 'readMemory' },
      { cycles: 18, kind: 'advanceHardware' },
      { kind: 'executeInstruction' },
      { kind: 'executeInstruction' },
      { cycles: 7, kind: 'clockCycles' },
      { address: 0x0800, kind: 'writeMemory', value: 0x3c },
      { address: 0x17ff, kind: 'readMemory' },
      { kind: 'resetCpu' },
      { kind: 'resetTiming' },
      { cycles: 1, kind: 'clockCycles' },
      { cycles: 1, kind: 'clockCycles' },
    ],
    rom: [...buildRom()],
  };
}

function snapshot(
  machine: Drive1541Machine,
  iecBus: IecBus,
  lastResult: number | null,
  byteReadyTransitionSequence: number,
  byteReadyEdgeSequence: number,
): MachineSnapshot {
  const mechanism = machine.mechanism;
  const memory = machine.memory;
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
    (mechanism.byteReadyAsserted ? CONTROL_BYTE_READY_ASSERTED : 0);
  const interruptPendingMask =
    (memory.iecVia.interruptPending ? INTERRUPT_IEC_VIA : 0) |
    (memory.diskVia.interruptPending ? INTERRUPT_DISK_VIA : 0);
  return {
    byteReadyEdgeSequence,
    byteReadyTransitionSequence,
    cpu: {
      atInstructionBoundary: machine.cpu.isAtInstructionBoundary,
      jammed: machine.cpu.isJammed,
      registers: machine.cpu.getRegisters(),
    },
    elapsedCycles: machine.elapsedCycles,
    iecBusLowMask,
    interruptPendingMask,
    lastDataBusValue: memory.lastDataBusValue,
    lastResult,
    mechanismAngularBitOffset: mechanism.angularBitOffset,
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
  const machine = new Drive1541Machine(new Cpu6502(memory), memory, mechanism);
  let byteReadyTransitionSequence = 0;
  let byteReadyEdgeSequence = 0;
  mechanism.observeByteReady(() => {
    byteReadyTransitionSequence += 1;
  });
  mechanism.observeByteReadyEdge(() => {
    byteReadyEdgeSequence += 1;
  });
  const snapshots: MachineSnapshot[] = [snapshot(machine, iecBus, null, 0, 0)];

  for (const operation of input.operations) {
    let lastResult: number | null = null;
    switch (operation.kind) {
      case 'advanceHardware':
        machine.advanceHardware(operation.cycles);
        lastResult = operation.cycles;
        break;
      case 'clockCycles':
        lastResult = machine.clockCycles(operation.cycles);
        break;
      case 'executeInstruction':
        lastResult = machine.executeInstruction();
        break;
      case 'mechanismTick':
        mechanism.tick(operation.cycles);
        break;
      case 'mountD64':
        mechanism.mountDisk(new D64DiskImage(Uint8Array.from(input.diskBytes)));
        break;
      case 'readMemory':
        lastResult = memory.read(operation.address);
        break;
      case 'resetCpu':
        lastResult = machine.resetCpu();
        break;
      case 'resetTiming':
        machine.resetTiming();
        break;
      case 'setMechanism':
        mechanism.setSpeedZone(operation.speedZone);
        mechanism.setWriteDataByte(operation.writeDataByte);
        mechanism.setReadMode(operation.reading);
        mechanism.setMotorOn(operation.motorOn);
        break;
      case 'writeMemory':
        memory.write(operation.address, operation.value);
        break;
    }
    snapshots.push(
      snapshot(machine, iecBus, lastResult, byteReadyTransitionSequence, byteReadyEdgeSequence),
    );
  }
  machine.disconnect();
  diskVia.disconnect();
  iecVia.disconnect();
  return { snapshots };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'drive1541_machine_trace'],
    {
      encoding: 'utf8',
      input: JSON.stringify(input),
      maxBuffer: 128 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(`Rust 1541 machine adapter failed:\n${result.stderr || result.stdout}`);
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
    `Rust/TypeScript 1541 machine mismatch after operation ${mismatch - 1}.\n` +
      `Operation=${JSON.stringify(input.operations[mismatch - 1])}\n` +
      `TypeScript=${JSON.stringify(typescript.snapshots[mismatch])}\n` +
      `Rust=${JSON.stringify(rust.snapshots[mismatch])}`,
  );
}

console.log(
  `PASS Rust/TypeScript 1541 machine differential: ${input.operations.length} operations, ` +
    `${rust.snapshots.length} exact scheduler snapshots.`,
);
