// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - MOS 6522 外部参考验证器
//
//   文件:       verifyMos6522Reference.ts
//
//   日期:       2026年07月17日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

import { Cpu6502 } from '../src/core/cpu/Cpu6502';
import {
  MOS_6522_ACR_BIT,
  MOS_6522_INTERRUPT_BIT,
  MOS_6522_REGISTER,
  MOS_6522_SHIFT_MODE,
} from '../src/devices/Mos6522Registers';
import { Drive1541DiskVia } from '../src/peripherals/drive1541/Drive1541DiskVia';
import { Drive1541IecVia } from '../src/peripherals/drive1541/Drive1541IecVia';
import { Drive1541Machine } from '../src/peripherals/drive1541/Drive1541Machine';
import { Drive1541Mechanism } from '../src/peripherals/drive1541/Drive1541Mechanism';
import {
  DRIVE_1541_MEMORY_LAYOUT,
  Drive1541Memory,
} from '../src/peripherals/drive1541/Drive1541Memory';
import { IecBus, IEC_LINE } from '../src/peripherals/iec/IecBus';
import { runRustJsonTrace } from './runRustJsonTrace';

interface Mos6522ReferenceFile {
  readonly byteLength: number;
  readonly caseCount: number;
  readonly file: string;
  readonly loadAddress: number;
  readonly sha256: string;
}

interface Mos6522Pb7Reference extends Mos6522ReferenceFile {
  readonly controlBeforeTimer: boolean;
  readonly dataDirectionB: number;
  readonly portB: number;
}

interface Mos6522TimerReference extends Mos6522ReferenceFile {
  readonly startTimer1: boolean;
}

const VICE_TEST_REVISION = 46_176;
const REFERENCE_DIRECTORY = 'testprogs/drive/viavarious';
const REFERENCE_HEADER_SIZE = 2;
const REFERENCE_SAMPLE_COUNT = 0x0100;
const SAMPLE_BUFFER_ADDRESS = 0x0700;
const TEST_PROGRAM_ADDRESS = DRIVE_1541_MEMORY_LAYOUT.rom.primaryStart;
const MAXIMUM_REPLAY_INSTRUCTIONS = 2_000;

const OPCODE = {
  branchNotEqual: 0xd0,
  incrementX: 0xe8,
  loadAccumulatorAbsolute: 0xad,
  loadAccumulatorImmediate: 0xa9,
  loadXImmediate: 0xa2,
  storeAccumulatorAbsolute: 0x8d,
  storeAccumulatorAbsoluteX: 0x9d,
} as const;

const PB7_REFERENCES: readonly Mos6522Pb7Reference[] = [
  {
    byteLength: 2_050,
    caseCount: 8,
    controlBeforeTimer: false,
    dataDirectionB: 0x00,
    file: 'via10ref.bin',
    loadAddress: 0x0a00,
    portB: 0x00,
    sha256: 'fe01ae9b1093a9d08f7fe956a7f3efc9866a4021d2b4071ed4ae69359d08d035',
  },
  {
    byteLength: 2_050,
    caseCount: 8,
    controlBeforeTimer: false,
    dataDirectionB: 0x80,
    file: 'via11ref.bin',
    loadAddress: 0x0a00,
    portB: 0x00,
    sha256: 'fe01ae9b1093a9d08f7fe956a7f3efc9866a4021d2b4071ed4ae69359d08d035',
  },
  {
    byteLength: 2_050,
    caseCount: 8,
    controlBeforeTimer: false,
    dataDirectionB: 0x00,
    file: 'via12ref.bin',
    loadAddress: 0x0a00,
    portB: 0x80,
    sha256: 'fe01ae9b1093a9d08f7fe956a7f3efc9866a4021d2b4071ed4ae69359d08d035',
  },
  {
    byteLength: 2_050,
    caseCount: 8,
    controlBeforeTimer: false,
    dataDirectionB: 0x80,
    file: 'via13ref.bin',
    loadAddress: 0x0a00,
    portB: 0x80,
    sha256: 'a6158a8bf6d30e308fdae227b1099b4c9c29a166c8fd7186b928815c9fd9150d',
  },
  {
    byteLength: 2_050,
    caseCount: 8,
    controlBeforeTimer: true,
    dataDirectionB: 0x80,
    file: 'via14ref.bin',
    loadAddress: 0x0a00,
    portB: 0x80,
    sha256: 'ba83597adddb358cdd6d7ac7d3d0a2056531dc96dc96de46e8235f97ce91c700',
  },
];

