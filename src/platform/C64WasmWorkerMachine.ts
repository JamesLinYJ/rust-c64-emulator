// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - Worker 粗粒度机器命令执行器
//
//   文件:       C64WasmWorkerMachine.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import type {
  C64Counter64,
  C64WorkerDiagnostics,
  C64WorkerOperation,
  C64WorkerOperationResult,
} from './C64WasmWorkerProtocol';

export interface C64WasmVm {
  attach_drive1541(deviceNumber: number, rom: Uint8Array): void;
  attach_reu(sizeKib: number): void;
  attach_reu_image(sizeKib: number, image: Uint8Array): void;
  audio_sample_count(): number;
  audio_sample_rate_hz(): number;
  audio_samples_ptr(): number;
  awaiting_auto_calibration(): boolean;
  basic_ready(): boolean;
  cartridge_attached(): boolean;
  cartridge_kind(): number;
  current_slot(): number;
  detach_drive1541(): void;
  detach_reu(): Uint8Array;
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
  easyflash_dirty(): boolean;
  effective_slots_per_system_cycle(): number;
  eject_cartridge(): void;
  eject_drive1541_disk(): Uint8Array;
  eject_tap(): Uint8Array;
  export_easyflash_high(): Uint8Array;
  export_easyflash_low(): Uint8Array;
  export_reu_ram(): Uint8Array;
  frame_generation_low(): number;
  frame_height(): number;
  frame_pixels_len(): number;
  frame_pixels_ptr(): number;
  frame_width(): number;
  free(): void;
  insert_blank_tap(videoStandard: number): void;
  insert_crt(bytes: Uint8Array, easyFlashJumperInstalled: boolean): void;
  insert_tap(bytes: Uint8Array, legacyV0OverflowPulseCycles: number): void;
  install_basic_prg(bytes: Uint8Array): void;
  load_state(bytes: Uint8Array): void;
  memory_generation_low(): number;
  mount_drive1541_d64(bytes: Uint8Array, writeProtected: boolean): void;
  mount_drive1541_g64(bytes: Uint8Array, writeProtected: boolean): void;
  program_counter(): number;
  processor_clock_hz(): number;
  reset(): void;
  reu_attached(): boolean;
  reu_dma_active(): boolean;
  reu_size_kib(): number;
  run_until_next_frame(): number;
  save_state(): Uint8Array;
  set_host_input(
    pressedRowsByColumn: Uint8Array,
    shiftLockPressed: boolean,
    joystickPort1Grounded: number,
    joystickPort2Grounded: number,
    restoreKeyPressed: boolean,
  ): void;
  tape_motor_active(): boolean;
  tape_mounted(): boolean;
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
}

export interface C64WorkerOperationExecution {
  readonly result: C64WorkerOperationResult;
  readonly transfer: Transferable[];
}

const ACKNOWLEDGED: C64WorkerOperationExecution = { result: { kind: 'ack' }, transfer: [] };

export class C64WasmWorkerMachine {
  constructor(private readonly vm: C64WasmVm) {}

  execute(operation: C64WorkerOperation): C64WorkerOperationExecution {
    switch (operation.kind) {
      case 'saveState':
        return bytesResult('state', this.vm.save_state());
      case 'loadState':
        this.vm.load_state(new Uint8Array(operation.bytes));
        return ACKNOWLEDGED;
      case 'diagnostics':
        return { result: { kind: 'diagnostics', value: this.diagnostics() }, transfer: [] };
      case 'insertCartridge':
        this.vm.insert_crt(new Uint8Array(operation.bytes), operation.easyFlashJumperInstalled);
        if (operation.resetMachine) this.vm.reset();
        return ACKNOWLEDGED;
      case 'ejectCartridge':
        this.vm.eject_cartridge();
        if (operation.resetMachine) this.vm.reset();
        return ACKNOWLEDGED;
      case 'exportEasyFlash':
        return bytesResult(
          operation.chip === 'low' ? 'easyFlashLow' : 'easyFlashHigh',
          operation.chip === 'low'
            ? this.vm.export_easyflash_low()
            : this.vm.export_easyflash_high(),
        );
      case 'attachReu':
        if (operation.image) {
          this.vm.attach_reu_image(operation.sizeKib, new Uint8Array(operation.image));
        } else {
          this.vm.attach_reu(operation.sizeKib);
        }
        return ACKNOWLEDGED;
      case 'detachReu':
        return bytesResult('reu', this.vm.detach_reu());
      case 'exportReu':
        return bytesResult('reu', this.vm.export_reu_ram());
      case 'insertTap':
        this.vm.insert_tap(new Uint8Array(operation.bytes), operation.legacyV0OverflowPulseCycles);
        return ACKNOWLEDGED;
      case 'insertBlankTap':
        this.vm.insert_blank_tap(operation.videoStandard);
        return ACKNOWLEDGED;
      case 'ejectTap':
        return bytesResult('tap', this.vm.eject_tap());
      case 'tapeTransport':
        this.executeTapeTransport(operation);
        return ACKNOWLEDGED;
      case 'attachDrive1541':
        this.vm.attach_drive1541(operation.deviceNumber, new Uint8Array(operation.rom));
        return ACKNOWLEDGED;
      case 'detachDrive1541':
        this.vm.detach_drive1541();
        return ACKNOWLEDGED;
      case 'mountDrive1541Disk':
        if (operation.format === 'd64') {
          this.vm.mount_drive1541_d64(new Uint8Array(operation.bytes), operation.writeProtected);
        } else {
          this.vm.mount_drive1541_g64(new Uint8Array(operation.bytes), operation.writeProtected);
        }
        return ACKNOWLEDGED;
      case 'ejectDrive1541Disk':
        return driveDiskResult(this.vm.eject_drive1541_disk());
    }
  }

