// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - SID 包络逐周期差分验证
//
//   文件:       verifyRustSidEnvelopeDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';

import { SidEnvelopeGenerator } from '../src/devices/SidEnvelopeGenerator';

type Operation =
  | { readonly kind: 'attackDecay'; readonly value: number }
  | { readonly kind: 'clockCycle' }
  | { readonly kind: 'control'; readonly value: number }
  | { readonly kind: 'reset' }
  | { readonly kind: 'sustainRelease'; readonly value: number };

interface Observation {
  readonly output: number;
  readonly readback: number;
}

const RANDOM_OPERATION_COUNT = 80_000;

function fixedRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return state >>> 0;
  };
}

function appendClockCycles(operations: Operation[], cycles: number): void {
  for (let cycle = 0; cycle < cycles; cycle += 1) operations.push({ kind: 'clockCycle' });
}

function buildOperations(): readonly Operation[] {
  const operations: Operation[] = [
    { kind: 'attackDecay', value: 0x00 },
    { kind: 'sustainRelease', value: 0x80 },
    { kind: 'control', value: 0x01 },
  ];
  appendClockCycles(operations, 3_000);
  operations.push({ kind: 'control', value: 0x00 });
  appendClockCycles(operations, 6_000);
  operations.push(
    { kind: 'attackDecay', value: 0x90 },
    { kind: 'sustainRelease', value: 0xf0 },
    { kind: 'control', value: 0x01 },
  );
  appendClockCycles(operations, 800);
  operations.push({ kind: 'attackDecay', value: 0x00 });
  appendClockCycles(operations, 33_000);

  const random = fixedRandom(0x6581_ad5e);
  for (let index = 0; index < RANDOM_OPERATION_COUNT; index += 1) {
    const choice = random() % 32;
    if (choice < 26) {
      operations.push({ kind: 'clockCycle' });
    } else if (choice === 26) {
      operations.push({ kind: 'attackDecay', value: random() & 0xff });
    } else if (choice === 27) {
      operations.push({ kind: 'sustainRelease', value: random() & 0xff });
    } else if (choice <= 30) {
      operations.push({ kind: 'control', value: random() & 0xff });
    } else {
      operations.push({ kind: 'reset' });
    }
  }
  return operations;
}

function runTypeScript(operations: readonly Operation[]): readonly Observation[] {
  const envelope = new SidEnvelopeGenerator();
  return operations.map((operation) => {
    switch (operation.kind) {
      case 'attackDecay':
        envelope.writeAttackDecay(operation.value);
        break;
      case 'sustainRelease':
        envelope.writeSustainRelease(operation.value);
        break;
      case 'control':
        envelope.writeControl(operation.value);
        break;
      case 'clockCycle':
        envelope.clock();
        break;
      case 'reset':
        envelope.reset();
        break;
    }
    return { output: envelope.output, readback: envelope.readback };
  });
}

function runRust(operations: readonly Operation[]): readonly Observation[] {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'sid_envelope_trace'],
    {
      cwd: process.cwd(),
      encoding: 'utf8',
      input: JSON.stringify(operations),
      maxBuffer: 32 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `Rust SID envelope adapter failed (${String(result.status)}):\n${result.stderr}`,
    );
  }
  return JSON.parse(result.stdout) as readonly Observation[];
}

const operations = buildOperations();
const expected = runTypeScript(operations);
const actual = runRust(operations);
if (actual.length !== expected.length) {
  throw new Error(
    `Rust SID envelope trace has ${actual.length} observations; expected ${expected.length}.`,
  );
}
for (let index = 0; index < expected.length; index += 1) {
  const expectedObservation = expected[index];
  const actualObservation = actual[index];
  if (
    actualObservation?.output !== expectedObservation?.output ||
    actualObservation.readback !== expectedObservation.readback
  ) {
    const historyStart = Math.max(0, index - 32);
    throw new Error(
      `SID envelope mismatch after operation ${index} ${JSON.stringify(operations[index])}: ` +
        `expected ${JSON.stringify(expectedObservation)}, received ${JSON.stringify(actualObservation)}. ` +
        `Recent operations: ${JSON.stringify(operations.slice(historyStart, index + 1))}.`,
    );
  }
}

console.log(
  `PASS Rust/TypeScript SID envelope differential: ${operations.length.toLocaleString('en-US')} ` +
    'deterministic operations including ADSR delay, gate pipelines, RES retention and ENV3 latency.',
);
