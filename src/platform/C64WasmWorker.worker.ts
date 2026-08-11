// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 生产 Wasm 整机 Worker
//
//   文件:       C64WasmWorker.worker.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { TransferableBufferPool } from './TransferableBufferPool';
import {
  C64WasmWorkerMachine,
  c64WorkerOperationMutatesMachine,
  type C64WasmVm,
} from './C64WasmWorkerMachine';
import type {
  C64WorkerCommand,
  C64WorkerEvent,
  C64WorkerFrameEvent,
  C64WorkerProgramLoadedEvent,
  C64WorkerState,
} from './C64WasmWorkerProtocol';
import { frameDurationMs } from '../video/C64VideoStandard';

interface C64WasmVmConstructor {
  withFirmware(
    profile: number,
    videoStandard: number,
    basic: Uint8Array,
    character: Uint8Array,
    kernal: Uint8Array,
  ): C64WasmVm;
}

interface C64WasmInitOutput {
  readonly memory: WebAssembly.Memory;
}

interface C64WasmWebModule {
  readonly C64Vm: C64WasmVmConstructor;
  readonly default: () => Promise<C64WasmInitOutput>;
}

interface WorkerScope {
  addEventListener(
    type: 'message',
    listener: (event: MessageEvent<C64WorkerCommand>) => void,
  ): void;
  close(): void;
  postMessage(message: C64WorkerEvent, transfer: Transferable[]): void;
}

const MAXIMUM_CATCH_UP_FRAMES = 3;
const BASIC_BOOT_FRAME_LIMIT = 300;
const AUDIO_BUFFER_SAMPLE_CAPACITY = 2_048;
const DOUBLE_BUFFER_CAPACITY = 2;

const scope = globalThis as unknown as WorkerScope;
let vm: C64WasmVm | undefined;
let machine: C64WasmWorkerMachine | undefined;
let wasmMemory: WebAssembly.Memory | undefined;
let frameBuffers: TransferableBufferPool | undefined;
let audioBuffers: TransferableBufferPool | undefined;
let width = 0;
let height = 0;
let sampleRate = 0;
let scheduledFrameDurationMs = 0;
let state: C64WorkerState = 'paused';
let timer: number | undefined;
let frameScheduleToken = 0;
let nextFrameDeadline = 0;
let disposed = false;
const cancelledRequests = new Set<number>();
const frameTaskChannel = new MessageChannel();

frameTaskChannel.port1.onmessage = (event: MessageEvent<number>) => {
  runScheduledFrame(event.data);
};

scope.addEventListener('message', (event) => {
  void handleCommand(event.data).catch((error: unknown) => publishFatalError(error));
});

async function handleCommand(command: C64WorkerCommand): Promise<void> {
  switch (command.type) {
    case 'initialize':
      await initialize(command);
      return;
    case 'start':
      start();
      return;
    case 'pause':
      pause();
      return;
    case 'reset':
      requireVm().reset();
      return;
    case 'stepFrame':
      if (state === 'paused') produceFrame(false);
      return;
    case 'loadProgram':
      await loadProgram(command.requestId, command.bytes, command.resetMachine);
      return;
    case 'request':
      executeRequest(command);
      return;
    case 'cancelRequest':
      cancelledRequests.add(command.requestId);
      return;
    case 'hostInput':
      requireVm().set_host_input(
        command.rowsByColumn,
        command.shiftLockPressed,
        command.joystickPort1Grounded,
        command.joystickPort2Grounded,
        command.restoreKeyPressed,
      );
      return;
    case 'recycleFrame':
      requireFrameBuffers().release(command.buffer);
      return;
    case 'recycleAudio':
      requireAudioBuffers().release(command.buffer);
      return;
    case 'dispose':
      dispose();
      return;
  }
}

