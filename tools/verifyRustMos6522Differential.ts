// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - MOS 6522 VIA 逐操作差分验证
//
//   文件:       verifyRustMos6522Differential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { isDeepStrictEqual } from 'node:util';

import { Mos6522 } from '../src/devices/Mos6522';
import {
  MOS_6522_CONTROL_LINE,
  MOS_6522_INTERRUPT_BIT,
  MOS_6522_PCR_CONTROL_MODE,
  MOS_6522_REGISTER,
  MOS_6522_SHIFT_MODE,
  type Mos6522ControlLine,
} from '../src/devices/Mos6522Registers';
import { runRustJsonTraceSync } from './runRustJsonTrace';

type Operation =
  | { readonly cycles: number; readonly kind: 'clock' }
  | { readonly kind: 'externalPorts'; readonly portA: number; readonly portB: number }
  | { readonly high: boolean; readonly kind: 'portB6' }
  | { readonly address: number; readonly kind: 'read' }
  | { readonly kind: 'reset' }
  | { readonly high: boolean; readonly kind: 'signal'; readonly line: Mos6522ControlLine }
  | { readonly address: number; readonly kind: 'write'; readonly value: number };

interface Observation {
  readonly controlOutputs: number;
  readonly interruptEnable: number;
  readonly interruptFlags: number;
  readonly interruptPending: boolean;
  readonly portAOutputPins: number;
  readonly portBOutputPins: number;
  readonly readValue: number | null;
}

const RANDOM_OPERATION_COUNT = 100_000;

class DifferentialVia extends Mos6522 {
  externalPortA = 0xff;
  externalPortB = 0xff;
  readonly controlState: Record<Mos6522ControlLine, boolean> = {
    ca1: true,
    ca2: true,
    cb1: true,
    cb2: true,
  };

  protected override readPortAExternalInputs(): number {
    return this.externalPortA;
  }

  protected override readPortBExternalInputs(): number {
    return this.externalPortB;
  }

  protected override onControlLineOutputChanged(line: Mos6522ControlLine, high: boolean): void {
    if (this.controlState) this.controlState[line] = high;
  }
}

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
  const operations: Operation[] = [
    { kind: 'externalPorts', portA: 0xa0, portB: 0x5a },
    { kind: 'write', address: MOS_6522_REGISTER.dataDirectionA, value: 0x0f },
    { kind: 'write', address: MOS_6522_REGISTER.portA, value: 0x05 },
    {
      kind: 'write',
      address: MOS_6522_REGISTER.interruptEnable,
      value: MOS_6522_INTERRUPT_BIT.any | MOS_6522_INTERRUPT_BIT.timer1,
    },
    { kind: 'write', address: MOS_6522_REGISTER.timer1CounterLow, value: 0x02 },
    { kind: 'write', address: MOS_6522_REGISTER.timer1CounterHigh, value: 0x00 },
    { kind: 'clock', cycles: 4 },
    { kind: 'read', address: MOS_6522_REGISTER.timer1CounterLow },
    {
      kind: 'write',
      address: MOS_6522_REGISTER.auxiliaryControl,
      value: MOS_6522_SHIFT_MODE.outputProcessorClock << 2,
    },
    { kind: 'write', address: MOS_6522_REGISTER.shiftRegister, value: 0xa5 },
    { kind: 'clock', cycles: 17 },
    {
      kind: 'write',
      address: MOS_6522_REGISTER.peripheralControl,
      value:
        (MOS_6522_PCR_CONTROL_MODE.pulseOutput << 1) |
        (MOS_6522_PCR_CONTROL_MODE.handshakeOutput << 5),
    },
    { kind: 'write', address: MOS_6522_REGISTER.portA, value: 0 },
    { kind: 'write', address: MOS_6522_REGISTER.portB, value: 0 },
    { kind: 'clock', cycles: 1 },
  ];
  const random = fixedRandom(0x6522_2026);
  const lines = Object.values(MOS_6522_CONTROL_LINE);
  for (let operation = 0; operation < RANDOM_OPERATION_COUNT; operation += 1) {
    const choice = random() & 0xff;
    if (choice < 105) {
      operations.push({ kind: 'clock', cycles: 1 + (random() & 0x0f) });
    } else if (choice < 170) {
      operations.push({ kind: 'write', address: random() & 0x3f, value: random() & 0xff });
    } else if (choice < 205) {
      operations.push({ kind: 'read', address: random() & 0x3f });
    } else if (choice < 225) {
      const line = lines[random() % lines.length];
      if (line === undefined) throw new Error('MOS 6522 control line matrix is empty.');
      operations.push({ kind: 'signal', line, high: (random() & 1) !== 0 });
    } else if (choice < 239) {
      operations.push({ kind: 'portB6', high: (random() & 1) !== 0 });
    } else if (choice < 253) {
      operations.push({ kind: 'externalPorts', portA: random() & 0xff, portB: random() & 0xff });
    } else {
      operations.push({ kind: 'reset' });
    }
  }
  return operations;
}

