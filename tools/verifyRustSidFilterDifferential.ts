// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - SID 6581/8580 滤波器逐周期差分验证
//
//   文件:       verifyRustSidFilterDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';

import { SidFilter } from '../src/devices/SidFilter';
import { SID_MODEL, type SidModel } from '../src/devices/SidModel';
import { SID_FILTER_BIT } from '../src/devices/sidRegisters';

type Operation =
  | {
      readonly externalInput?: number;
      readonly kind: 'clock';
      readonly voice1: number;
      readonly voice2: number;
      readonly voice3: number;
    }
  | {
      readonly cutoff: number;
      readonly kind: 'registers';
      readonly modeVolume: number;
      readonly resonanceRouting: number;
    }
  | { readonly kind: 'reset' };

interface Scenario {
  readonly model: SidModel;
  readonly operations: readonly Operation[];
}

interface Observation {
  readonly cutoff: number;
  readonly outputPcm: number;
}

const RANDOM_OPERATION_COUNT = 60_000;

function fixedRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return state >>> 0;
  };
}

function voiceOutput(random: () => number, model: SidModel): number {
  const waveformDacZero = model === SID_MODEL.mos6581 ? 0x0380 : 0x09e0;
  return ((random() & 0x0fff) - waveformDacZero) * (random() & 0xff);
}

function buildOperations(model: SidModel, seed: number): readonly Operation[] {
  const operations: Operation[] = [
    {
      kind: 'registers',
      cutoff: 0x0640,
      resonanceRouting: 0xa0 | SID_FILTER_BIT.voice1,
      modeVolume: SID_FILTER_BIT.lowPass | 0x0f,
    },
    { kind: 'clock', voice1: 400_000, voice2: 0, voice3: 0 },
  ];
  for (let cycle = 0; cycle < 127; cycle += 1) {
    operations.push({ kind: 'clock', voice1: 0, voice2: 0, voice3: 0 });
  }

  const random = fixedRandom(seed);
  for (let operation = 0; operation < RANDOM_OPERATION_COUNT; operation += 1) {
    const choice = random() & 0xff;
    if (choice < 230) {
      const external = random();
      operations.push({
        kind: 'clock',
        voice1: voiceOutput(random, model),
        voice2: voiceOutput(random, model),
        voice3: voiceOutput(random, model),
        ...((external & 3) === 0 ? { externalInput: (external << 16) >> 16 } : {}),
      });
    } else if (choice < 254) {
      operations.push({
        kind: 'registers',
        cutoff: random() & 0x07ff,
        resonanceRouting: random() & 0xff,
        modeVolume: random() & 0xff,
      });
    } else {
      operations.push({ kind: 'reset' });
    }
  }
  return operations;
}

function runTypeScript(scenario: Scenario): readonly Observation[] {
  const filter = new SidFilter(scenario.model, 1_000_000);
  return scenario.operations.map((operation) => {
    switch (operation.kind) {
      case 'clock':
        filter.clock(operation.voice1, operation.voice2, operation.voice3, operation.externalInput);
        break;
      case 'registers':
        filter.cutoff = operation.cutoff;
        filter.resonanceRouting = operation.resonanceRouting;
        filter.modeVolume = operation.modeVolume;
        break;
      case 'reset':
        filter.reset();
        break;
    }
    return { cutoff: filter.cutoff, outputPcm: filter.outputPcm };
  });
}

function runRust(scenarios: readonly Scenario[]): readonly (readonly Observation[])[] {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'sid_filter_trace'],
    {
      cwd: process.cwd(),
      encoding: 'utf8',
      input: JSON.stringify(scenarios),
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Rust SID filter adapter failed (${String(result.status)}):\n${result.stderr}`);
  }
  return JSON.parse(result.stdout) as readonly (readonly Observation[])[];
}

const scenarios: readonly Scenario[] = [
  { model: SID_MODEL.mos6581, operations: buildOperations(SID_MODEL.mos6581, 0x6581_2026) },
  { model: SID_MODEL.mos8580, operations: buildOperations(SID_MODEL.mos8580, 0x8580_2026) },
];
const actualScenarios = runRust(scenarios);
let verifiedOperations = 0;
for (const [scenarioIndex, scenario] of scenarios.entries()) {
  const expected = runTypeScript(scenario);
  const actual = actualScenarios[scenarioIndex];
  if (actual?.length !== expected.length) {
    throw new Error(`Rust SID ${scenario.model} filter trace length mismatch.`);
  }
  for (let index = 0; index < expected.length; index += 1) {
    const expectedObservation = expected[index];
    const actualObservation = actual[index];
    if (JSON.stringify(actualObservation) !== JSON.stringify(expectedObservation)) {
      const historyStart = Math.max(0, index - 24);
      throw new Error(
        `SID ${scenario.model} filter mismatch after operation ${index} ` +
          `${JSON.stringify(scenario.operations[index])}: expected ${JSON.stringify(expectedObservation)}, ` +
          `received ${JSON.stringify(actualObservation)}. Recent operations: ` +
          `${JSON.stringify(scenario.operations.slice(historyStart, index + 1))}.`,
      );
    }
  }
  verifiedOperations += expected.length;
}

console.log(
  `PASS Rust/TypeScript SID filter differential: ${verifiedOperations.toLocaleString('en-US')} ` +
    'MOS 6581/8580 register, routing, nonlinear integration, reset and external-input operations.',
);
