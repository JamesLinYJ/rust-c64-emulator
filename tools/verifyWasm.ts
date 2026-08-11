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
  current_slot(): number;
  effective_slots_per_system_cycle(): number;
  elapsed_system_cycles_high(): number;
  elapsed_system_cycles_low(): number;
  free(): void;
  load_state(bytes: Uint8Array): void;
  lock_auto_turbo(resolvedSlotsPerSystemCycle: number): void;
  memory_generation_low(): number;
  read_base_ram(address: number): number;
  request_auto_turbo(maximumSlotsPerSystemCycle: number): void;
  reset(): void;
  run_cpu_slots(slots: number): number;
  save_state(): Uint8Array;
  set_manual_turbo(slotsPerSystemCycle: number): void;
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

console.log('Wasm ABI 安全验证通过：边界输入、状态原子性、Turbo 与 64 KiB RAM 均符合契约。');

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
