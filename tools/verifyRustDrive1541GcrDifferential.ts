// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 1541 GCR 读写分离电路差分验证
//
//   文件:       verifyRustDrive1541GcrDifferential.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

import {
  Drive1541GcrCircuit,
  type Drive1541GcrBit,
  type Drive1541SpeedZone,
} from '../src/peripherals/drive1541/Drive1541GcrCircuit';

type Operation =
  | { readonly kind: 'advance'; readonly ticks: number }
  | { readonly enabled: boolean; readonly kind: 'byteReadyEnabled' }
  | { readonly kind: 'flux' }
  | { readonly kind: 'readMode'; readonly reading: boolean }
  | { readonly kind: 'reset' }
  | { readonly kind: 'speedZone'; readonly zone: Drive1541SpeedZone }
  | { readonly kind: 'writeData'; readonly value: number };

interface Observation {
  readonly byteReady: readonly number[];
  readonly byteReadyEnabled: boolean;
  readonly dataByte: number;
  readonly reading: boolean;
  readonly referenceTicksUntilNextShift: number;
  readonly speedZone: number;
  readonly syncFound: boolean;
  readonly weakFluxTicksRemaining: number;
  readonly writeBits: readonly boolean[];
  readonly writeDataByte: number;
}

const RANDOM_OPERATION_COUNT = 100_000;

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
    { kind: 'speedZone', zone: 3 },
    { kind: 'readMode', reading: false },
    { kind: 'writeData', value: 0xa5 },
    { kind: 'advance', ticks: 1_000 },
    { kind: 'readMode', reading: true },
    { kind: 'flux' },
    { kind: 'advance', ticks: 40 },
    { kind: 'reset' },
  ];
  const random = fixedRandom(0x1541_6c72);
  for (let index = 0; index < RANDOM_OPERATION_COUNT; index += 1) {
    const choice = random() & 0xff;
    if (choice < 150) {
      operations.push({ kind: 'advance', ticks: random() % 257 });
    } else if (choice < 177) {
      operations.push({ kind: 'flux' });
    } else if (choice < 198) {
      operations.push({ kind: 'readMode', reading: (random() & 1) !== 0 });
    } else if (choice < 218) {
      operations.push({ kind: 'writeData', value: random() & 0xff });
    } else if (choice < 234) {
      operations.push({ kind: 'speedZone', zone: (random() & 3) as Drive1541SpeedZone });
    } else if (choice < 250) {
      operations.push({ kind: 'byteReadyEnabled', enabled: (random() & 1) !== 0 });
    } else {
      operations.push({ kind: 'reset' });
    }
  }
  return operations;
}

function runTypeScript(operations: readonly Operation[]): readonly Observation[] {
  const byteReady: number[] = [];
  const writeBits: Drive1541GcrBit[] = [];
  const circuit = new Drive1541GcrCircuit({
    signalByteReady: (dataByte) => byteReady.push(dataByte),
    writeFluxBit: (bit) => writeBits.push(bit),
  });
  let byteReadyEnabled = true;
  return operations.map((operation) => {
    byteReady.length = 0;
    writeBits.length = 0;
    switch (operation.kind) {
      case 'advance':
        circuit.advance(operation.ticks);
        break;
      case 'byteReadyEnabled':
        byteReadyEnabled = operation.enabled;
        circuit.setByteReadyEnabled(operation.enabled);
        break;
      case 'flux':
        circuit.observeRecordedFluxReversal();
        break;
      case 'readMode':
        circuit.setReadMode(operation.reading);
        break;
      case 'reset':
        circuit.reset();
        break;
      case 'speedZone':
        circuit.setSpeedZone(operation.zone);
        break;
      case 'writeData':
        circuit.setWriteDataByte(operation.value);
        break;
    }
    return {
      byteReady: [...byteReady],
      byteReadyEnabled,
      dataByte: circuit.dataByte,
      reading: circuit.reading,
      referenceTicksUntilNextShift: circuit.referenceTicksUntilNextShift,
      speedZone: circuit.speedZone,
      syncFound: circuit.syncFound,
      weakFluxTicksRemaining: circuit.weakFluxTicksRemaining,
      writeBits: writeBits.map(Boolean),
      writeDataByte: circuit.writeDataByte,
    };
  });
}

function runRust(operations: readonly Operation[]): readonly Observation[] {
  const result = spawnSync(
    'cargo',
    ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', 'gcr_trace'],
    {
      encoding: 'utf8',
      input: JSON.stringify(operations),
      maxBuffer: 128 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    throw new Error(`Rust 1541 GCR adapter failed:\n${result.stderr || result.stdout}`);
  }
  return JSON.parse(result.stdout) as readonly Observation[];
}

const operations = buildOperations();
const typescript = runTypeScript(operations);
const rust = runRust(operations);
if (typescript.length !== rust.length) {
  throw new Error(`Rust returned ${rust.length} observations for ${typescript.length} operations.`);
}
for (let index = 0; index < operations.length; index += 1) {
  if (!isDeepStrictEqual(typescript[index], rust[index])) {
    throw new Error(
      `1541 GCR differential mismatch at operation ${index}: ` +
        `${JSON.stringify(operations[index])}\nTypeScript=${JSON.stringify(typescript[index])}` +
        `\nRust=${JSON.stringify(rust[index])}`,
    );
  }
}

console.log(
  `PASS Rust/TypeScript 1541 GCR differential: ${operations.length.toLocaleString('en-US')} ` +
    'divider, flux, sync, weak-bit, write and BYTE READY operations.',
);
