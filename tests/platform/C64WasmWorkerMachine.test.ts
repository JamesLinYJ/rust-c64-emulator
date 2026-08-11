// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - Worker 粗粒度机器命令根因测试
//
//   文件:       C64WasmWorkerMachine.test.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it, vi } from 'vitest';

import { C64WasmWorkerMachine, type C64WasmVm } from '../../src/platform/C64WasmWorkerMachine';

function createVm() {
  const attachDrive1541 = vi.fn();
  const ejectCartridge = vi.fn();
  const insertCrt = vi.fn();
  const loadState = vi.fn();
  const mountDrive1541G64 = vi.fn();
  const reset = vi.fn();
  const tapePlay = vi.fn();
  const tapeSeekPulse = vi.fn();
  const vm: C64WasmVm = {
    attach_drive1541: attachDrive1541,
    attach_reu: vi.fn(),
    attach_reu_image: vi.fn(),
    audio_sample_count: vi.fn(() => 0),
    audio_sample_rate_hz: vi.fn(() => 44_100),
    audio_samples_ptr: vi.fn(() => 0),
    awaiting_auto_calibration: vi.fn(() => false),
    basic_ready: vi.fn(() => true),
    cartridge_attached: vi.fn(() => true),
    cartridge_kind: vi.fn(() => 32),
    current_slot: vi.fn(() => 0),
    detach_drive1541: vi.fn(),
    detach_reu: vi.fn(() => Uint8Array.of(0x11, 0x22)),
    diagnostics_cpu_bus_transactions_high: vi.fn(() => 0),
    diagnostics_cpu_bus_transactions_low: vi.fn(() => 37),
    diagnostics_elapsed_system_cycles_high: vi.fn(() => 1),
    diagnostics_elapsed_system_cycles_low: vi.fn(() => 2),
    diagnostics_execution_mode_changes_high: vi.fn(() => 0),
    diagnostics_execution_mode_changes_low: vi.fn(() => 3),
    diagnostics_held_cpu_read_system_cycles_high: vi.fn(() => 0),
    diagnostics_held_cpu_read_system_cycles_low: vi.fn(() => 4),
    diagnostics_retired_cpu_slots_high: vi.fn(() => 5),
    diagnostics_retired_cpu_slots_low: vi.fn(() => 6),
    diagnostics_reu_dma_bus_cycles_high: vi.fn(() => 0),
    diagnostics_reu_dma_bus_cycles_low: vi.fn(() => 7),
    diagnostics_reu_dma_system_cycles_high: vi.fn(() => 0),
    diagnostics_reu_dma_system_cycles_low: vi.fn(() => 8),
    diagnostics_reu_dma_vic_stall_cycles_high: vi.fn(() => 0),
    diagnostics_reu_dma_vic_stall_cycles_low: vi.fn(() => 9),
    diagnostics_state_loads_high: vi.fn(() => 0),
    diagnostics_state_loads_low: vi.fn(() => 10),
    drive1541_attached: vi.fn(() => false),
    drive1541_disk_mounted: vi.fn(() => false),
    drive1541_disk_write_protected: vi.fn(() => true),
    easyflash_dirty: vi.fn(() => false),
    effective_slots_per_system_cycle: vi.fn(() => 1),
    eject_cartridge: ejectCartridge,
    eject_drive1541_disk: vi.fn(() => Uint8Array.of(1, 0x44)),
    eject_tap: vi.fn(() => Uint8Array.of(0x55)),
    export_easyflash_high: vi.fn(() => Uint8Array.of(0x66)),
    export_easyflash_low: vi.fn(() => Uint8Array.of(0x77)),
    export_reu_ram: vi.fn(() => Uint8Array.of(0x88)),
    frame_generation_low: vi.fn(() => 12),
    frame_height: vi.fn(() => 284),
    frame_pixels_len: vi.fn(() => 403 * 284),
    frame_pixels_ptr: vi.fn(() => 0),
    frame_width: vi.fn(() => 403),
    free: vi.fn(),
    insert_blank_tap: vi.fn(),
    insert_crt: insertCrt,
    insert_tap: vi.fn(),
    install_basic_prg: vi.fn(),
    load_state: loadState,
    memory_generation_low: vi.fn(() => 13),
    mount_drive1541_d64: vi.fn(),
    mount_drive1541_g64: mountDrive1541G64,
    program_counter: vi.fn(() => 0xe5cd),
    reset,
    reu_attached: vi.fn(() => true),
    reu_dma_active: vi.fn(() => false),
    reu_size_kib: vi.fn(() => 512),
    run_until_next_frame: vi.fn(() => 19_656),
    save_state: vi.fn(() => new Uint8Array(Uint8Array.of(0xaa, 0xbb, 0xcc).buffer, 1, 1)),
    set_host_input: vi.fn(),
    tape_motor_active: vi.fn(() => false),
    tape_mounted: vi.fn(() => true),
    tape_play: tapePlay,
    tape_pulse_count: vi.fn(() => 20),
    tape_pulse_index: vi.fn(() => 2),
    tape_record: vi.fn(),
    tape_rewind: vi.fn(),
    tape_seek_pulse: tapeSeekPulse,
    tape_sense_switch_closed: vi.fn(() => true),
    tape_stop: vi.fn(),
    tape_transport: vi.fn(() => 1),
    tape_writable: vi.fn(() => false),
  };
  return {
    attachDrive1541,
    ejectCartridge,
    insertCrt,
    loadState,
    mountDrive1541G64,
    reset,
    tapePlay,
    tapeSeekPulse,
    vm,
  };
}