async function initialize(command: Extract<C64WorkerCommand, { readonly type: 'initialize' }>) {
  try {
    if (vm) throw new Error('C64 Wasm Worker has already been initialized.');
    const imported: unknown = await import(/* @vite-ignore */ command.wasmModuleUrl);
    assertC64WasmWebModule(imported);
    const initialized = await imported.default();
    const nextVm = imported.C64Vm.withFirmware(
      0,
      command.videoStandard,
      new Uint8Array(command.firmware.basic),
      new Uint8Array(command.firmware.character),
      new Uint8Array(command.firmware.kernal),
    );
    nextVm.reset();

    vm = nextVm;
    machine = new C64WasmWorkerMachine(nextVm);
    wasmMemory = initialized.memory;
    width = nextVm.frame_width();
    height = nextVm.frame_height();
    sampleRate = nextVm.audio_sample_rate_hz();
    const processorClockHz = nextVm.processor_clock_hz();
    const videoFrameCycles = nextVm.video_frame_cycles();
    const videoStandard = nextVm.video_standard();
    if (videoStandard !== 0 && videoStandard !== 1) {
      throw new Error(`Rust/Wasm returned invalid video standard ${videoStandard}.`);
    }
    if (videoStandard !== command.videoStandard) {
      throw new Error(
        `Rust/Wasm initialized video standard ${videoStandard}; expected ${command.videoStandard}.`,
      );
    }
    scheduledFrameDurationMs = frameDurationMs(processorClockHz, videoFrameCycles);
    const frameByteLength = width * height * Uint32Array.BYTES_PER_ELEMENT;
    frameBuffers = new TransferableBufferPool(frameByteLength, DOUBLE_BUFFER_CAPACITY);
    audioBuffers = new TransferableBufferPool(
      AUDIO_BUFFER_SAMPLE_CAPACITY * Float32Array.BYTES_PER_ELEMENT,
      DOUBLE_BUFFER_CAPACITY,
    );
    post(
      {
        height,
        processorClockHz,
        requestId: command.requestId,
        sampleRate,
        type: 'initialized',
        videoFrameCycles,
        videoStandard,
        width,
      },
      [],
    );
  } catch (error: unknown) {
    publishRequestError(command.requestId, error);
  }
}

function start(): void {
  requireVm();
  if (state === 'running') return;
  state = 'running';
  nextFrameDeadline = performance.now();
  publishState();
  scheduleFrame(0);
}

function pause(): void {
  if (state === 'paused') return;
  state = 'paused';
  clearScheduledFrame();
  publishState();
}

function scheduleFrame(delayMs: number): void {
  clearScheduledFrame();
  const token = frameScheduleToken;
  if (delayMs <= 0) {
    frameTaskChannel.port2.postMessage(token);
    return;
  }
  timer = setTimeout(() => {
    timer = undefined;
    if (token === frameScheduleToken) frameTaskChannel.port2.postMessage(token);
  }, delayMs);
}

function runScheduledFrame(token: number): void {
  if (token !== frameScheduleToken) return;
  if (state !== 'running' || disposed) return;
  if (!produceFrame(true)) {
    scheduleFrame(1);
    return;
  }

  nextFrameDeadline += scheduledFrameDurationMs;
  const now = performance.now();
  if (now - nextFrameDeadline > scheduledFrameDurationMs * MAXIMUM_CATCH_UP_FRAMES) {
    nextFrameDeadline = now;
  }
  scheduleFrame(Math.max(0, nextFrameDeadline - now));
}

function produceFrame(streamAudio: boolean): boolean {
  const nextVm = requireVm();
  const frameBuffer = requireFrameBuffers().acquire();
  if (!frameBuffer) return false;
  const startedAt = performance.now();
  try {
    nextVm.run_until_next_frame();
    publishCurrentFrame(frameBuffer, performance.now() - startedAt, streamAudio);
    return true;
  } catch (error: unknown) {
    requireFrameBuffers().release(frameBuffer);
    throw error;
  }
}

function publishCurrentFrame(
  frameBuffer: ArrayBuffer,
  renderTime: number,
  streamAudio: boolean,
): void {
  const nextVm = requireVm();
  const memory = requireWasmMemory();
  const pixelCount = nextVm.frame_pixels_len();
  if (pixelCount !== width * height) {
    throw new Error(`Wasm frame contains ${pixelCount} pixels; expected ${width * height}.`);
  }
  new Uint32Array(frameBuffer).set(
    new Uint32Array(memory.buffer, nextVm.frame_pixels_ptr(), pixelCount),
  );

  const audioSampleCount = streamAudio ? nextVm.audio_sample_count() : 0;
  const audioBuffer = audioSampleCount === 0 ? undefined : requireAudioBuffers().acquire();
  if (audioBuffer && audioSampleCount > AUDIO_BUFFER_SAMPLE_CAPACITY) {
    requireAudioBuffers().release(audioBuffer);
    throw new Error(
      `Wasm audio batch contains ${audioSampleCount} samples; capacity is ${AUDIO_BUFFER_SAMPLE_CAPACITY}.`,
    );
  }
  if (audioBuffer) {
    new Float32Array(audioBuffer, 0, audioSampleCount).set(
      new Float32Array(memory.buffer, nextVm.audio_samples_ptr(), audioSampleCount),
    );
  }

  const message: C64WorkerFrameEvent = {
    audioBuffer: audioBuffer ?? null,
    audioSampleCount: audioBuffer ? audioSampleCount : 0,
    basicReady: nextVm.basic_ready(),
    frameBuffer,
    frameNumber: nextVm.frame_generation_low(),
    height,
    programCounter: nextVm.program_counter(),
    renderTime,
    sampleRate,
    type: 'frame',
    width,
  };
  post(message, audioBuffer ? [frameBuffer, audioBuffer] : [frameBuffer]);
}

