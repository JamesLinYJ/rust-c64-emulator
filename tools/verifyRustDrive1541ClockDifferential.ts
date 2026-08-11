// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Rust 1541 integer clock 差分
//
//   文件:       verifyRustDrive1541ClockDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import { Cpu6502 } from '../src/core/cpu/Cpu6502';
import { Drive1541ClockSynchronizer } from '../src/peripherals/drive1541/Drive1541ClockSynchronizer';
import { Drive1541DiskVia } from '../src/peripherals/drive1541/Drive1541DiskVia';
import { Drive1541IecVia } from '../src/peripherals/drive1541/Drive1541IecVia';
import { Drive1541Machine } from '../src/peripherals/drive1541/Drive1541Machine';
import { Drive1541Mechanism } from '../src/peripherals/drive1541/Drive1541Mechanism';
import {
  DRIVE_1541_MEMORY_LAYOUT,
  Drive1541Memory,
} from '../src/peripherals/drive1541/Drive1541Memory';
import { IEC_LINE, IecBus, type IecLine } from '../src/peripherals/iec/IecBus';
import { PAL_VIDEO_STANDARD } from '../src/video/palVideoStandard';

const IEC_LINES_BY_MASK: readonly (readonly [number, IecLine])[] = [
  [1 << 0, IEC_LINE.attention],
  [1 << 1, IEC_LINE.clock],
  [1 << 2, IEC_LINE.data],
  [1 << 3, IEC_LINE.reset],
  [1 << 4, IEC_LINE.serviceRequest],
];

type Operation =
  | { readonly kind: 'advanceHostCycle' }
  | { readonly cycles: number; readonly kind: 'advanceHostCycles' }
  | { readonly kind: 'resetClock' }
  | { readonly kind: 'setIecLowMask'; readonly lowMask: number };

interface DifferentialInput {
  readonly hostClockHz: number;
  readonly operations: readonly Operation[];
  readonly rom: readonly number[];
}

interface DifferentialOutput {
  readonly snapshots: readonly ClockSnapshot[];
}

interface ClockSnapshot {
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
  readonly elapsedCycles: number;
  readonly hostClockHz: number;
  readonly iecBusLowMask: number;
  readonly lastDataBusValue: number;
  readonly lastResult: number | null;
  readonly leadCycles: number;
  readonly phaseRemainder: number;
  readonly ram: readonly number[];
  readonly targetCycles: number;
}

function buildInput(): DifferentialInput {
  const rom = new Uint8Array(DRIVE_1541_MEMORY_LAYOUT.rom.imageSize).fill(0xea);
  rom.set([0xad, 0x00, 0x18, 0x8d, 0x00, 0x00, 0xea]);
  rom[0x3ffc] = 0x00;
  rom[0x3ffd] = 0xc0;
  rom[0x3ffe] = 0x00;
  rom[0x3fff] = 0xc0;
  return {
    hostClockHz: PAL_VIDEO_STANDARD.timing.processorClockHz,
    operations: [
      { kind: 'advanceHostCycle' },
      { kind: 'advanceHostCycle' },
      { cycles: 0, kind: 'advanceHostCycles' },
      { cycles: 1, kind: 'advanceHostCycles' },
      { kind: 'setIecLowMask', lowMask: 1 << 2 },
      { kind: 'advanceHostCycle' },
      { kind: 'setIecLowMask', lowMask: 0 },
      { cycles: 61, kind: 'advanceHostCycles' },
      { cycles: 997, kind: 'advanceHostCycles' },
      { kind: 'resetClock' },
      {
        cycles: PAL_VIDEO_STANDARD.timing.processorClockHz,
        kind: 'advanceHostCycles',
      },
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
  clock: Drive1541ClockSynchronizer,
  machine: Drive1541Machine,
  iecBus: IecBus,
  hostClockHz: number,
  lastResult: number | null,
): ClockSnapshot {
  const registers = machine.cpu.getRegisters();
  return {
    cpu: {
      accumulator: registers.accumulator,
      atInstructionBoundary: machine.cpu.isAtInstructionBoundary,
      indexX: registers.indexX,
      indexY: registers.indexY,
      jammed: machine.cpu.isJammed,
      programCounter: registers.programCounter,
      stackPointer: registers.stackPointer,
      status: registers.status,
    },
    elapsedCycles: machine.elapsedCycles,
    hostClockHz,
    iecBusLowMask: iecBusLowMask(iecBus),
    lastDataBusValue: machine.memory.lastDataBusValue,
    lastResult,
    leadCycles: clock.leadCycles,
    phaseRemainder: clock.phaseRemainder,
    ram: [...machine.memory.ram],
    targetCycles: clock.targetCycles,
  };
}

function runTypeScript(input: DifferentialInput): DifferentialOutput {
  const iecBus = new IecBus();
  const hostPort = iecBus.attach('clock differential host');
  const mechanism = new Drive1541Mechanism();
  const iecVia = new Drive1541IecVia({ deviceNumber: 8, iecBus });
  const diskVia = new Drive1541DiskVia({ deviceNumber: 8, mechanism });
  const memory = new Drive1541Memory(Uint8Array.from(input.rom), { diskVia, iecVia });
  const machine = new Drive1541Machine(new Cpu6502(memory), memory, mechanism);
  const clock = new Drive1541ClockSynchronizer(machine, input.hostClockHz);
  const snapshots: ClockSnapshot[] = [snapshot(clock, machine, iecBus, input.hostClockHz, null)];

  for (const operation of input.operations) {
    const elapsedBefore = machine.elapsedCycles;
    switch (operation.kind) {
      case 'advanceHostCycle':
        clock.advanceHostCycle();
        break;
      case 'advanceHostCycles':
        clock.advanceHostCycles(operation.cycles);
        break;
      case 'resetClock':
        clock.resetClock();
        break;
      case 'setIecLowMask':
        hostPort.setPulledLowLines(
          IEC_LINES_BY_MASK.filter(([mask]) => (operation.lowMask & mask) !== 0).map(
            ([, line]) => line,
          ),
        );
        break;
    }
    const lastResult = operation.kind.startsWith('advanceHost')
      ? machine.elapsedCycles - elapsedBefore
      : null;
    snapshots.push(snapshot(clock, machine, iecBus, input.hostClockHz, lastResult));
  }

  machine.disconnect();
  diskVia.disconnect();
  iecVia.disconnect();
  hostPort.disconnect();
  return { snapshots };
}

function runRust(input: DifferentialInput): DifferentialOutput {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'drive1541_clock_trace'],
    {
      encoding: 'utf8',
      input: JSON.stringify(input),
      maxBuffer: 128 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(`Rust 1541 clock adapter failed:\n${result.stderr || result.stdout}`);
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
    `Rust/TypeScript 1541 clock mismatch after operation ${mismatch - 1}.\n` +
      `Operation=${JSON.stringify(input.operations[mismatch - 1])}\n` +
      `TypeScript=${JSON.stringify(typescript.snapshots[mismatch])}\n` +
      `Rust=${JSON.stringify(rust.snapshots[mismatch])}`,
  );
}

console.log(
  `PASS Rust/TypeScript 1541 integer clock differential: ${input.operations.length} operations, ` +
    `${rust.snapshots.length} exact synchronized-machine snapshots.`,
);