const TIMER_REFERENCES: readonly Mos6522TimerReference[] = [
  {
    byteLength: 3_075,
    caseCount: 12,
    file: 'via20ref.bin',
    loadAddress: 0x2000,
    sha256: '694c6f75d6417e5511941c02ffe707ddbedc3452970ac5c807931f403014e3f8',
    startTimer1: false,
  },
  {
    byteLength: 3_075,
    caseCount: 12,
    file: 'via21ref.bin',
    loadAddress: 0x2000,
    sha256: '5b7eefa9bc13280c0521d19393b3ec64e43aa124d06244e97237685b3216b2ca',
    startTimer1: true,
  },
];

const PB7_CONTROL_VALUES = [0x00, 0x80, 0x40, 0xc0] as const;
const TIMER_REFERENCE_CASES = [
  { timer1: 0x0013, timer2: 0x0114 },
  { timer1: 0x0014, timer2: 0x0115 },
  { timer1: 0x0015, timer2: 0x0116 },
  { timer1: 0x0017, timer2: 0x0118 },
  { timer1: 0x0018, timer2: 0x0119 },
  { timer1: 0x0028, timer2: 0x0101 },
  { timer1: 0x0038, timer2: 0x0102 },
  { timer1: 0x0048, timer2: 0x0103 },
  { timer1: 0x0050, timer2: 0x0104 },
  { timer1: 0x0051, timer2: 0x0104 },
  { timer1: 0x0052, timer2: 0x0104 },
  { timer1: 0x0053, timer2: 0x0117 },
] as const;

const TIMER_REFERENCE_AUXILIARY_CONTROL =
  MOS_6522_ACR_BIT.timer1FreeRunning | (MOS_6522_SHIFT_MODE.outputFreeRunningTimer2 << 2);
const TIMER_REFERENCE_PERIPHERAL_CONTROL = 0xee;

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

function referenceUrl(file: string): string {
  return (
    `https://sourceforge.net/p/vice-emu/code/${VICE_TEST_REVISION}/tree/` +
    `${REFERENCE_DIRECTORY}/${file}?format=raw`
  );
}

async function loadReference(reference: Mos6522ReferenceFile): Promise<Uint8Array> {
  const cachePath = resolve(`output/reference/${reference.file}`);
  try {
    const cached = new Uint8Array(await readFile(cachePath));
    if (sha256(cached) === reference.sha256) return cached;
  } catch {
    // 首次运行时没有本地缓存属于正常路径，下载后仍必须通过固定哈希校验。
  }

  const response = await fetch(referenceUrl(reference.file));
  if (!response.ok) {
    throw new Error(
      `Unable to download VICE reference ${reference.file}: HTTP ${response.status}.`,
    );
  }
  const downloaded = new Uint8Array(await response.arrayBuffer());
  const actualHash = sha256(downloaded);
  if (actualHash !== reference.sha256) {
    throw new Error(
      `VICE reference ${reference.file} SHA-256 mismatch: expected ${reference.sha256}, received ${actualHash}.`,
    );
  }
  await mkdir(dirname(cachePath), { recursive: true });
  await writeFile(cachePath, downloaded);
  return downloaded;
}

function splitReferenceSamples(reference: Mos6522ReferenceFile, bytes: Uint8Array): Uint8Array[] {
  const requiredDataLength = REFERENCE_HEADER_SIZE + reference.caseCount * REFERENCE_SAMPLE_COUNT;
  if (bytes.length !== reference.byteLength || bytes.length < requiredDataLength) {
    throw new RangeError(
      `${reference.file} must contain ${reference.byteLength} bytes with at least ${requiredDataLength} bytes of sampled pages; received ${bytes.length}.`,
    );
  }
  const loadAddress = (bytes[0] ?? 0) | ((bytes[1] ?? 0) << 8);
  if (loadAddress !== reference.loadAddress) {
    throw new Error(
      `${reference.file} has unexpected load address $${loadAddress.toString(16).padStart(4, '0')}.`,
    );
  }

  return Array.from({ length: reference.caseCount }, (_unused, caseIndex) => {
    const start = REFERENCE_HEADER_SIZE + caseIndex * REFERENCE_SAMPLE_COUNT;
    return bytes.slice(start, start + REFERENCE_SAMPLE_COUNT);
  });
}

function appendAbsoluteInstruction(program: number[], opcode: number, address: number): void {
  program.push(opcode, address & 0xff, (address >>> 8) & 0xff);
}