function observe(via: DifferentialVia, readValue: number | null): Observation {
  return {
    controlOutputs:
      Number(via.controlState[MOS_6522_CONTROL_LINE.ca2]) |
      (Number(via.controlState[MOS_6522_CONTROL_LINE.cb1]) << 1) |
      (Number(via.controlState[MOS_6522_CONTROL_LINE.cb2]) << 2),
    interruptEnable: via.read(MOS_6522_REGISTER.interruptEnable),
    interruptFlags: via.read(MOS_6522_REGISTER.interruptFlags),
    interruptPending: via.interruptPending,
    portAOutputPins: via.portAOutputPins,
    portBOutputPins: via.portBOutputPins,
    readValue,
  };
}

function runTypeScript(operations: readonly Operation[]): readonly Observation[] {
  const via = new DifferentialVia('differential VIA');
  return operations.map((operation) => {
    let readValue: number | null = null;
    switch (operation.kind) {
      case 'clock':
        via.tick(operation.cycles);
        break;
      case 'externalPorts':
        via.externalPortA = operation.portA;
        via.externalPortB = operation.portB;
        break;
      case 'portB6':
        via.signalPortB6(operation.high);
        break;
      case 'read':
        readValue = via.read(operation.address);
        break;
      case 'reset':
        via.reset();
        via.controlState.cb1 = true;
        break;
      case 'signal':
        via.signalControlLine(operation.line, operation.high);
        break;
      case 'write':
        via.write(operation.address, operation.value);
        break;
    }
    return observe(via, readValue);
  });
}

function runRust(operations: readonly Operation[]): readonly Observation[] {
  return runRustJsonTraceSync<readonly Observation[]>({
    example: 'via_trace',
    input: operations,
    label: 'Rust MOS 6522 adapter',
    maximumOutputBytes: 96 * 1024 * 1024,
  });
}

const operations = buildOperations();
const expected = runTypeScript(operations);
const actual = runRust(operations);
if (actual.length !== expected.length) throw new Error('Rust MOS 6522 trace length mismatch.');
for (let index = 0; index < expected.length; index += 1) {
  if (!isDeepStrictEqual(actual[index], expected[index])) {
    const historyStart = Math.max(0, index - 24);
    throw new Error(
      `MOS 6522 mismatch after operation ${index} ${JSON.stringify(operations[index])}: ` +
        `expected ${JSON.stringify(expected[index])}, received ${JSON.stringify(actual[index])}. ` +
        `Recent operations: ${JSON.stringify(operations.slice(historyStart, index + 1))}.`,
    );
  }
}

console.log(
  `PASS Rust/TypeScript MOS 6522 differential: ${operations.length.toLocaleString('en-US')} ` +
    'register, timer, shift, port, control-line, IRQ and reset operations.',
);