  private diagnostics(): C64WorkerDiagnostics {
    return {
      awaitingAutoCalibration: this.vm.awaiting_auto_calibration(),
      cartridgeAttached: this.vm.cartridge_attached(),
      cartridgeKind: this.vm.cartridge_kind(),
      cpuBusTransactions: counter(
        this.vm.diagnostics_cpu_bus_transactions_low(),
        this.vm.diagnostics_cpu_bus_transactions_high(),
      ),
      currentSlot: this.vm.current_slot(),
      drive1541Attached: this.vm.drive1541_attached(),
      drive1541DiskMounted: this.vm.drive1541_disk_mounted(),
      drive1541DiskWriteProtected: this.vm.drive1541_disk_write_protected(),
      easyFlashDirty: this.vm.easyflash_dirty(),
      effectiveSlotsPerSystemCycle: this.vm.effective_slots_per_system_cycle(),
      elapsedSystemCycles: counter(
        this.vm.diagnostics_elapsed_system_cycles_low(),
        this.vm.diagnostics_elapsed_system_cycles_high(),
      ),
      executionModeChanges: counter(
        this.vm.diagnostics_execution_mode_changes_low(),
        this.vm.diagnostics_execution_mode_changes_high(),
      ),
      frameNumber: this.vm.frame_generation_low(),
      heldCpuReadSystemCycles: counter(
        this.vm.diagnostics_held_cpu_read_system_cycles_low(),
        this.vm.diagnostics_held_cpu_read_system_cycles_high(),
      ),
      memoryGeneration: this.vm.memory_generation_low(),
      programCounter: this.vm.program_counter(),
      retiredCpuSlots: counter(
        this.vm.diagnostics_retired_cpu_slots_low(),
        this.vm.diagnostics_retired_cpu_slots_high(),
      ),
      reuAttached: this.vm.reu_attached(),
      reuDmaActive: this.vm.reu_dma_active(),
      reuDmaBusCycles: counter(
        this.vm.diagnostics_reu_dma_bus_cycles_low(),
        this.vm.diagnostics_reu_dma_bus_cycles_high(),
      ),
      reuDmaSystemCycles: counter(
        this.vm.diagnostics_reu_dma_system_cycles_low(),
        this.vm.diagnostics_reu_dma_system_cycles_high(),
      ),
      reuDmaVicStallCycles: counter(
        this.vm.diagnostics_reu_dma_vic_stall_cycles_low(),
        this.vm.diagnostics_reu_dma_vic_stall_cycles_high(),
      ),
      reuSizeKib: this.vm.reu_size_kib(),
      stateLoads: counter(
        this.vm.diagnostics_state_loads_low(),
        this.vm.diagnostics_state_loads_high(),
      ),
      tapeMotorActive: this.vm.tape_motor_active(),
      tapeMounted: this.vm.tape_mounted(),
      tapePulseCount: this.vm.tape_pulse_count(),
      tapePulseIndex: this.vm.tape_pulse_index(),
      tapeSenseSwitchClosed: this.vm.tape_sense_switch_closed(),
      tapeTransport: this.vm.tape_transport(),
      tapeWritable: this.vm.tape_writable(),
    };
  }

  private executeTapeTransport(
    operation: Extract<C64WorkerOperation, { readonly kind: 'tapeTransport' }>,
  ): void {
    switch (operation.action) {
      case 'play':
        this.vm.tape_play();
        return;
      case 'record':
        this.vm.tape_record();
        return;
      case 'stop':
        this.vm.tape_stop();
        return;
      case 'rewind':
        this.vm.tape_rewind();
        return;
      case 'seek':
        this.vm.tape_seek_pulse(operation.pulseIndex);
        return;
    }
  }
}

export function c64WorkerOperationMutatesMachine(operation: C64WorkerOperation): boolean {
  switch (operation.kind) {
    case 'saveState':
    case 'diagnostics':
    case 'exportEasyFlash':
    case 'exportReu':
      return false;
    default:
      return true;
  }
}

function counter(low: number, high: number): C64Counter64 {
  return { high: high >>> 0, low: low >>> 0 };
}

function bytesResult(
  source: Extract<C64WorkerOperationResult, { readonly kind: 'bytes' }>['source'],
  bytes: Uint8Array,
): C64WorkerOperationExecution {
  const buffer = Uint8Array.from(bytes).buffer;
  return { result: { bytes: buffer, kind: 'bytes', source }, transfer: [buffer] };
}

function driveDiskResult(exported: Uint8Array): C64WorkerOperationExecution {
  const format = exported[0];
  if (format !== 0 && format !== 1) {
    throw new Error(`Rust/Wasm returned invalid 1541 disk format code ${String(format)}.`);
  }
  const bytes = Uint8Array.from(exported.subarray(1)).buffer;
  return {
    result: { bytes, format: format === 0 ? 'd64' : 'g64', kind: 'driveDisk' },
    transfer: [bytes],
  };
}
