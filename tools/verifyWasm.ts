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
  audio_sample_count(): number;
  audio_sample_rate_hz(): number;
  audio_samples_ptr(): number;
  attach_drive1541(deviceNumber: number, rom: Uint8Array): void;
  attach_reu(sizeKib: number): void;
  attach_reu_image(sizeKib: number, image: Uint8Array): void;
  awaiting_auto_calibration(): boolean;
  basic_ready(): boolean;
  cartridge_attached(): boolean;
  cartridge_kind(): number;
  current_slot(): number;
  detach_drive1541(): void;
  diagnostics_cpu_bus_transactions_high(): number;
  diagnostics_cpu_bus_transactions_low(): number;
  diagnostics_elapsed_system_cycles_high(): number;
  diagnostics_elapsed_system_cycles_low(): number;
  diagnostics_execution_mode_changes_high(): number;
  diagnostics_execution_mode_changes_low(): number;
  diagnostics_held_cpu_read_system_cycles_high(): number;
  diagnostics_held_cpu_read_system_cycles_low(): number;
  diagnostics_retired_cpu_slots_high(): number;
  diagnostics_retired_cpu_slots_low(): number;
  diagnostics_reu_dma_bus_cycles_high(): number;
  diagnostics_reu_dma_bus_cycles_low(): number;
  diagnostics_reu_dma_system_cycles_high(): number;
  diagnostics_reu_dma_system_cycles_low(): number;
  diagnostics_reu_dma_vic_stall_cycles_high(): number;
  diagnostics_reu_dma_vic_stall_cycles_low(): number;
  diagnostics_state_loads_high(): number;
  diagnostics_state_loads_low(): number;
  drive1541_attached(): boolean;
  drive1541_disk_mounted(): boolean;
  drive1541_disk_write_protected(): boolean;
  effective_slots_per_system_cycle(): number;
  easyflash_dirty(): boolean;
  eject_cartridge(): void;
  eject_drive1541_disk(): Uint8Array;
  detach_reu(): Uint8Array;
  eject_tap(): Uint8Array;
  elapsed_system_cycles_high(): number;
  elapsed_system_cycles_low(): number;
  export_easyflash_high(): Uint8Array;
  export_easyflash_low(): Uint8Array;
  export_reu_ram(): Uint8Array;
  frame_generation_low(): number;
  frame_height(): number;
  frame_pixels_len(): number;
  frame_pixels_ptr(): number;
  frame_width(): number;
  free(): void;
  load_state(bytes: Uint8Array): void;
  lock_auto_turbo(resolvedSlotsPerSystemCycle: number): void;
  memory_generation_low(): number;
  mount_drive1541_d64(bytes: Uint8Array, writeProtected: boolean): void;
  mount_drive1541_g64(bytes: Uint8Array, writeProtected: boolean): void;
  processor_clock_hz(): number;
  insert_blank_tap(videoStandard: number): void;
  install_basic_prg(bytes: Uint8Array): void;
  insert_crt(bytes: Uint8Array, easyFlashJumperInstalled: boolean): void;
  insert_tap(bytes: Uint8Array, legacyV0OverflowPulseCycles: number): void;
  read_base_ram(address: number): number;
  request_auto_turbo(maximumSlotsPerSystemCycle: number): void;
  reu_attached(): boolean;
  reu_dma_active(): boolean;
  reu_size_kib(): number;
  reset(): void;
  run_cpu_slots(slots: number): number;
  run_until_next_frame(): number;
  save_state(): Uint8Array;
  set_manual_turbo(slotsPerSystemCycle: number): void;
  set_host_input(
    pressedRowsByColumn: Uint8Array,
    shiftLockPressed: boolean,
    joystickPort1Grounded: number,
    joystickPort2Grounded: number,
    restoreKeyPressed: boolean,
  ): void;
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
  video_frame_cycles(): number;
  video_standard(): number;
  write_base_ram(address: number, value: number): void;
}

interface C64VmConstructor {
  new (profile: number, videoStandard: number): C64VmInstance;
  withFirmware(
    profile: number,
    videoStandard: number,
    basic: Uint8Array,
    character: Uint8Array,
    kernal: Uint8Array,
  ): C64VmInstance;
}

interface C64WasmModule {
  readonly C64Vm: C64VmConstructor;
}

const requireFromHere = createRequire(import.meta.url);
const generatedModulePath = path.resolve('generated/wasm/node/c64_vm.js');
const loadedModule: unknown = requireFromHere(generatedModulePath);
assertC64WasmModule(loadedModule);

assert.throws(() => new loadedModule.C64Vm(255, 0), /profile/u);
assert.throws(() => new loadedModule.C64Vm(0, 255), /视频制式/u);
assert.throws(
  () =>
    loadedModule.C64Vm.withFirmware(
      0,
      0,
      new Uint8Array(0x1fff),
      new Uint8Array(0x1000),
      new Uint8Array(0x2000),
    ),
  /BASIC ROM must contain 8192 bytes/u,
);
const production = loadedModule.C64Vm.withFirmware(
  0,
  0,
  new Uint8Array(0x2000),
  new Uint8Array(0x1000),
  new Uint8Array(0x2000),
);
production.free();

