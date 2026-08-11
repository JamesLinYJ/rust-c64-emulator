// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - Node WebAssembly ABI 安全验证
//
//   文件:       verifyWasm.ts
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import path from 'node:path';

interface C64VmInstance {
  awaiting_auto_calibration(): boolean;
  cartridge_attached(): boolean;
  cartridge_kind(): number;
  current_slot(): number;
  effective_slots_per_system_cycle(): number;
  easyflash_dirty(): boolean;
  eject_cartridge(): void;
  eject_tap(): Uint8Array;
  elapsed_system_cycles_high(): number;
  elapsed_system_cycles_low(): number;
  export_easyflash_high(): Uint8Array;
  export_easyflash_low(): Uint8Array;
  free(): void;
  load_state(bytes: Uint8Array): void;
  lock_auto_turbo(resolvedSlotsPerSystemCycle: number): void;
  memory_generation_low(): number;
  insert_blank_tap(videoStandard: number): void;
  insert_crt(bytes: Uint8Array, easyFlashJumperInstalled: boolean): void;
  insert_tap(bytes: Uint8Array, legacyV0OverflowPulseCycles: number): void;
  read_base_ram(address: number): number;
  request_auto_turbo(maximumSlotsPerSystemCycle: number): void;
  reset(): void;
  run_cpu_slots(slots: number): number;
  save_state(): Uint8Array;
  set_manual_turbo(slotsPerSystemCycle: number): void;
  tape_mounted(): boolean;
  tape_motor_active(): boolean;
  tape_play(): void;
  tape_pulse_count(): number;
  tape_pulse_index(): number;
  tape_record(): void;
  tape_rewind(): void;
  tape_seek_pulse(pulseIndex: number): void;
  tape_sense_switch_closed(): boolean;
  tape_stop(): void;
  tape_transport(): number;
  tape_writable(): boolean;
  write_base_ram(address: number, value: number): void;
}

type C64VmConstructor = new (profile: number, videoStandard: number) => C64VmInstance;

interface C64WasmModule {
  readonly C64Vm: C64VmConstructor;
}

const requireFromHere = createRequire(import.meta.url);
const generatedModulePath = path.resolve('generated/wasm/node/c64_vm.js');
const loadedModule: unknown = requireFromHere(generatedModulePath);
assertC64WasmModule(loadedModule);

assert.throws(() => new loadedModule.C64Vm(255, 0), /profile/u);
assert.throws(() => new loadedModule.C64Vm(0, 255), /视频制式/u);

const source = new loadedModule.C64Vm(0, 0);
const restored = new loadedModule.C64Vm(0, 0);
try {
  assert.equal(source.effective_slots_per_system_cycle(), 1);
  source.set_manual_turbo(20);
  assert.equal(source.run_cpu_slots(40), 2);
  assert.equal(source.elapsed_system_cycles_low(), 2);
  assert.equal(source.elapsed_system_cycles_high(), 0);
  assert.equal(source.current_slot(), 0);

  const generationBeforeHostWrite = source.memory_generation_low();
  source.write_base_ram(0xc000, 0x5a);
  assert.equal(source.read_base_ram(0xc000), 0x5a);
  assert.equal((source.memory_generation_low() - generationBeforeHostWrite) >>> 0, 1);

  assert.throws(() => source.insert_crt(Uint8Array.of(0), false), /CRT image/u);
  assert.equal(source.cartridge_attached(), false, '失败的 CRT 插入必须保持空插槽');
  const standardCrt = createStandard8kCrt(0x81);
  source.insert_crt(standardCrt, false);
  assert.equal(source.cartridge_attached(), true);
  assert.equal(source.cartridge_kind(), 0);
  assert.throws(() => source.insert_crt(standardCrt, false), /already attached/u);
  source.eject_cartridge();
  assert.equal(source.cartridge_attached(), false);
  assert.equal(source.cartridge_kind(), 0xffff);
  assert.throws(() => source.eject_cartridge(), /no cartridge/u);
  assert.throws(() => source.export_easyflash_low(), /not an EasyFlash/u);

  source.insert_crt(createEmptyEasyFlashCrt(), false);
  assert.equal(source.cartridge_kind(), 32);
  assert.equal(source.easyflash_dirty(), false);
  const flashLow = source.export_easyflash_low();
  const flashHigh = source.export_easyflash_high();
  assert.equal(flashLow.length, 0x80000);
  assert.equal(flashHigh.length, 0x80000);
  assert.equal(flashLow[0], 0xff);
  assert.equal(flashHigh[flashHigh.length - 1], 0xff);
  source.eject_cartridge();

  assert.throws(() => source.insert_tap(Uint8Array.of(0), 0), /20-byte header/u);
  assert.equal(source.tape_mounted(), false, '失败的 TAP 插入必须保持空仓');
  const readOnlyTap = createTap(Uint8Array.of(2, 3));
  source.insert_tap(readOnlyTap, 0);
  assert.equal(source.tape_mounted(), true);
  assert.equal(source.tape_writable(), false);
  assert.equal(source.tape_pulse_count(), 2);
  source.tape_play();
  assert.equal(source.tape_transport(), 1);
  assert.equal(source.tape_sense_switch_closed(), true);
  assert.throws(() => source.eject_tap(), /stop/u);
  source.tape_stop();
  source.tape_seek_pulse(1);
  assert.equal(source.tape_pulse_index(), 1);
  source.tape_rewind();
  assert.equal(source.tape_pulse_index(), 0);
  assert.deepEqual(source.eject_tap(), readOnlyTap);
  assert.equal(source.tape_mounted(), false);

  assert.throws(() => source.insert_blank_tap(255), /video standard/u);
  source.insert_blank_tap(0);
  assert.equal(source.tape_writable(), true);
  source.tape_record();
  assert.equal(source.tape_transport(), 2);
  source.tape_stop();
  const blankTap = source.eject_tap();
  assert.equal(blankTap.length, 20);
  assert.equal(blankTap[12], 1);
  assert.equal(blankTap[14], 0);
  const state = source.save_state();

  restored.write_base_ram(0xc000, 0x33);
  const corruptState = state.slice();
  corruptState[0] ^= 0xff;
  assert.throws(() => restored.load_state(corruptState), /magic/u);
  assert.equal(restored.read_base_ram(0xc000), 0x33, '失败的载入必须保持原状态');

  restored.load_state(state);
  assert.equal(restored.read_base_ram(0xc000), 0x5a);
  assert.equal(restored.effective_slots_per_system_cycle(), 20);
  assert.equal(restored.elapsed_system_cycles_low(), 2);

  restored.reset();
  assert.equal(restored.effective_slots_per_system_cycle(), 1);
  restored.request_auto_turbo(16);
  assert.equal(restored.awaiting_auto_calibration(), true);
  assert.throws(() => restored.lock_auto_turbo(20), /超过请求上限/u);
  restored.lock_auto_turbo(8);
  assert.equal(restored.awaiting_auto_calibration(), false);
  assert.equal(restored.effective_slots_per_system_cycle(), 8);
} finally {
  source.free();
  restored.free();
}

