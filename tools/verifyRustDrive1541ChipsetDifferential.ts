// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust 1541 chipset integration 差分
//
//   文件:       verifyRustDrive1541ChipsetDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import { C64Memory } from '../src/core/memory/C64Memory';
import { C64_MEMORY_LAYOUT } from '../src/core/memory/memoryLayout';
import { D64_LAYOUT, d64SectorCountThroughTrack } from '../src/media/D64DiskImage';
import { Commodore1541Drive } from '../src/peripherals/drive1541/Commodore1541Drive';
import type { Drive1541SpeedZone } from '../src/peripherals/drive1541/Drive1541Mechanism';
import { DRIVE_1541_MEMORY_LAYOUT } from '../src/peripherals/drive1541/Drive1541Memory';
import { IEC_LINE, IecBus } from '../src/peripherals/iec/IecBus';

const MECHANISM_DISK_PRESENT = 1 << 0;
const MECHANISM_LED_ON = 1 << 1;
const MECHANISM_MOTOR_ON = 1 << 2;
const MECHANISM_READING = 1 << 3;

type Operation =
  | { readonly cycles: number; readonly kind: 'clockSystemCycles' }
  | { readonly kind: 'mountD64' }
  | { readonly kind: 'resetBoard' }
  | {
      readonly kind: 'setMechanism';
      readonly ledOn: boolean;
      readonly motorOn: boolean;
      readonly speedZone: Drive1541SpeedZone;
      readonly stepperPhase: number;
    }
  | { readonly address: number; readonly kind: 'writeCia2'; readonly value: number };

interface DifferentialInput {
  readonly deviceNumber: number;
  readonly diskBytes: readonly number[];
  readonly operations: readonly Operation[];
  readonly rom: readonly number[];
}

interface DifferentialOutput {
  readonly snapshots: readonly ChipsetSnapshot[];
}

interface ChipsetSnapshot {
  readonly cia2PortA: number;
  readonly clock: {
    readonly leadCycles: number;
    readonly phaseRemainder: number;
    readonly targetCycles: number;
  };
  readonly cpu: {
    readonly accumulator: number;
    readonly atInstructionBoundary: boolean;
    readonly indexX: number;
    readonly indexY: number;
    readonly jammed: boolean;
    readonly programCounter: number;
    readonly stackPointer: number;
    readonly status: number;
  };
  readonly deviceNumber: number;
  readonly elapsedCycles: number;
  readonly iecBusLowMask: number;
  readonly lastDataBusValue: number;
  readonly lastResult: number | null;
  readonly mechanism: {
    readonly angularBitOffset: number;
    readonly controlMask: number;
    readonly halfTrack: number;
    readonly speedZone: number;
  };
  readonly ram: readonly number[];
  readonly resetAssertionSequence: number;
}

function buildInput(): DifferentialInput {
  const rom = new Uint8Array(DRIVE_1541_MEMORY_LAYOUT.rom.imageSize).fill(0xea);
  rom.set([
    0xad,
    0x00,
    0x18, // LDA $1800
    0x85,
    0x20, // STA $20
    0xa9,
    0x0a, // LDA #IEC DATA|CLOCK outputs
    0x8d,
    0x02,
    0x18, // STA $1802
    0x8d,
    0x00,
    0x18, // STA $1800
    0x4c,
    0x0d,
    0xc0, // JMP $c00d
  ]);
  rom[0x3ffc] = 0x00;
  rom[0x3ffd] = 0xc0;
  rom[0x3ffe] = 0x00;
  rom[0x3fff] = 0xc0;
  const diskBytes = new Uint8Array(d64SectorCountThroughTrack(35) * D64_LAYOUT.sectorSize);
  return {
    deviceNumber: 8,
    diskBytes: [...diskBytes],
    operations: [
      { cycles: 3, kind: 'clockSystemCycles' },
      { address: 0xdd02, kind: 'writeCia2', value: 1 << 5 },
      { address: 0xdd00, kind: 'writeCia2', value: 1 << 5 },
      { cycles: 1, kind: 'clockSystemCycles' },
      { address: 0xdd00, kind: 'writeCia2', value: 0 },
      { cycles: 6, kind: 'clockSystemCycles' },
      { kind: 'mountD64' },
      {
        kind: 'setMechanism',
        ledOn: false,
        motorOn: true,
        speedZone: 3,
        stepperPhase: 3,
      },
      { cycles: 20, kind: 'clockSystemCycles' },
      { kind: 'resetBoard' },
      { cycles: 4, kind: 'clockSystemCycles' },
      { address: 0xdd02, kind: 'writeCia2', value: (1 << 4) | (1 << 5) },
      { address: 0xdd00, kind: 'writeCia2', value: (1 << 4) | (1 << 5) },
      { cycles: 12, kind: 'clockSystemCycles' },
      { kind: 'resetBoard' },
      { cycles: 1, kind: 'clockSystemCycles' },
    ],
    rom: [...rom],
  };
}

