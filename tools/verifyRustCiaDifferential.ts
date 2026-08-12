// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - TypeScript/Rust CIA deterministic differential test
//
//   File:       verifyRustCiaDifferential.ts
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { spawn } from 'node:child_process';

import { Mos6526 } from '../src/devices/Mos6526';
import { MOS_6526_MODEL, type Mos6526Model } from '../src/devices/Mos6526Model';

interface WriteOperation {
  readonly kind: 'write';
  readonly address: number;
  readonly value: number;
}

interface ReadOperation {
  readonly kind: 'read';
  readonly address: number;
}

interface CountOperation {
  readonly kind: 'pulseCount';
  readonly pulses: number;
}

interface TickOperation {
  readonly kind: 'tick';
  readonly cycles: number;
}

interface BooleanOperation {
  readonly kind: 'setCountPinHigh' | 'setFlagPinHigh' | 'pulseSerialClock';
  readonly high: boolean;
}

interface TodOperation {
  readonly kind: 'tickTimeOfDayInput';
  readonly pulses: number;
}

interface ParameterlessOperation {
  readonly kind: 'clockCycle' | 'reset';
}

type Operation =
  | BooleanOperation
  | CountOperation
  | ParameterlessOperation
  | ReadOperation
  | TickOperation
  | TodOperation
  | WriteOperation;

interface Scenario {
  readonly revised: boolean;
  readonly processorClockHz: number;
  readonly timeOfDayInputHz: number;
  readonly operations: readonly Operation[];
}

interface Observation {
  readonly readValue: number | null;
  readonly portAOutputPins: number;
  readonly portBOutputPins: number;
  readonly stateFlags: number;
}

const PROCESSOR_CLOCK_HZ = 1_000;
const TIME_OF_DAY_INPUT_HZ = 50;
const RANDOM_OPERATION_COUNT = 20_000;
const MAXIMUM_TRACE_BYTES = 64 * 1024 * 1024;

function fixedRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return state >>> 0;
  };
}

function buildOperations(seed: number): readonly Operation[] {
  const random = fixedRandom(seed);
  const operations: Operation[] = [
    { kind: 'write', address: 0x04, value: 0x01 },
    { kind: 'write', address: 0x05, value: 0x00 },
    { kind: 'write', address: 0x0d, value: 0x81 },
    { kind: 'write', address: 0x0e, value: 0x91 },
    { kind: 'tick', cycles: 6 },
    { kind: 'read', address: 0x0d },
  ];
  for (let index = 0; index < RANDOM_OPERATION_COUNT; index += 1) {
    const choice = random() % 16;
    if (choice <= 4) {
      operations.push({ kind: 'write', address: random() & 0xffff, value: random() & 0xff });
    } else if (choice <= 7) {
      operations.push({ kind: 'read', address: random() & 0xffff });
    } else if (choice === 8) {
      operations.push({ kind: 'tick', cycles: random() % 7 });
    } else if (choice === 9) {
      operations.push({ kind: 'clockCycle' });
    } else if (choice === 10) {
      operations.push({ kind: 'pulseCount', pulses: random() % 5 });
    } else if (choice === 11) {
      operations.push({ kind: 'setCountPinHigh', high: (random() & 1) !== 0 });
    } else if (choice === 12) {
      operations.push({ kind: 'setFlagPinHigh', high: (random() & 1) !== 0 });
    } else if (choice === 13) {
      operations.push({ kind: 'tickTimeOfDayInput', pulses: random() % 9 });
    } else if (choice === 14) {
      operations.push({ kind: 'pulseSerialClock', high: (random() & 1) !== 0 });
    } else {
      operations.push({ kind: 'reset' });
    }
  }
  return operations;
}

function observe(cia: Mos6526, readValue: number | null): Observation {
  const stateFlags =
    Number(cia.interruptPending) |
    (Number(cia.portControlOutputHigh) << 1) |
    (Number(cia.serialClockOutputHigh) << 2) |
    (Number(cia.serialDataOutputHigh) << 3);
  return {
    readValue,
    portAOutputPins: cia.portAOutputPins,
    portBOutputPins: cia.portBOutputPins,
    stateFlags,
  };
}