async function loadProgram(
  requestId: number,
  bytes: ArrayBuffer,
  resetMachine: boolean,
): Promise<void> {
  const nextVm = requireVm();
  const resumeAfterLoad = state === 'running';
  pause();
  try {
    if (resetMachine) {
      nextVm.reset();
      let readyWasAbsent = !nextVm.basic_ready();
      let ready = false;
      for (let frame = 0; frame < BASIC_BOOT_FRAME_LIMIT; frame += 1) {
        throwIfCancelled(requestId);
        nextVm.run_until_next_frame();
        ready = nextVm.basic_ready();
        if (!ready) readyWasAbsent = true;
        else if (readyWasAbsent) break;
        await yieldToWorkerQueue();
      }
      if (!ready || !readyWasAbsent) {
        throw new Error(
          `C64 BASIC did not reach READY within ${BASIC_BOOT_FRAME_LIMIT} video frames.`,
        );
      }
    } else if (!nextVm.basic_ready()) {
      throw new Error('C64 BASIC must be ready before a PRG can be started with RUN.');
    }
    throwIfCancelled(requestId);
    const program = new Uint8Array(bytes);
    nextVm.install_basic_prg(program);
    const event: C64WorkerProgramLoadedEvent = {
      loadAddress: (program[0] ?? 0) | ((program[1] ?? 0) << 8),
      requestId,
      size: Math.max(0, program.length - 2),
      type: 'programLoaded',
    };
    post(event, []);
    cancelledRequests.delete(requestId);
    if (resumeAfterLoad) start();
  } catch (error: unknown) {
    cancelledRequests.delete(requestId);
    publishRequestError(requestId, error);
  }
}

function executeRequest(command: Extract<C64WorkerCommand, { readonly type: 'request' }>): void {
  const resumeAfterRequest = state === 'running';
  const mutatesMachine = c64WorkerOperationMutatesMachine(command.operation);
  if (mutatesMachine) pause();
  try {
    const execution = requireMachine().execute(command.operation);
    post(
      {
        requestId: command.requestId,
        result: execution.result,
        type: 'requestCompleted',
      },
      execution.transfer,
    );
    if (mutatesMachine && resumeAfterRequest) start();
  } catch (error: unknown) {
    publishRequestError(command.requestId, error);
  }
}

function throwIfCancelled(requestId: number): void {
  if (cancelledRequests.has(requestId)) throw new DOMException('Operation aborted.', 'AbortError');
}

function yieldToWorkerQueue(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function publishState(): void {
  post({ state, type: 'state' }, []);
}

function publishRequestError(requestId: number, error: unknown): void {
  post({ error: errorMessage(error), requestId, type: 'requestError' }, []);
}

function publishFatalError(error: unknown): void {
  pause();
  post({ error: errorMessage(error), type: 'fatalError' }, []);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function clearScheduledFrame(): void {
  frameScheduleToken += 1;
  if (timer === undefined) return;
  clearTimeout(timer);
  timer = undefined;
}

function dispose(): void {
  if (disposed) return;
  disposed = true;
  pause();
  vm?.free();
  vm = undefined;
  machine = undefined;
  wasmMemory = undefined;
  frameBuffers = undefined;
  audioBuffers = undefined;
  frameTaskChannel.port1.close();
  frameTaskChannel.port2.close();
  scope.close();
}

function requireVm(): C64WasmVm {
  if (!vm) throw new Error('C64 Wasm Worker is not initialized.');
  return vm;
}

function requireMachine(): C64WasmWorkerMachine {
  if (!machine) throw new Error('C64 Wasm Worker machine is not initialized.');
  return machine;
}

function requireWasmMemory(): WebAssembly.Memory {
  if (!wasmMemory) throw new Error('C64 Wasm memory is not initialized.');
  return wasmMemory;
}

function requireFrameBuffers(): TransferableBufferPool {
  if (!frameBuffers) throw new Error('C64 frame buffers are not initialized.');
  return frameBuffers;
}

function requireAudioBuffers(): TransferableBufferPool {
  if (!audioBuffers) throw new Error('C64 audio buffers are not initialized.');
  return audioBuffers;
}

function post(message: C64WorkerEvent, transfer: Transferable[]): void {
  scope.postMessage(message, transfer);
}

function assertC64WasmWebModule(value: unknown): asserts value is C64WasmWebModule {
  if (typeof value !== 'object' || value === null) {
    throw new TypeError('Wasm module namespace is not an object.');
  }
  const candidate = value as Partial<C64WasmWebModule>;
  if (
    typeof candidate.default !== 'function' ||
    typeof candidate.C64Vm?.withFirmware !== 'function'
  ) {
    throw new TypeError('Wasm module does not export the production C64Vm facade.');
  }
}
