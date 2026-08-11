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

export type C64WorkerCommand =
  | {
      readonly firmware: C64WorkerFirmwareBuffers;
      readonly requestId: number;
      readonly type: 'initialize';
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
  readonly requestId: number;
  readonly sampleRate: number;
  readonly type: 'initialized';
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
  | C64WorkerRequestErrorEvent
  | C64WorkerFrameEvent
  | { readonly state: C64WorkerState; readonly type: 'state' }
  | { readonly error: string; readonly type: 'fatalError' };