describe('C64WasmWorkerMachine', () => {
  it('round-trips state through exact transferable buffers and reports counters losslessly', () => {
    const { loadState, vm } = createVm();
    const machine = new C64WasmWorkerMachine(vm);

    const saved = machine.execute({ kind: 'saveState' });
    expect(saved.result).toMatchObject({ kind: 'bytes', source: 'state' });
    if (saved.result.kind !== 'bytes') throw new Error('Expected state bytes.');
    expect(new Uint8Array(saved.result.bytes)).toEqual(Uint8Array.of(0xbb));
    expect(saved.transfer).toEqual([saved.result.bytes]);

    const state = Uint8Array.of(1, 2, 3).buffer;
    expect(machine.execute({ bytes: state, kind: 'loadState' }).result).toEqual({ kind: 'ack' });
    expect(loadState).toHaveBeenCalledWith(Uint8Array.of(1, 2, 3));

    const diagnostics = machine.execute({ kind: 'diagnostics' });
    expect(diagnostics.transfer).toEqual([]);
    if (diagnostics.result.kind !== 'diagnostics') throw new Error('Expected diagnostics.');
    expect(diagnostics.result.value.cpuBusTransactions).toEqual({ high: 0, low: 37 });
    expect(diagnostics.result.value.elapsedSystemCycles).toEqual({ high: 1, low: 2 });
    expect(diagnostics.result.value.frameNumber).toBe(12);
    expect(diagnostics.result.value.programCounter).toBe(0xe5cd);
    expect(diagnostics.result.value.retiredCpuSlots).toEqual({ high: 5, low: 6 });
    expect(diagnostics.result.value.stateLoads).toEqual({ high: 0, low: 10 });
  });

  it('keeps media changes coarse and applies reset only when the command requests it', () => {
    const { ejectCartridge, insertCrt, reset, tapePlay, tapeSeekPulse, vm } = createVm();
    const machine = new C64WasmWorkerMachine(vm);

    const crt = Uint8Array.of(0x43, 0x36, 0x34).buffer;
    expect(
      machine.execute({
        bytes: crt,
        easyFlashJumperInstalled: true,
        kind: 'insertCartridge',
        resetMachine: true,
      }).result,
    ).toEqual({ kind: 'ack' });
    expect(insertCrt).toHaveBeenCalledWith(Uint8Array.of(0x43, 0x36, 0x34), true);
    expect(reset).toHaveBeenCalledOnce();

    machine.execute({ kind: 'ejectCartridge', resetMachine: false });
    expect(ejectCartridge).toHaveBeenCalledOnce();
    expect(reset).toHaveBeenCalledOnce();

    const reu = machine.execute({ kind: 'detachReu' });
    expect(reu.result).toMatchObject({ kind: 'bytes', source: 'reu' });
    const tape = machine.execute({ kind: 'ejectTap' });
    expect(tape.result).toMatchObject({ kind: 'bytes', source: 'tap' });

    machine.execute({ action: 'play', kind: 'tapeTransport' });
    machine.execute({ action: 'seek', kind: 'tapeTransport', pulseIndex: 7 });
    expect(tapePlay).toHaveBeenCalledOnce();
    expect(tapeSeekPulse).toHaveBeenCalledWith(7);
  });

  it('owns 1541 firmware and disk images through one request at each boundary', () => {
    const { attachDrive1541, mountDrive1541G64, vm } = createVm();
    const machine = new C64WasmWorkerMachine(vm);

    const rom = new Uint8Array(0x4000).buffer;
    machine.execute({ deviceNumber: 8, kind: 'attachDrive1541', rom });
    expect(attachDrive1541).toHaveBeenCalledWith(8, new Uint8Array(rom));

    const disk = Uint8Array.of(1, 2, 3).buffer;
    machine.execute({
      bytes: disk,
      format: 'g64',
      kind: 'mountDrive1541Disk',
      writeProtected: true,
    });
    expect(mountDrive1541G64).toHaveBeenCalledWith(Uint8Array.of(1, 2, 3), true);

    const ejected = machine.execute({ kind: 'ejectDrive1541Disk' });
    expect(ejected.result).toMatchObject({ format: 'g64', kind: 'driveDisk' });
    expect(ejected.transfer).toEqual([
      ejected.result.kind === 'driveDisk' ? ejected.result.bytes : undefined,
    ]);
  });
});