function appendImmediateInstruction(program: number[], opcode: number, value: number): void {
  program.push(opcode, value & 0xff);
}

function viaAddress(register: number): number {
  return DRIVE_1541_MEMORY_LAYOUT.iecVia.start + register;
}

interface ReplayProgram {
  readonly bytes: Uint8Array;
  readonly stopAddress: number;
}

interface RustReplayResult {
  readonly samples: readonly number[];
}

function appendLoopBack(program: number[], loopAddress: number): void {
  program.push(OPCODE.incrementX, OPCODE.branchNotEqual);
  const branchNextAddress = TEST_PROGRAM_ADDRESS + program.length + 1;
  const branchDisplacement = loopAddress - branchNextAddress;
  if (branchDisplacement < -128 || branchDisplacement > 127) {
    throw new RangeError(`VIA replay branch displacement ${branchDisplacement} is invalid.`);
  }
  program.push(branchDisplacement & 0xff);
}

function finishReplayProgram(program: number[]): ReplayProgram {
  return {
    bytes: Uint8Array.from(program),
    stopAddress: TEST_PROGRAM_ADDRESS + program.length,
  };
}

function buildPb7ReplayProgram(reference: Mos6522Pb7Reference, caseIndex: number): ReplayProgram {
  const program: number[] = [];
  const controlValue = PB7_CONTROL_VALUES[Math.floor(caseIndex / 2)];
  if (controlValue === undefined) throw new RangeError(`Invalid VIA reference case ${caseIndex}.`);
  const timerRegister =
    caseIndex % 2 === 0 ? MOS_6522_REGISTER.timer1CounterLow : MOS_6522_REGISTER.timer1CounterHigh;

  appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, reference.dataDirectionB);
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.dataDirectionB),
  );
  appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, reference.portB);
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.portB),
  );

  const appendTimerWrite = (): void => {
    appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, 0x01);
    appendAbsoluteInstruction(program, OPCODE.storeAccumulatorAbsolute, viaAddress(timerRegister));
  };
  const appendControlWrite = (): void => {
    appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, controlValue);
    appendAbsoluteInstruction(
      program,
      OPCODE.storeAccumulatorAbsolute,
      viaAddress(MOS_6522_REGISTER.auxiliaryControl),
    );
  };

  if (reference.controlBeforeTimer) {
    appendControlWrite();
    appendTimerWrite();
  } else {
    appendTimerWrite();
    appendControlWrite();
  }

  appendImmediateInstruction(program, OPCODE.loadXImmediate, 0x00);
  const loopAddress = TEST_PROGRAM_ADDRESS + program.length;
  appendAbsoluteInstruction(
    program,
    OPCODE.loadAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.portB),
  );
  appendAbsoluteInstruction(program, OPCODE.storeAccumulatorAbsoluteX, SAMPLE_BUFFER_ADDRESS);
  appendLoopBack(program, loopAddress);

  return finishReplayProgram(program);
}

function buildTimerReplayProgram(
  reference: Mos6522TimerReference,
  caseIndex: number,
): ReplayProgram {
  const timerValues = TIMER_REFERENCE_CASES[caseIndex];
  if (timerValues === undefined) throw new RangeError(`Invalid VIA timer case ${caseIndex}.`);
  const timer1LowRegister = reference.startTimer1
    ? MOS_6522_REGISTER.timer1CounterLow
    : MOS_6522_REGISTER.timer1LatchLow;
  const timer1HighRegister = reference.startTimer1
    ? MOS_6522_REGISTER.timer1CounterHigh
    : MOS_6522_REGISTER.timer1LatchHigh;
  const program: number[] = [];

  appendImmediateInstruction(
    program,
    OPCODE.loadAccumulatorImmediate,
    TIMER_REFERENCE_PERIPHERAL_CONTROL,
  );
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.peripheralControl),
  );
  appendImmediateInstruction(
    program,
    OPCODE.loadAccumulatorImmediate,
    TIMER_REFERENCE_AUXILIARY_CONTROL,
  );
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.auxiliaryControl),
  );

  appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, timerValues.timer1);
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(timer1LowRegister),
  );
  appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, timerValues.timer2);
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.timer2CounterLow),
  );
  appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, timerValues.timer1 >>> 8);
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(timer1HighRegister),
  );
  appendImmediateInstruction(program, OPCODE.loadAccumulatorImmediate, timerValues.timer2 >>> 8);
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.timer2CounterHigh),
  );

  appendImmediateInstruction(program, OPCODE.loadXImmediate, 0x00);
  const loopAddress = TEST_PROGRAM_ADDRESS + program.length;
  appendAbsoluteInstruction(
    program,
    OPCODE.loadAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.interruptFlags),
  );
  appendAbsoluteInstruction(
    program,
    OPCODE.storeAccumulatorAbsolute,
    viaAddress(MOS_6522_REGISTER.interruptFlags),
  );
  appendAbsoluteInstruction(program, OPCODE.storeAccumulatorAbsoluteX, SAMPLE_BUFFER_ADDRESS);
  appendLoopBack(program, loopAddress);

  return finishReplayProgram(program);
}

