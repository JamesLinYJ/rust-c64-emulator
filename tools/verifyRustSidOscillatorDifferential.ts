// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - SID 三声部振荡器逐周期差分验证
//
//   文件:       verifyRustSidOscillatorDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { runRustJsonTraceSync } from './runRustJsonTrace';

import { SID_MODEL, type SidModel } from '../src/devices/SidModel';
import { SidOscillator } from '../src/devices/SidOscillator';
import { SID_CONTROL_BIT } from '../src/devices/sidRegisters';

type Operation =
  | { readonly kind: 'clockCycle' }
  | { readonly index: 0 | 1 | 2; readonly kind: 'control'; readonly value: number }
  | { readonly index: 0 | 1 | 2; readonly kind: 'frequency'; readonly value: number }
  | { readonly index: 0 | 1 | 2; readonly kind: 'pulseWidth'; readonly value: number }
  | { readonly kind: 'reset' };

interface Scenario {
  readonly model: SidModel;
  readonly operations: readonly Operation[];
}

interface Observation {
  readonly readback: readonly [number, number, number];
  readonly waveform: readonly [number, number, number];
}

const RANDOM_OPERATION_COUNT = 70_000;

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

function buildOperations(seed: number): readonly Operation[] {
  const operations: Operation[] = [
    { kind: 'frequency', index: 0, value: 0x7100 },
    { kind: 'frequency', index: 1, value: 0x9300 },
    { kind: 'frequency', index: 2, value: 0xb500 },
    {
      kind: 'control',
      index: 0,
      value: SID_CONTROL_BIT.triangle | SID_CONTROL_BIT.synchronize,
    },
    {
      kind: 'control',
      index: 1,
      value: SID_CONTROL_BIT.triangle | SID_CONTROL_BIT.ringModulation,
    },
    { kind: 'control', index: 2, value: SID_CONTROL_BIT.sawtooth },
  ];
  appendClockCycles(operations, 20_000);
  operations.push(
    { kind: 'frequency', index: 0, value: 0xffff },
    { kind: 'control', index: 0, value: SID_CONTROL_BIT.noise },
  );
  appendClockCycles(operations, 20_000);
  operations.push({
    kind: 'control',
    index: 0,
    value: SID_CONTROL_BIT.test | SID_CONTROL_BIT.noise | SID_CONTROL_BIT.triangle,
  });
  appendClockCycles(operations, 40_000);
  operations.push({ kind: 'control', index: 0, value: SID_CONTROL_BIT.noise });
  appendClockCycles(operations, 32);

  const random = fixedRandom(seed);
  for (let operation = 0; operation < RANDOM_OPERATION_COUNT; operation += 1) {
    const choice = random() % 32;
    const index = (random() % 3) as 0 | 1 | 2;
    if (choice < 25) {
      operations.push({ kind: 'clockCycle' });
    } else if (choice <= 26) {
      operations.push({ kind: 'frequency', index, value: random() & 0xffff });
    } else if (choice === 27) {
      operations.push({ kind: 'pulseWidth', index, value: random() & 0xffff });
    } else if (choice <= 30) {
      operations.push({ kind: 'control', index, value: random() & 0xff });
    } else {
      operations.push({ kind: 'reset' });
    }
  }
  return operations;
}

function observe(oscillators: readonly [SidOscillator, SidOscillator, SidOscillator]): Observation {
  return {
    readback: oscillators.map((oscillator) => oscillator.oscillatorReadback) as [
      number,
      number,
      number,
    ],
    waveform: oscillators.map((oscillator) => oscillator.waveformOutput) as [
      number,
      number,
      number,
    ],
  };
}

function runTypeScript(scenario: Scenario): readonly Observation[] {
  const oscillators: [SidOscillator, SidOscillator, SidOscillator] = [
    new SidOscillator(scenario.model),
    new SidOscillator(scenario.model),
    new SidOscillator(scenario.model),
  ];
  oscillators[0].setSyncSource(oscillators[2]);
  oscillators[1].setSyncSource(oscillators[0]);
  oscillators[2].setSyncSource(oscillators[1]);

  return scenario.operations.map((operation) => {
    switch (operation.kind) {
      case 'clockCycle':
        for (const oscillator of oscillators) oscillator.clock();
        for (const oscillator of oscillators) oscillator.synchronizeDestination();
        for (const oscillator of oscillators) oscillator.updateWaveformOutput();
        break;
      case 'control':
        oscillators[operation.index].setControl(operation.value);
        break;
      case 'frequency':
        oscillators[operation.index].frequency = operation.value;
        break;
      case 'pulseWidth':
        oscillators[operation.index].pulseWidth = operation.value;
        break;
      case 'reset':
        for (const oscillator of oscillators) oscillator.reset();
        break;
    }
    return observe(oscillators);
  });
}

function runRust(scenarios: readonly Scenario[]): readonly (readonly Observation[])[] {
  return runRustJsonTraceSync<readonly (readonly Observation[])[]>({
    example: 'sid_oscillator_trace',
    input: scenarios,
    label: 'Rust SID oscillator adapter',
    maximumOutputBytes: 96 * 1024 * 1024,
  });
}

const scenarios: readonly Scenario[] = [
  { model: SID_MODEL.mos6581, operations: buildOperations(0x6581_2026) },
  { model: SID_MODEL.mos8580, operations: buildOperations(0x8580_2026) },
];
const rustResults = runRust(scenarios);
let verifiedOperations = 0;
for (const [scenarioIndex, scenario] of scenarios.entries()) {
  const expected = runTypeScript(scenario);
  const actual = rustResults[scenarioIndex];
  if (actual?.length !== expected.length) {
    throw new Error(`Rust SID ${scenario.model} oscillator trace length mismatch.`);
  }
  for (let index = 0; index < expected.length; index += 1) {
    const expectedObservation = expected[index];
    const actualObservation = actual[index];
    if (JSON.stringify(actualObservation) !== JSON.stringify(expectedObservation)) {
      const historyStart = Math.max(0, index - 32);
      throw new Error(
        `SID ${scenario.model} oscillator mismatch after operation ${index} ` +
          `${JSON.stringify(scenario.operations[index])}: expected ${JSON.stringify(expectedObservation)}, ` +
          `received ${JSON.stringify(actualObservation)}. Recent operations: ` +
          `${JSON.stringify(scenario.operations.slice(historyStart, index + 1))}.`,
      );
    }
  }
  verifiedOperations += expected.length;
}

console.log(
  `PASS Rust/TypeScript SID oscillator differential: ${verifiedOperations.toLocaleString('en-US')} ` +
    'MOS 6581/8580 operations with exact three-voice sync, ring, TEST, noise and waveform outputs.',
);