function runTypeScriptScenario(
  model: Mos6526Model,
  operations: readonly Operation[],
): readonly Observation[] {
  const cia = new Mos6526('CIA differential oracle', {
    model,
    timing: {
      processorClockHz: PROCESSOR_CLOCK_HZ,
      timeOfDayInputHz: TIME_OF_DAY_INPUT_HZ,
    },
  });
  return operations.map((operation) => {
    let readValue: number | null = null;
    switch (operation.kind) {
      case 'write':
        cia.write(operation.address, operation.value);
        break;
      case 'read':
        readValue = cia.read(operation.address);
        break;
      case 'tick':
        cia.tick(operation.cycles);
        break;
      case 'clockCycle':
        cia.clockCycle();
        break;
      case 'pulseCount':
        cia.pulseCount(operation.pulses);
        break;
      case 'setCountPinHigh':
        cia.setCountPin(operation.high);
        break;
      case 'setFlagPinHigh':
        cia.setFlagPinHigh(operation.high);
        break;
      case 'tickTimeOfDayInput':
        cia.tickTimeOfDayInput(operation.pulses);
        break;
      case 'pulseSerialClock':
        cia.pulseSerialClock(operation.high);
        break;
      case 'reset':
        cia.reset();
        break;
    }
    return observe(cia, readValue);
  });
}

async function requireRustResults(
  scenarios: readonly Scenario[],
): Promise<readonly (readonly Observation[])[]> {
  const child = spawn(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'cia_trace'],
    {
      cwd: process.cwd(),
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  );

  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  let stdoutBytes = 0;
  let stderrBytes = 0;
  let outputLimitExceeded = false;
  child.stdout.on('data', (chunk: Buffer) => {
    stdoutBytes += chunk.byteLength;
    if (stdoutBytes > MAXIMUM_TRACE_BYTES) {
      outputLimitExceeded = true;
      child.kill();
      return;
    }
    stdout.push(chunk);
  });
  child.stderr.on('data', (chunk: Buffer) => {
    stderrBytes += chunk.byteLength;
    if (stderrBytes > MAXIMUM_TRACE_BYTES) {
      outputLimitExceeded = true;
      child.kill();
      return;
    }
    stderr.push(chunk);
  });

  const completion = new Promise<number | null>((resolve, reject) => {
    child.once('error', reject);
    child.once('close', resolve);
  });
  child.stdin.end(JSON.stringify(scenarios));
  const exitCode = await completion;
  if (outputLimitExceeded) {
    throw new Error(`Rust CIA trace adapter exceeded ${MAXIMUM_TRACE_BYTES} output bytes.`);
  }
  if (exitCode !== 0) {
    throw new Error(
      `Rust CIA trace adapter failed (${String(exitCode)}):\n${Buffer.concat(stderr).toString('utf8')}`,
    );
  }
  return JSON.parse(Buffer.concat(stdout).toString('utf8')) as readonly (readonly Observation[])[];
}

const operations = buildOperations(0x6526_2026);
const scenarios: readonly Scenario[] = [
  {
    revised: false,
    processorClockHz: PROCESSOR_CLOCK_HZ,
    timeOfDayInputHz: TIME_OF_DAY_INPUT_HZ,
    operations,
  },
  {
    revised: true,
    processorClockHz: PROCESSOR_CLOCK_HZ,
    timeOfDayInputHz: TIME_OF_DAY_INPUT_HZ,
    operations,
  },
];
const rustResults = await requireRustResults(scenarios);
const models = [MOS_6526_MODEL.original, MOS_6526_MODEL.revised] as const;
for (const [scenarioIndex, model] of models.entries()) {
  const expected = runTypeScriptScenario(model, operations);
  const actual = rustResults[scenarioIndex];
  if (actual?.length !== expected.length) {
    throw new Error(`Rust CIA trace length mismatch for ${model}.`);
  }
  for (let operationIndex = 0; operationIndex < expected.length; operationIndex += 1) {
    const expectedObservation = expected[operationIndex];
    const actualObservation = actual[operationIndex];
    if (JSON.stringify(actualObservation) !== JSON.stringify(expectedObservation)) {
      const historyStart = Math.max(0, operationIndex - 64);
      throw new Error(
        `CIA ${model} mismatch after operation ${operationIndex} ` +
          `${JSON.stringify(operations[operationIndex])}: expected ` +
          `${JSON.stringify(expectedObservation)}, received ${JSON.stringify(actualObservation)}. ` +
          `Recent operations: ${JSON.stringify(operations.slice(historyStart, operationIndex + 1))}.`,
      );
    }
  }
}

console.log(
  `PASS Rust/TypeScript CIA differential: ${models.length} models × ${operations.length} ` +
    'deterministic operations, exact register/pin/interrupt observations.',
);