const source = new loadedModule.C64Vm(0, 0);
const restored = new loadedModule.C64Vm(0, 0);
const ntsc = new loadedModule.C64Vm(0, 1);
try {
  assert.equal(source.effective_slots_per_system_cycle(), 1);
  source.set_manual_turbo(20);
  assert.equal(source.run_cpu_slots(40), 2);
  assert.equal(source.elapsed_system_cycles_low(), 2);
  assert.equal(source.elapsed_system_cycles_high(), 0);
  assert.equal(source.current_slot(), 0);
  assert.equal(source.diagnostics_retired_cpu_slots_low(), 40);
  assert.equal(source.diagnostics_retired_cpu_slots_high(), 0);
  assert.equal(source.diagnostics_elapsed_system_cycles_low(), 2);
  assert.equal(source.diagnostics_elapsed_system_cycles_high(), 0);
  assert.ok(source.diagnostics_cpu_bus_transactions_low() > 0);
  assert.equal(source.diagnostics_cpu_bus_transactions_high(), 0);
  assert.equal(source.diagnostics_held_cpu_read_system_cycles_high(), 0);
  assert.equal(source.diagnostics_reu_dma_system_cycles_high(), 0);
  assert.equal(source.diagnostics_reu_dma_vic_stall_cycles_high(), 0);
  assert.equal(source.diagnostics_reu_dma_bus_cycles_high(), 0);
  assert.ok(source.diagnostics_execution_mode_changes_low() > 0);
  assert.equal(source.diagnostics_execution_mode_changes_high(), 0);
  assert.equal(source.diagnostics_state_loads_low(), 0);
  assert.equal(source.diagnostics_state_loads_high(), 0);

  assert.throws(
    () => source.set_host_input(new Uint8Array(7), false, 0, 0, false),
    /exactly 8 columns/u,
  );
  assert.throws(
    () => source.set_host_input(new Uint8Array(8), false, 0x20, 0, false),
    /five-bit digital mask/u,
  );
  source.set_host_input(Uint8Array.of(0, 0x04, 0, 0, 0, 0, 0, 0), false, 0x10, 0x01, true);

  const frameGenerationBefore = source.frame_generation_low();
  assert.ok(source.run_until_next_frame() > 0);
  assert.notEqual(source.frame_generation_low(), frameGenerationBefore);
  assert.equal(source.frame_width(), 403);
  assert.equal(source.frame_height(), 284);
  assert.equal(source.frame_pixels_len(), 403 * 284);
  assert.equal(source.video_standard(), 0);
  assert.equal(source.processor_clock_hz(), 985_248);
  assert.equal(source.video_frame_cycles(), 63 * 312);
  assert.equal(source.audio_sample_rate_hz(), 44_100);
  assert.ok(source.audio_sample_count() > 0);
  assert.ok(source.frame_pixels_ptr() > 0);
  assert.ok(source.audio_samples_ptr() > 0);

  const generationBeforeHostWrite = source.memory_generation_low();
  source.write_base_ram(0xc000, 0x5a);
  assert.equal(source.read_base_ram(0xc000), 0x5a);
  assert.equal((source.memory_generation_low() - generationBeforeHostWrite) >>> 0, 1);

  source.write_base_ram(0x002b, 0x01);
  source.write_base_ram(0x002c, 0x08);
  source.write_base_ram(0x0289, 10);
  assert.throws(() => source.install_basic_prg(Uint8Array.of(0x00, 0x20, 0xea)), /\$0801/u);
  source.install_basic_prg(Uint8Array.of(0x01, 0x08, 0x0b, 0x08, 0x00, 0x00));
  assert.equal(source.read_base_ram(0x0801), 0x0b);
  assert.equal(source.read_base_ram(0x00c6), 4);

  for (const [offset, value] of Uint8Array.of(0x12, 0x05, 0x01, 0x04, 0x19, 0x2e).entries()) {
    source.write_base_ram(0x0400 + offset, value);
  }
  assert.equal(source.basic_ready(), true);

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

  assert.throws(() => source.attach_reu(64), /128, 256 or 512/u);
  source.attach_reu(512);
  assert.equal(source.reu_attached(), true);
  assert.equal(source.reu_size_kib(), 512);
  assert.equal(source.reu_dma_active(), false);
  const blankReu = source.export_reu_ram();
  assert.equal(blankReu.length, 512 * 1024);
  assert.equal(blankReu[0], 0xff);
  assert.throws(() => source.insert_crt(standardCrt, false), /already attached/u);
  assert.deepEqual(source.detach_reu(), blankReu);
  assert.equal(source.reu_attached(), false);
  assert.throws(() => source.detach_reu(), /no REU/u);

  const initializedReu = new Uint8Array(256 * 1024);
  initializedReu[0] = 0x12;
  initializedReu[initializedReu.length - 1] = 0x34;
  assert.throws(
    () => source.attach_reu_image(256, initializedReu.subarray(1)),
    /exactly 262144 bytes/u,
  );
  source.attach_reu_image(256, initializedReu);
  assert.deepEqual(source.detach_reu(), initializedReu);

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

  const driveRom = new Uint8Array(0x4000);
  const d64 = new Uint8Array(174_848);
  const g64 = createEmptyG64();
  assert.equal(source.drive1541_attached(), false);
  assert.throws(() => source.attach_drive1541(8, driveRom.subarray(1)), /16384/u);
  source.attach_drive1541(8, driveRom);
  assert.equal(source.drive1541_attached(), true);
  assert.equal(source.drive1541_disk_mounted(), false);
  assert.throws(() => source.mount_drive1541_d64(Uint8Array.of(0), false), /D64/u);
  source.mount_drive1541_d64(d64, true);
  assert.equal(source.drive1541_disk_mounted(), true);
  assert.equal(source.drive1541_disk_write_protected(), true);
  const exportedD64 = source.eject_drive1541_disk();
  assert.equal(exportedD64[0], 0);
  assert.deepEqual(exportedD64.subarray(1), d64);
  source.mount_drive1541_g64(g64, false);
  assert.equal(source.drive1541_disk_write_protected(), false);
  const exportedG64 = source.eject_drive1541_disk();
  assert.equal(exportedG64[0], 1);
  assert.deepEqual(exportedG64.subarray(1), g64);
  source.mount_drive1541_d64(d64, false);

  source.attach_reu_image(256, initializedReu);
  source.insert_tap(readOnlyTap, 0);
  source.tape_play();
  source.run_cpu_slots(400);
  const savedElapsedSystemCycles = source.elapsed_system_cycles_low();
  const savedTapePulseIndex = source.tape_pulse_index();
  const state = source.save_state();
  assert.equal(new TextDecoder().decode(state.subarray(0, 8)), 'RC64VM02');
  assert.equal(
    new DataView(state.buffer, state.byteOffset, state.byteLength).getUint16(8, true),
    2,
  );

  restored.write_base_ram(0xc000, 0x33);
  const corruptState = state.slice();
  corruptState[0] ^= 0xff;
  assert.throws(() => restored.load_state(corruptState), /magic/u);
  assert.equal(restored.read_base_ram(0xc000), 0x33, '失败的载入必须保持原状态');

  restored.load_state(state);
  assert.equal(restored.read_base_ram(0xc000), 0x5a);
  assert.equal(restored.effective_slots_per_system_cycle(), 20);
  assert.equal(restored.elapsed_system_cycles_low(), savedElapsedSystemCycles);
  assert.equal(restored.reu_attached(), true);
  assert.deepEqual(restored.export_reu_ram(), initializedReu);
  assert.equal(restored.tape_mounted(), true);
  assert.equal(restored.tape_transport(), 1);
  assert.equal(restored.tape_pulse_index(), savedTapePulseIndex);
  assert.equal(restored.drive1541_attached(), true);
  assert.equal(restored.drive1541_disk_mounted(), true);
  assert.equal(restored.drive1541_disk_write_protected(), false);
  assert.equal(restored.diagnostics_state_loads_low(), 1);
  assert.equal(restored.diagnostics_state_loads_high(), 0);
  assert.deepEqual(restored.save_state(), state);
  const restoredD64 = restored.eject_drive1541_disk();
  assert.equal(restoredD64[0], 0);
  assert.deepEqual(restoredD64.subarray(1), d64);
  restored.detach_drive1541();
  assert.equal(restored.drive1541_attached(), false);

  restored.reset();
  assert.equal(restored.effective_slots_per_system_cycle(), 1);
  restored.request_auto_turbo(16);
  assert.equal(restored.awaiting_auto_calibration(), true);
  assert.throws(() => restored.lock_auto_turbo(20), /超过请求上限/u);
  restored.lock_auto_turbo(8);
  assert.equal(restored.awaiting_auto_calibration(), false);
  assert.equal(restored.effective_slots_per_system_cycle(), 8);

  assert.equal(ntsc.frame_width(), 403);
  assert.equal(ntsc.frame_height(), 247);
  assert.equal(ntsc.frame_pixels_len(), 403 * 247);
  assert.equal(ntsc.video_standard(), 1);
  assert.equal(ntsc.processor_clock_hz(), 1_022_727);
  assert.equal(ntsc.video_frame_cycles(), 65 * 263);
  assert.equal(ntsc.run_until_next_frame(), 65 * 263);
} finally {
  source.free();
  restored.free();
  ntsc.free();
}

console.log(
  'Wasm ABI 安全验证通过：PAL/NTSC、边界输入、完整状态原子性、Cartridge/EasyFlash、REU、Tape、1541、诊断、Turbo 与 64 KiB RAM 均符合契约。',
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

function createEmptyG64(): Uint8Array {
  const halfTrackCount = 84;
  const bytes = new Uint8Array(12 + halfTrackCount * 8);
  writeAscii(bytes, 0, 'GCR-1541');
  bytes[8] = 0;
  bytes[9] = halfTrackCount;
  bytes[10] = 16;
  bytes[11] = 0;
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