function createReplayRom(program: Uint8Array): Uint8Array {
  const rom = new Uint8Array(DRIVE_1541_MEMORY_LAYOUT.rom.imageSize);
  rom.set(program, TEST_PROGRAM_ADDRESS - DRIVE_1541_MEMORY_LAYOUT.rom.primaryStart);
  const resetVectorOffset = 0x3ffc;
  rom[resetVectorOffset] = TEST_PROGRAM_ADDRESS & 0xff;
  rom[resetVectorOffset + 1] = TEST_PROGRAM_ADDRESS >>> 8;
  return rom;
}

function initializeReferenceVia(via: Drive1541IecVia, machine: Drive1541Machine): void {
  via.write(MOS_6522_REGISTER.auxiliaryControl, MOS_6522_ACR_BIT.timer2CountPortB6);
  via.write(MOS_6522_REGISTER.peripheralControl, 0x00);
  via.write(MOS_6522_REGISTER.shiftRegister, 0xa5);
  via.write(MOS_6522_REGISTER.interruptEnable, MOS_6522_INTERRUPT_BIT.sourceMask);
  via.write(MOS_6522_REGISTER.interruptFlags, MOS_6522_INTERRUPT_BIT.sourceMask);
  via.write(MOS_6522_REGISTER.timer1CounterLow, 0x00);
  via.write(MOS_6522_REGISTER.timer1CounterHigh, 0x00);
  via.write(MOS_6522_REGISTER.timer1LatchLow, 0x00);
  via.write(MOS_6522_REGISTER.timer1LatchHigh, 0x00);
  via.write(MOS_6522_REGISTER.timer2CounterLow, 0x00);
  via.write(MOS_6522_REGISTER.timer2CounterHigh, 0x00);

  // 真机框架进入被测片段前已经让零值 T1 完成首次超时；这里仅复现该确定状态。
  machine.advanceHardware(2);
  machine.resetTiming();
}

function replayReferenceCase(
  reference: Mos6522ReferenceFile,
  caseIndex: number,
  replay: ReplayProgram,
): Uint8Array {
  const iecBus = new IecBus();
  const referenceBusPort = iecBus.attach('MOS 6522 reference bus state');
  // VICE viavarious 的 C64 接收框架开始采样时会把 IEC DATA/CLOCK 拉低并释放 ATN。
  // 真机参考页因此在 VIA1 输入位上呈现 PB0=1、PB2=1、PB7=0；显式复现该板级状态，
  // 避免把 IEC 外部输入错误地当成 MOS 6522 芯片自身的固定上拉。
  referenceBusPort.setPulledLowLines([IEC_LINE.clock, IEC_LINE.data]);
  const mechanism = new Drive1541Mechanism();
  const iecVia = new Drive1541IecVia({ deviceNumber: 8, iecBus });
  const diskVia = new Drive1541DiskVia({ deviceNumber: 8, mechanism });
  const memory = new Drive1541Memory(createReplayRom(replay.bytes), { diskVia, iecVia });
  const cpu = new Cpu6502(memory);
  const machine = new Drive1541Machine(cpu, memory, mechanism);

  try {
    initializeReferenceVia(iecVia, machine);
    let executedInstructions = 0;
    while (cpu.pc !== replay.stopAddress && executedInstructions < MAXIMUM_REPLAY_INSTRUCTIONS) {
      machine.executeInstruction();
      executedInstructions += 1;
    }
    if (cpu.pc !== replay.stopAddress) {
      throw new Error(
        `${reference.file} case ${caseIndex} did not stop within ${MAXIMUM_REPLAY_INSTRUCTIONS} instructions.`,
      );
    }
    return memory.ram.slice(SAMPLE_BUFFER_ADDRESS, SAMPLE_BUFFER_ADDRESS + REFERENCE_SAMPLE_COUNT);
  } finally {
    machine.disconnect();
    diskVia.disconnect();
    iecVia.disconnect();
    referenceBusPort.disconnect();
  }
}

