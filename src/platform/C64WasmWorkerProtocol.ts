// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 生产 Wasm Worker 消息协议
//
//   文件:       C64WasmWorkerProtocol.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

export type C64WorkerState = 'paused' | 'running';

export interface C64WorkerFirmwareBuffers {
  readonly basic: ArrayBuffer;
  readonly character: ArrayBuffer;
  readonly kernal: ArrayBuffer;
}

export interface C64Counter64 {
  readonly high: number;
  readonly low: number;
}

export interface C64WorkerDiagnostics {
  readonly awaitingAutoCalibration: boolean;
  readonly cartridgeAttached: boolean;
  readonly cartridgeKind: number;
  readonly cpuBusTransactions: C64Counter64;
  readonly currentSlot: number;
  readonly drive1541Attached: boolean;
  readonly drive1541DiskMounted: boolean;
  readonly drive1541DiskWriteProtected: boolean;
  readonly easyFlashDirty: boolean;
  readonly effectiveSlotsPerSystemCycle: number;
  readonly elapsedSystemCycles: C64Counter64;
  readonly executionModeChanges: C64Counter64;
  readonly frameNumber: number;
  readonly heldCpuReadSystemCycles: C64Counter64;
  readonly memoryGeneration: number;
  readonly programCounter: number;
  readonly retiredCpuSlots: C64Counter64;
  readonly reuAttached: boolean;
  readonly reuDmaActive: boolean;
  readonly reuDmaBusCycles: C64Counter64;
  readonly reuDmaSystemCycles: C64Counter64;
  readonly reuDmaVicStallCycles: C64Counter64;
  readonly reuSizeKib: number;
  readonly stateLoads: C64Counter64;
  readonly tapeMotorActive: boolean;
  readonly tapeMounted: boolean;
  readonly tapePulseCount: number;
  readonly tapePulseIndex: number;
  readonly tapeSenseSwitchClosed: boolean;
  readonly tapeTransport: number;
  readonly tapeWritable: boolean;
}

export type C64WorkerOperation =
  | { readonly kind: 'saveState' }
  | { readonly bytes: ArrayBuffer; readonly kind: 'loadState' }
  | { readonly kind: 'diagnostics' }
  | {
      readonly bytes: ArrayBuffer;
      readonly easyFlashJumperInstalled: boolean;
      readonly kind: 'insertCartridge';
      readonly resetMachine: boolean;
    }
  | { readonly kind: 'ejectCartridge'; readonly resetMachine: boolean }
  | { readonly chip: 'high' | 'low'; readonly kind: 'exportEasyFlash' }
  | { readonly image: ArrayBuffer | null; readonly kind: 'attachReu'; readonly sizeKib: number }
  | { readonly kind: 'detachReu' }
  | { readonly kind: 'exportReu' }
  | {
      readonly bytes: ArrayBuffer;
      readonly kind: 'insertTap';
      readonly legacyV0OverflowPulseCycles: number;
    }
  | { readonly kind: 'insertBlankTap'; readonly videoStandard: number }
  | { readonly kind: 'ejectTap' }
  | ((
      | { readonly action: 'play' | 'record' | 'rewind' | 'stop' }
      | { readonly action: 'seek'; readonly pulseIndex: number }
    ) & { readonly kind: 'tapeTransport' })
  | { readonly deviceNumber: number; readonly kind: 'attachDrive1541'; readonly rom: ArrayBuffer }
  | { readonly kind: 'detachDrive1541' }
  | {
      readonly bytes: ArrayBuffer;
      readonly format: 'd64' | 'g64';
      readonly kind: 'mountDrive1541Disk';
      readonly writeProtected: boolean;
    }
  | { readonly kind: 'ejectDrive1541Disk' };

export type C64WorkerOperationResult =
  | { readonly kind: 'ack' }
  | {
      readonly bytes: ArrayBuffer;
      readonly kind: 'bytes';
      readonly source: 'easyFlashHigh' | 'easyFlashLow' | 'reu' | 'state' | 'tap';
    }
  | {
      readonly bytes: ArrayBuffer;
      readonly format: 'd64' | 'g64';
      readonly kind: 'driveDisk';
    }
  | { readonly kind: 'diagnostics'; readonly value: C64WorkerDiagnostics };

export type C64WorkerCommand =
  | {
      readonly firmware: C64WorkerFirmwareBuffers;
      readonly requestId: number;
      readonly type: 'initialize';
      readonly videoStandard: 0 | 1;
      readonly wasmModuleUrl: string;
    }
  | { readonly type: 'start' }
  | { readonly type: 'pause' }
  | { readonly type: 'reset' }
  | { readonly type: 'stepFrame' }
  | {
      readonly bytes: ArrayBuffer;
      readonly requestId: number;
      readonly resetMachine: boolean;
      readonly type: 'loadProgram';
    }
  | { readonly operation: C64WorkerOperation; readonly requestId: number; readonly type: 'request' }
  | { readonly requestId: number; readonly type: 'cancelRequest' }
  | {
      readonly joystickPort1Grounded: number;
      readonly joystickPort2Grounded: number;
      readonly restoreKeyPressed: boolean;
      readonly rowsByColumn: Uint8Array;
      readonly shiftLockPressed: boolean;
      readonly type: 'hostInput';
    }
  | { readonly buffer: ArrayBuffer; readonly type: 'recycleFrame' }
  | { readonly buffer: ArrayBuffer; readonly type: 'recycleAudio' }
  | { readonly type: 'dispose' };

export interface C64WorkerInitializedEvent {
  readonly height: number;
  readonly processorClockHz: number;
  readonly requestId: number;
  readonly sampleRate: number;
  readonly type: 'initialized';
  readonly videoFrameCycles: number;
  readonly videoStandard: 0 | 1;
  readonly width: number;
}

export interface C64WorkerProgramLoadedEvent {
  readonly loadAddress: number;
  readonly requestId: number;
  readonly size: number;
  readonly type: 'programLoaded';
}

export interface C64WorkerRequestErrorEvent {
  readonly error: string;
  readonly requestId: number;
  readonly type: 'requestError';
}

export interface C64WorkerRequestCompletedEvent {
  readonly requestId: number;
  readonly result: C64WorkerOperationResult;
  readonly type: 'requestCompleted';
}

export interface C64WorkerFrameEvent {
  readonly audioBuffer: ArrayBuffer | null;
  readonly audioSampleCount: number;
  readonly basicReady: boolean;
  readonly frameBuffer: ArrayBuffer;
  readonly frameNumber: number;
  readonly height: number;
  readonly programCounter: number;
  readonly renderTime: number;
  readonly sampleRate: number;
  readonly type: 'frame';
  readonly width: number;
}

export type C64WorkerEvent =
  | C64WorkerInitializedEvent
  | C64WorkerProgramLoadedEvent
  | C64WorkerRequestCompletedEvent
  | C64WorkerRequestErrorEvent
  | C64WorkerFrameEvent
  | { readonly state: C64WorkerState; readonly type: 'state' }
  | { readonly error: string; readonly type: 'fatalError' };