console.log(
  'Wasm ABI 安全验证通过：边界输入、状态原子性、Cartridge/EasyFlash、Tape、Turbo 与 64 KiB RAM 均符合契约。',
);

function assertC64WasmModule(value: unknown): asserts value is C64WasmModule {
  if (
    typeof value !== 'object' ||
    value === null ||
    !('C64Vm' in value) ||
    typeof value.C64Vm !== 'function'
  ) {
    throw new TypeError('生成的 Node Wasm 模块没有导出 C64Vm 构造器。');
  }
}

function createTap(data: Uint8Array): Uint8Array {
  const bytes = new Uint8Array(20 + data.length);
  bytes.set(Uint8Array.from('C64-TAPE-RAW', (character) => character.charCodeAt(0)));
  bytes[12] = 1;
  bytes[13] = 0;
  bytes[14] = 0;
  new DataView(bytes.buffer).setUint32(16, data.length, true);
  bytes.set(data, 20);
  return bytes;
}

function createStandard8kCrt(fill: number): Uint8Array {
  const headerSize = 0x40;
  const bankSize = 0x2000;
  const chipHeaderSize = 0x10;
  const bytes = new Uint8Array(headerSize + chipHeaderSize + bankSize);
  writeAscii(bytes, 0, 'C64 CARTRIDGE   ');
  writeUint32Be(bytes, 0x10, headerSize);
  writeUint16Be(bytes, 0x14, 0x0100);
  bytes[0x18] = 0;
  bytes[0x19] = 1;
  writeAscii(bytes, headerSize, 'CHIP');
  writeUint32Be(bytes, headerSize + 4, chipHeaderSize + bankSize);
  writeUint16Be(bytes, headerSize + 0x0c, 0x8000);
  writeUint16Be(bytes, headerSize + 0x0e, bankSize);
  bytes.fill(fill, headerSize + chipHeaderSize);
  return bytes;
}

function createEmptyEasyFlashCrt(): Uint8Array {
  const bytes = new Uint8Array(0x40);
  writeAscii(bytes, 0, 'C64 CARTRIDGE   ');
  writeUint32Be(bytes, 0x10, bytes.length);
  writeUint16Be(bytes, 0x14, 0x0100);
  writeUint16Be(bytes, 0x16, 32);
  bytes[0x18] = 1;
  bytes[0x19] = 0;
  return bytes;
}

function writeAscii(bytes: Uint8Array, offset: number, value: string): void {
  for (let index = 0; index < value.length; index += 1) {
    bytes[offset + index] = value.charCodeAt(index);
  }
}

function writeUint16Be(bytes: Uint8Array, offset: number, value: number): void {
  bytes[offset] = value >>> 8;
  bytes[offset + 1] = value;
}

function writeUint32Be(bytes: Uint8Array, offset: number, value: number): void {
  bytes[offset] = value >>> 24;
  bytes[offset + 1] = value >>> 16;
  bytes[offset + 2] = value >>> 8;
  bytes[offset + 3] = value;
}