async function replayRustReferenceCases(
  replays: readonly ReplayProgram[],
): Promise<readonly Uint8Array[]> {
  const results = await runRustJsonTrace<readonly RustReplayResult[]>({
    example: 'vice_1541_via',
    input: replays.map((replay) => ({
      program: [...replay.bytes],
      stopAddress: replay.stopAddress,
    })),
    label: 'Rust 1541 VIA replay',
    maximumOutputBytes: 4 * 1024 * 1024,
  });
  if (results.length !== replays.length) {
    throw new Error(
      `Rust 1541 VIA replay returned ${results.length} cases for ${replays.length} requests.`,
    );
  }
  return results.map((result, caseIndex) => {
    if (result.samples.length !== REFERENCE_SAMPLE_COUNT) {
      throw new Error(
        `Rust 1541 VIA replay case ${caseIndex} returned ${result.samples.length} samples.`,
      );
    }
    return Uint8Array.from(result.samples);
  });
}

function formatDifference(expected: Uint8Array, actual: Uint8Array): string {
  const differences: string[] = [];
  for (let index = 0; index < expected.length && differences.length < 16; index += 1) {
    const expectedValue = expected[index];
    const actualValue = actual[index];
    if (expectedValue !== actualValue) {
      differences.push(
        `${index}:$${(actualValue ?? 0).toString(16).padStart(2, '0')}!=` +
          `$${(expectedValue ?? 0).toString(16).padStart(2, '0')}`,
      );
    }
  }
  return differences.join(' ');
}

interface LoadedReference<Reference extends Mos6522ReferenceFile> {
  readonly cases: readonly Uint8Array[];
  readonly reference: Reference;
}

async function loadFixtures<Reference extends Mos6522ReferenceFile>(
  references: readonly Reference[],
): Promise<LoadedReference<Reference>[]> {
  return Promise.all(
    references.map(async (reference) => ({
      cases: splitReferenceSamples(reference, await loadReference(reference)),
      reference,
    })),
  );
}

async function verifyFixtures<Reference extends Mos6522ReferenceFile>(
  fixtures: readonly LoadedReference<Reference>[],
  buildReplay: (reference: Reference, caseIndex: number) => ReplayProgram,
): Promise<number> {
  const replays = fixtures.flatMap((fixture) =>
    fixture.cases.map((_expected, caseIndex) => buildReplay(fixture.reference, caseIndex)),
  );
  const rustResults = await replayRustReferenceCases(replays);
  let verifiedCases = 0;
  let replayIndex = 0;
  for (const fixture of fixtures) {
    for (let caseIndex = 0; caseIndex < fixture.cases.length; caseIndex += 1) {
      const expected = fixture.cases[caseIndex];
      if (expected === undefined) throw new RangeError(`Missing reference case ${caseIndex}.`);
      const replay = replays[replayIndex];
      const rustActual = rustResults[replayIndex];
      if (!replay || !rustActual) throw new RangeError(`Missing VIA replay ${replayIndex}.`);
      const actual = replayReferenceCase(fixture.reference, caseIndex, replay);
      const difference = formatDifference(expected, actual);
      if (difference.length > 0) {
        throw new Error(
          `VICE ${fixture.reference.file} case ${caseIndex} mismatch: ${difference}.`,
        );
      }
      const rustDifference = formatDifference(expected, rustActual);
      if (rustDifference.length > 0) {
        throw new Error(
          `Rust/VICE ${fixture.reference.file} case ${caseIndex} mismatch: ${rustDifference}.`,
        );
      }
      verifiedCases += 1;
      replayIndex += 1;
    }
  }
  return verifiedCases;
}

async function main(): Promise<void> {
  const [pb7Fixtures, timerFixtures] = await Promise.all([
    loadFixtures(PB7_REFERENCES),
    loadFixtures(TIMER_REFERENCES),
  ]);
  const pb7Cases = await verifyFixtures(pb7Fixtures, buildPb7ReplayProgram);
  const timerCases = await verifyFixtures(timerFixtures, buildTimerReplayProgram);

  console.log(
    `PASS TypeScript/Rust MOS 6522 reference: ${pb7Cases} PB7 pages and ${timerCases} timer/IFR pages from real 1541 hardware at VICE revision ${VICE_TEST_REVISION}.`,
  );
}

await main();
