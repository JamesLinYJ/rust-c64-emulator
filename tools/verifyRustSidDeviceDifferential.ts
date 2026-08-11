// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 完整 SID 芯片寄存器与音频差分验证
//
//   文件:       verifyRustSidDeviceDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import { Sid, type SidVoiceState } from '../src/devices/Sid';
import { SID_MODEL, type SidModel } from '../src/devices/SidModel';
import { SID_CONTROL_BIT, SID_FILTER_BIT, SID_REGISTER } from '../src/devices/sidRegisters';

type Operation =
  | { readonly cycles: number; readonly kind: 'clock' }
  | { readonly kind: 'drain'; readonly maximumLength?: number }
  | { readonly kind: 'paddles'; readonly x: number; readonly y: number }
  | { readonly address: number; readonly kind: 'read' }
  | { readonly kind: 'reset' }
  | { readonly address: number; readonly kind: 'write'; readonly value: number };

interface Scenario {
  readonly model: SidModel;
  readonly operations: readonly Operation[];
  readonly processorClockHz: number;
  readonly sampleRateHz: number;
}

interface Observation {
  readonly filterCutoff: number;
  readonly masterVolume: number;
  readonly pendingSampleCount: number;
  readonly readValue: number | null;
  readonly sampleBits: readonly number[];
  readonly voices: readonly SidVoiceState[];
}

const RANDOM_OPERATION_COUNT = 24_000;

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
  const operations: Operation[] = [
    { kind: 'write', address: 0x00, value: 0x34 },
    { kind: 'write', address: 0x01, value: 0x12 },
    { kind: 'write', address: 0x02, value: 0x80 },
    { kind: 'write', address: 0x03, value: 0x08 },
    { kind: 'write', address: 0x04, value: SID_CONTROL_BIT.gate | SID_CONTROL_BIT.noise },
    { kind: 'write', address: 0x05, value: 0x24 },
    { kind: 'write', address: 0x06, value: 0xf8 },
    { kind: 'write', address: 0x0b, value: SID_CONTROL_BIT.gate | SID_CONTROL_BIT.pulse },
    { kind: 'write', address: 0x12, value: SID_CONTROL_BIT.gate | SID_CONTROL_BIT.sawtooth },
    { kind: 'write', address: 0x15, value: 0x07 },
    { kind: 'write', address: 0x16, value: 0x90 },
    { kind: 'write', address: 0x17, value: 0xf7 },
    {
      kind: 'write',
      address: SID_REGISTER.filterModeVolume,
      value: SID_FILTER_BIT.lowPass | SID_FILTER_BIT.bandPass | 0x0f,
    },
    { kind: 'clock', cycles: 1_000 },
    { kind: 'drain' },
    { kind: 'paddles', x: 0x12, y: 0x34 },
    { kind: 'read', address: SID_REGISTER.paddleX },
    { kind: 'read', address: SID_REGISTER.paddleY },
    { kind: 'read', address: SID_REGISTER.oscillator3 },
    { kind: 'read', address: SID_REGISTER.envelope3 },
  ];

  const random = fixedRandom(seed);
  for (let operation = 0; operation < RANDOM_OPERATION_COUNT; operation += 1) {
    const choice = random() & 0xff;
    if (choice < 145) {
      operations.push({ kind: 'clock', cycles: 1 + (random() & 0x0f) });
    } else if (choice < 205) {
      operations.push({ kind: 'write', address: random() & 0x7f, value: random() & 0xff });
    } else if (choice < 228) {
      operations.push({ kind: 'read', address: random() & 0x7f });
    } else if (choice < 240) {
      const maximumLength = random() & 0x3f;
      operations.push((random() & 3) === 0 ? { kind: 'drain' } : { kind: 'drain', maximumLength });
    } else if (choice < 252) {
      operations.push({ kind: 'paddles', x: random() & 0xff, y: random() & 0xff });
    } else {
      operations.push({ kind: 'reset' });
    }
  }
  operations.push({ kind: 'clock', cycles: 2_000 }, { kind: 'drain' });
  return operations;
}

function sampleBits(samples: Float32Array<ArrayBufferLike>): readonly number[] {
  return Array.from(new Uint32Array(samples.buffer, samples.byteOffset, samples.length));
}

function runTypeScript(scenario: Scenario): readonly Observation[] {
  const sid = new Sid(false, {
    model: scenario.model,
    processorClockHz: scenario.processorClockHz,
    sampleRateHz: scenario.sampleRateHz,
  });
  return scenario.operations.map((operation) => {
    let readValue: number | null = null;
    let drained: Float32Array<ArrayBufferLike> = new Float32Array();
    switch (operation.kind) {
      case 'clock':
        sid.tick(operation.cycles);
        break;
      case 'drain':
        drained = sid.drainSamples(operation.maximumLength);
        break;
      case 'paddles':
        sid.setPaddleInputs(operation.x, operation.y);
        break;
      case 'read':
        readValue = sid.read(operation.address);
        break;
      case 'reset':
        sid.reset();
        break;
      case 'write':
        sid.write(operation.address, operation.value);
        break;
    }
    return {
      filterCutoff: sid.filterCutoff,
      masterVolume: sid.masterVolume,
      pendingSampleCount: sid.pendingSampleCount,
      readValue,
      sampleBits: sampleBits(drained),
      voices: [sid.getVoice(0), sid.getVoice(1), sid.getVoice(2)],
    };
  });
}

function runRust(scenarios: readonly Scenario[]): readonly (readonly Observation[])[] {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'sid_device_trace'],
    {
      cwd: process.cwd(),
      encoding: 'utf8',
      input: JSON.stringify(scenarios),
      maxBuffer: 96 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Rust SID device adapter failed (${String(result.status)}):\n${result.stderr}`);
  }
  return JSON.parse(result.stdout) as readonly (readonly Observation[])[];
}

const scenarios: readonly Scenario[] = [
  {
    model: SID_MODEL.mos6581,
    processorClockHz: 985_248,
    sampleRateHz: 44_100,
    operations: buildOperations(0x6581_c64),
  },
  {
    model: SID_MODEL.mos8580,
    processorClockHz: 1_022_727,
    sampleRateHz: 48_000,
    operations: buildOperations(0x8580_c64),
  },
];
const actualScenarios = runRust(scenarios);
let verifiedOperations = 0;
for (const [scenarioIndex, scenario] of scenarios.entries()) {
  const expected = runTypeScript(scenario);
  const actual = actualScenarios[scenarioIndex];
  if (actual?.length !== expected.length) {
    throw new Error(`Rust SID ${scenario.model} device trace length mismatch.`);
  }
  for (let index = 0; index < expected.length; index += 1) {
    const expectedObservation = expected[index];
    const actualObservation = actual[index];
    if (!isDeepStrictEqual(actualObservation, expectedObservation)) {
      const historyStart = Math.max(0, index - 24);
      throw new Error(
        `SID ${scenario.model} device mismatch after operation ${index} ` +
          `${JSON.stringify(scenario.operations[index])}: expected ${JSON.stringify(expectedObservation)}, ` +
          `received ${JSON.stringify(actualObservation)}. Recent operations: ` +
          `${JSON.stringify(scenario.operations.slice(historyStart, index + 1))}.`,
      );
    }
  }
  verifiedOperations += expected.length;
}

console.log(
  `PASS Rust/TypeScript complete SID differential: ${verifiedOperations.toLocaleString('en-US')} ` +
    'MOS 6581/8580 bus, voice-ring, filter, paddle, latch and exact Float32 audio operations.',
);