function iecBusLowMask(iecBus: IecBus): number {
  const state = iecBus.state;
  return (
    (state.attentionHigh ? 0 : 1 << 0) |
    (state.clockHigh ? 0 : 1 << 1) |
    (state.dataHigh ? 0 : 1 << 2) |
    (state.resetHigh ? 0 : 1 << 3) |
    (state.serviceRequestHigh ? 0 : 1 << 4)
  );
}

function snapshot(
  memory: C64Memory,
  drive: Commodore1541Drive,
  lastResult: number | null,
  resetAssertionSequence: number,
): ChipsetSnapshot {
  const registers = drive.cpu.getRegisters();
  const mechanism = drive.mechanism;
  return {
    cia2PortA: memory.read(0xdd00),
    clock: {
      leadCycles: drive.clock.leadCycles,
      phaseRemainder: drive.clock.phaseRemainder,
      targetCycles: drive.clock.targetCycles,
    },
    cpu: {
      accumulator: registers.accumulator,
      atInstructionBoundary: drive.cpu.isAtInstructionBoundary,
      indexX: registers.indexX,
      indexY: registers.indexY,
      jammed: drive.cpu.isJammed,
      programCounter: registers.programCounter,
      stackPointer: registers.stackPointer,
      status: registers.status,
    },
    deviceNumber: drive.deviceNumber,
    elapsedCycles: drive.machine.elapsedCycles,
    iecBusLowMask: iecBusLowMask(memory.iecBus),
    lastDataBusValue: drive.memory.lastDataBusValue,
    lastResult,
    mechanism: {
      angularBitOffset: mechanism.angularBitOffset,
      controlMask:
        (mechanism.mountedDisk === undefined ? 0 : MECHANISM_DISK_PRESENT) |
        (mechanism.ledOn ? MECHANISM_LED_ON : 0) |
        (mechanism.motorOn ? MECHANISM_MOTOR_ON : 0) |
        (mechanism.reading ? MECHANISM_READING : 0),
      halfTrack: mechanism.currentHalfTrack,
      speedZone: mechanism.selectedSpeedZone,
    },
    ram: [...drive.memory.ram],
    resetAssertionSequence,
  };
}

function runTypeScript(input: DifferentialInput): DifferentialOutput {
  const iecBus = new IecBus();
  let resetAssertionSequence = 0;
  const stopObservingReset = iecBus.observe((transition) => {
    if (transition.changedLines.includes(IEC_LINE.reset) && !transition.state.resetHigh) {
      resetAssertionSequence += 1;
    }
  });
  const memory = new C64Memory(
    {
      basic: new Uint8Array(C64_MEMORY_LAYOUT.basicRom.size),
      character: new Uint8Array(C64_MEMORY_LAYOUT.characterRom.size),
      kernal: new Uint8Array(C64_MEMORY_LAYOUT.kernalRom.size),
    },
    { iecBus },
  );
  const drive = new Commodore1541Drive({
    deviceNumber: input.deviceNumber,
    iecBus,
    rom: Uint8Array.from(input.rom),
  });
  const snapshots: ChipsetSnapshot[] = [snapshot(memory, drive, null, resetAssertionSequence)];

  for (const operation of input.operations) {
    const elapsedBefore = drive.machine.elapsedCycles;
    let lastResult: number | null = null;
    switch (operation.kind) {
      case 'clockSystemCycles':
        for (let cycle = 0; cycle < operation.cycles; cycle += 1) {
          drive.clock.advanceHostCycle();
        }
        lastResult = drive.machine.elapsedCycles - elapsedBefore;
        break;
      case 'mountD64':
        drive.mountD64(Uint8Array.from(input.diskBytes));
        break;
      case 'resetBoard':
        memory.resetHardware();
        break;
      case 'setMechanism':
        drive.mechanism.applyControlState(operation);
        break;
      case 'writeCia2':
        memory.write(operation.address, operation.value);
        break;
    }
    snapshots.push(snapshot(memory, drive, lastResult, resetAssertionSequence));
  }

  drive.dispose();
  memory.dispose();
  stopObservingReset();
  return { snapshots };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'drive1541_chipset_trace'],
    {
      encoding: 'utf8',
      input: JSON.stringify(input),
      maxBuffer: 128 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(`Rust 1541 chipset adapter failed:\n${result.stderr || result.stdout}`);
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
    `Rust/TypeScript 1541 chipset mismatch after operation ${mismatch - 1}.\n` +
      `Operation=${JSON.stringify(input.operations[mismatch - 1])}\n` +
      `TypeScript=${JSON.stringify(typescript.snapshots[mismatch])}\n` +
      `Rust=${JSON.stringify(rust.snapshots[mismatch])}`,
  );
}

console.log(
  `PASS Rust/TypeScript 1541 chipset differential: ${input.operations.length} operations, ` +
    `${rust.snapshots.length} exact whole-drive snapshots.`,
);
