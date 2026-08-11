// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 主线程 Worker 整机控制器
//
//   文件:       C64WorkerEmulator.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import type {
  C64EmulatorOptions,
  C64ProgramLoadOptions,
  C64RemoteProgramLoadOptions,
} from '../core/C64Emulator';
import { C64KeyboardMatrix } from '../devices/C64KeyboardMatrix';
import type { RestoreKeyInput } from '../devices/RestoreKeyNmiCircuit';
import {
  assertPrgStartCompatibility,
  parsePrg,
  PRG_START_MODE,
  type LoadedProgram,
} from '../media/PrgLoader';
import { C64ControlPorts } from '../peripherals/control/C64ControlPorts';
import { assertSha256 } from '../shared/BinaryIntegrity';
import { TypedEventEmitter } from '../shared/TypedEventEmitter';
import { CanvasRenderer, C64_CANVAS_SIZE } from '../video/CanvasRenderer';
import { BrowserC64Input } from './BrowserC64Input';
import { fetchBinary, loadFirmware } from './FirmwareLoader';
import type {
  C64WorkerCommand,
  C64WorkerDiagnostics,
  C64WorkerEvent,
  C64WorkerFrameEvent,
  C64WorkerInitializedEvent,
  C64WorkerOperation,
  C64WorkerOperationResult,
  C64WorkerProgramLoadedEvent,
  C64WorkerRequestCompletedEvent,
  C64WorkerRequestErrorEvent,
  C64WorkerState,
} from './C64WasmWorkerProtocol';
import { WebAudioOutput, type WebAudioOutputStatus } from './WebAudioOutput';

interface C64WorkerEmulatorEvents {
  readonly audioState: WebAudioOutputStatus;
  readonly error: Error;
  readonly frame: { readonly frameNumber: number; readonly renderTime: number };
  readonly programLoaded: LoadedProgram;
  readonly state: C64WorkerState;
}

interface WorkerPort {
  addEventListener(type: 'error', listener: (event: ErrorEvent) => void): void;
  addEventListener(type: 'message', listener: (event: MessageEvent<C64WorkerEvent>) => void): void;
  postMessage(message: C64WorkerCommand, transfer?: Transferable[]): void;
  removeEventListener(type: 'error', listener: (event: ErrorEvent) => void): void;
  removeEventListener(
    type: 'message',
    listener: (event: MessageEvent<C64WorkerEvent>) => void,
  ): void;
  terminate(): void;
}

interface PendingRequest {
  readonly reject: (error: Error) => void;
  readonly resolve: (
    event: C64WorkerInitializedEvent | C64WorkerProgramLoadedEvent | C64WorkerRequestCompletedEvent,
  ) => void;
  readonly stopAbort: () => void;
}

export interface C64DriveDiskExport {
  readonly bytes: Uint8Array;
  readonly format: 'd64' | 'g64';
}

export type C64TapeTransportAction = 'play' | 'record' | 'rewind' | 'stop';

interface MutableCpuRegisters {
  programCounter: number;
}

const WASM_MODULE_PATH = 'wasm/c64_vm.js';

export class C64WorkerEmulator extends TypedEventEmitter<C64WorkerEmulatorEvents> {
  readonly input: BrowserC64Input;
  readonly registers: MutableCpuRegisters = { programCounter: 0 };
  readonly renderer: CanvasRenderer;

  private readonly audioOutput: WebAudioOutput;
  private basicReadyValue = false;
  private disposed = false;
  private readonly fetcher;
  private nextRequestId = 1;
  private readonly pendingRequests = new Map<number, PendingRequest>();
  private restoreKeyPressed = false;
  private stateValue: C64WorkerState = 'paused';
  private readonly stopObservingAudioState: () => void;
  private readonly stopObservingControlPorts: () => void;
  private readonly stopObservingKeyboard: () => void;

  private readonly handleWorkerError = (event: ErrorEvent): void => {
    this.emitFatalError(new Error(event.message || 'C64 Wasm Worker failed.'));
  };

  private readonly handleWorkerMessage = (event: MessageEvent<C64WorkerEvent>): void => {
    const message = event.data;
    switch (message.type) {
      case 'initialized':
      case 'programLoaded':
      case 'requestCompleted':
        this.resolveRequest(message.requestId, message);
        if (message.type === 'programLoaded') {
          this.emit('programLoaded', {
            endAddress: message.loadAddress + message.size,
            loadAddress: message.loadAddress,
            size: message.size,
            startMode: PRG_START_MODE.basicRun,
          });
        }
        return;
      case 'requestError':
        this.rejectRequest(message);
        return;
      case 'state':
        this.stateValue = message.state;
        if (message.state === 'paused') this.audioOutput.clear();
        this.emit('state', message.state);
        return;
      case 'frame':
        this.presentFrame(message);
        return;
      case 'fatalError':
        this.emitFatalError(new Error(message.error));
        return;
    }
  };

  private constructor(
    private readonly worker: WorkerPort,
    canvas: HTMLCanvasElement,
    keyboardTarget: EventTarget,
    options: C64EmulatorOptions,
  ) {
    super();
    this.fetcher = options.fetcher ?? fetch;
    this.renderer = new CanvasRenderer(canvas, 0xff000000);
    this.audioOutput = new WebAudioOutput(options.audioTarget ?? document);
    this.stopObservingAudioState = this.audioOutput.observeStatus((status) =>
      this.emit('audioState', status),
    );

    const controlPorts = new C64ControlPorts();
    const keyboard = new C64KeyboardMatrix();
    const restoreKeyInput: RestoreKeyInput = {
      setRestoreKeyPressed: (pressed) => {
        if (this.restoreKeyPressed === pressed) return;
        this.restoreKeyPressed = pressed;
        this.synchronizeHostInput(keyboard, controlPorts);
      },
    };
    this.input = new BrowserC64Input({
      controlPorts,
      ...(options.joystickPort !== undefined ? { joystickPort: options.joystickPort } : {}),
      keyboard,
      restoreKeyInput,
    });
    this.stopObservingKeyboard = keyboard.observeChanges(() =>
      this.synchronizeHostInput(keyboard, controlPorts),
    );
    this.stopObservingControlPorts = controlPorts.observeDeviceSignals(() =>
      this.synchronizeHostInput(keyboard, controlPorts),
    );
    worker.addEventListener('message', this.handleWorkerMessage);
    worker.addEventListener('error', this.handleWorkerError);
    this.input.attach(keyboardTarget);
    this.synchronizeHostInput(keyboard, controlPorts);
  }

  static async create(options: C64EmulatorOptions = {}): Promise<C64WorkerEmulator> {
    const signal = options.signal;
    signal?.throwIfAborted();
    const firmware = await loadFirmware(options.firmwareUrls, options.fetcher ?? fetch, signal);
    signal?.throwIfAborted();

    const worker: WorkerPort = new Worker(new URL('./C64WasmWorker.worker.ts', import.meta.url), {
      name: 'c64-wasm-runtime',
      type: 'module',
    });
    try {
      const initialized = await initializeWorker(worker, firmware, signal);
      if (
        initialized.width !== C64_CANVAS_SIZE.width ||
        initialized.height !== C64_CANVAS_SIZE.height
      ) {
        throw new Error(
          `Rust/Wasm video size ${initialized.width}x${initialized.height} does not match the PAL canvas.`,
        );
      }
      return new C64WorkerEmulator(
        worker,
        options.canvas ?? document.createElement('canvas'),
        options.keyboardTarget ?? document,
        options,
      );
    } catch (error: unknown) {
      worker.terminate();
      throw error;
    }
  }

  get state(): C64WorkerState {
    return this.stateValue;
  }

  get basicReady(): boolean {
    return this.basicReadyValue;
  }

  get audioStatus(): WebAudioOutputStatus {
    return this.audioOutput.status;
  }

  enableAudio(): Promise<WebAudioOutputStatus> {
    return this.audioOutput.activate();
  }

  start(): void {
    this.post({ type: 'start' });
  }

  pause(): void {
    this.post({ type: 'pause' });
  }

  toggle(): void {
    if (this.stateValue === 'running') this.pause();
    else this.start();
  }

  reset(): void {
    this.basicReadyValue = false;
    this.audioOutput.clear();
    this.post({ type: 'reset' });
  }

  stepFrame(): void {
    if (this.stateValue === 'paused') this.post({ type: 'stepFrame' });
  }

  async loadProgram(
    url: string,
    options: C64RemoteProgramLoadOptions = {},
  ): Promise<LoadedProgram> {
    const bytes = await fetchBinary(url, this.fetcher, options.signal);
    if (options.expectedSha256 !== undefined) {
      await assertSha256(bytes, options.expectedSha256, `PRG ${url}`);
    }
    return this.loadProgramBytesAsync(bytes, options, options.signal);
  }

  async loadProgramBytesAsync(
    input: ArrayBuffer | Uint8Array,
    options: C64ProgramLoadOptions = {},
    signal?: AbortSignal,
  ): Promise<LoadedProgram> {
    const image = parsePrg(input);
    const startMode = options.startMode ?? PRG_START_MODE.basicRun;
    assertPrgStartCompatibility(image, {
      ...(options.entryAddress !== undefined ? { entryAddress: options.entryAddress } : {}),
      startMode,
    });
    if (startMode !== PRG_START_MODE.basicRun) {
      throw new Error('The production Rust/Wasm page currently accepts BASIC RUN PRG loading.');
    }

    const bytes =
      input instanceof Uint8Array ? Uint8Array.from(input) : new Uint8Array(input.slice(0));
    const requestId = this.takeRequestId();
    const event = await this.sendRequest<C64WorkerProgramLoadedEvent>(
      requestId,
      {
        bytes: bytes.buffer,
        requestId,
        resetMachine: options.resetMachine ?? true,
        type: 'loadProgram',
      },
      [bytes.buffer],
      signal,
    );
    return {
      endAddress: event.loadAddress + event.size,
      loadAddress: event.loadAddress,
      size: event.size,
      startMode,
    };
  }

  async saveState(signal?: AbortSignal): Promise<Uint8Array> {
    const result = await this.requestOperation({ kind: 'saveState' }, [], signal);
    return operationBytes(result, 'state');
  }

  async loadState(input: ArrayBuffer | Uint8Array, signal?: AbortSignal): Promise<void> {
    const bytes = copyToArrayBuffer(input);
    requireAcknowledgement(
      await this.requestOperation({ bytes, kind: 'loadState' }, [bytes], signal),
    );
  }

  async getDiagnostics(signal?: AbortSignal): Promise<C64WorkerDiagnostics> {
    const result = await this.requestOperation({ kind: 'diagnostics' }, [], signal);
    if (result.kind !== 'diagnostics') {
      throw new Error(`Worker returned ${result.kind} for a diagnostics request.`);
    }
    return result.value;
  }

  async insertCartridgeBytes(
    input: ArrayBuffer | Uint8Array,
    options: {
      readonly easyFlashJumperInstalled?: boolean;
      readonly resetMachine?: boolean;
      readonly signal?: AbortSignal;
    } = {},
  ): Promise<void> {
    const bytes = copyToArrayBuffer(input);
    requireAcknowledgement(
      await this.requestOperation(
        {
          bytes,
          easyFlashJumperInstalled: options.easyFlashJumperInstalled ?? false,
          kind: 'insertCartridge',
          resetMachine: options.resetMachine ?? true,
        },
        [bytes],
        options.signal,
      ),
    );
  }

  async ejectCartridge(resetMachine = true, signal?: AbortSignal): Promise<void> {
    requireAcknowledgement(
      await this.requestOperation({ kind: 'ejectCartridge', resetMachine }, [], signal),
    );
  }

  async exportEasyFlash(chip: 'high' | 'low', signal?: AbortSignal): Promise<Uint8Array> {
    const result = await this.requestOperation({ chip, kind: 'exportEasyFlash' }, [], signal);
    return operationBytes(result, chip === 'low' ? 'easyFlashLow' : 'easyFlashHigh');
  }

  async attachReu(
    sizeKib: number,
    image?: ArrayBuffer | Uint8Array,
    signal?: AbortSignal,
  ): Promise<void> {
    const imageBuffer = image === undefined ? null : copyToArrayBuffer(image);
    requireAcknowledgement(
      await this.requestOperation(
        { image: imageBuffer, kind: 'attachReu', sizeKib },
        imageBuffer ? [imageBuffer] : [],
        signal,
      ),
    );
  }

  async detachReu(signal?: AbortSignal): Promise<Uint8Array> {
    const result = await this.requestOperation({ kind: 'detachReu' }, [], signal);
    return operationBytes(result, 'reu');
  }

  async exportReu(signal?: AbortSignal): Promise<Uint8Array> {
    const result = await this.requestOperation({ kind: 'exportReu' }, [], signal);
    return operationBytes(result, 'reu');
  }

  async insertTap(
    input: ArrayBuffer | Uint8Array,
    legacyV0OverflowPulseCycles = 0,
    signal?: AbortSignal,
  ): Promise<void> {
    const bytes = copyToArrayBuffer(input);
    requireAcknowledgement(
      await this.requestOperation(
        { bytes, kind: 'insertTap', legacyV0OverflowPulseCycles },
        [bytes],
        signal,
      ),
    );
  }

  async insertBlankTap(videoStandard: number, signal?: AbortSignal): Promise<void> {
    requireAcknowledgement(
      await this.requestOperation({ kind: 'insertBlankTap', videoStandard }, [], signal),
    );
  }

  async ejectTap(signal?: AbortSignal): Promise<Uint8Array> {
    const result = await this.requestOperation({ kind: 'ejectTap' }, [], signal);
    return operationBytes(result, 'tap');
  }

  async setTapeTransport(action: C64TapeTransportAction, signal?: AbortSignal): Promise<void> {
    requireAcknowledgement(
      await this.requestOperation({ action, kind: 'tapeTransport' }, [], signal),
    );
  }

  async seekTape(pulseIndex: number, signal?: AbortSignal): Promise<void> {
    requireAcknowledgement(
      await this.requestOperation(
        { action: 'seek', kind: 'tapeTransport', pulseIndex },
        [],
        signal,
      ),
    );
  }

  async attachDrive1541(
    deviceNumber: number,
    rom: ArrayBuffer | Uint8Array,
    signal?: AbortSignal,
  ): Promise<void> {
    const romBuffer = copyToArrayBuffer(rom);
    requireAcknowledgement(
      await this.requestOperation(
        { deviceNumber, kind: 'attachDrive1541', rom: romBuffer },
        [romBuffer],
        signal,
      ),
    );
  }

  async detachDrive1541(signal?: AbortSignal): Promise<void> {
    requireAcknowledgement(await this.requestOperation({ kind: 'detachDrive1541' }, [], signal));
  }

  async mountDrive1541Disk(
    format: 'd64' | 'g64',
    input: ArrayBuffer | Uint8Array,
    writeProtected = false,
    signal?: AbortSignal,
  ): Promise<void> {
    const bytes = copyToArrayBuffer(input);
    requireAcknowledgement(
      await this.requestOperation(
        { bytes, format, kind: 'mountDrive1541Disk', writeProtected },
        [bytes],
        signal,
      ),
    );
  }

  async ejectDrive1541Disk(signal?: AbortSignal): Promise<C64DriveDiskExport> {
    const result = await this.requestOperation({ kind: 'ejectDrive1541Disk' }, [], signal);
    if (result.kind !== 'driveDisk') {
      throw new Error(`Worker returned ${result.kind} for a 1541 disk eject request.`);
    }
    return { bytes: new Uint8Array(result.bytes), format: result.format };
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.input.dispose();
    this.stopObservingKeyboard();
    this.stopObservingControlPorts();
    this.stopObservingAudioState();
    this.audioOutput.dispose();
    this.worker.removeEventListener('message', this.handleWorkerMessage);
    this.worker.removeEventListener('error', this.handleWorkerError);
    for (const pending of this.pendingRequests.values()) {
      pending.stopAbort();
      pending.reject(new Error('C64 Wasm Worker was disposed.'));
    }
    this.pendingRequests.clear();
    this.worker.postMessage({ type: 'dispose' });
    this.worker.terminate();
    this.clearListeners();
  }

  private synchronizeHostInput(keyboard: C64KeyboardMatrix, controlPorts: C64ControlPorts): void {
    if (this.disposed) return;
    const snapshot = keyboard.snapshot();
    this.post({
      joystickPort1Grounded: controlPorts.port1.deviceSignals.groundedDigitalLines,
      joystickPort2Grounded: controlPorts.port2.deviceSignals.groundedDigitalLines,
      restoreKeyPressed: this.restoreKeyPressed,
      rowsByColumn: snapshot.rowsByColumn,
      shiftLockPressed: snapshot.shiftLockPressed,
      type: 'hostInput',
    });
  }

  private presentFrame(frame: C64WorkerFrameEvent): void {
    try {
      if (frame.width !== C64_CANVAS_SIZE.width || frame.height !== C64_CANVAS_SIZE.height) {
        throw new Error(`Worker frame has unexpected dimensions ${frame.width}x${frame.height}.`);
      }
      this.renderer.presentPixels(new Uint32Array(frame.frameBuffer));
      this.registers.programCounter = frame.programCounter;
      this.basicReadyValue = frame.basicReady;
      this.emit('frame', { frameNumber: frame.frameNumber, renderTime: frame.renderTime });
    } finally {
      this.post({ buffer: frame.frameBuffer, type: 'recycleFrame' }, [frame.frameBuffer]);
    }

    if (frame.audioBuffer) {
      this.audioOutput.enqueueTransfer(
        frame.audioBuffer,
        frame.audioSampleCount,
        frame.sampleRate,
        (buffer) => this.post({ buffer, type: 'recycleAudio' }, [buffer]),
      );
    }
  }

  private resolveRequest(
    requestId: number,
    event: C64WorkerInitializedEvent | C64WorkerProgramLoadedEvent | C64WorkerRequestCompletedEvent,
  ): void {
    const pending = this.pendingRequests.get(requestId);
    if (!pending) return;
    this.pendingRequests.delete(requestId);
    pending.stopAbort();
    pending.resolve(event);
  }

  private rejectRequest(event: C64WorkerRequestErrorEvent): void {
    const pending = this.pendingRequests.get(event.requestId);
    if (!pending) return;
    this.pendingRequests.delete(event.requestId);
    pending.stopAbort();
    pending.reject(new Error(event.error));
  }

  private sendRequest<
    Result extends
      C64WorkerInitializedEvent | C64WorkerProgramLoadedEvent | C64WorkerRequestCompletedEvent,
  >(
    requestId: number,
    command: C64WorkerCommand,
    transfer: Transferable[],
    signal?: AbortSignal,
  ): Promise<Result> {
    signal?.throwIfAborted();
    if (this.disposed) {
      return Promise.reject(new Error('C64 Wasm Worker was disposed.'));
    }
    return new Promise<Result>((resolve, reject) => {
      const handleAbort = (): void => {
        this.pendingRequests.delete(requestId);
        this.worker.postMessage({ requestId, type: 'cancelRequest' });
        reject(abortError(signal));
      };
      signal?.addEventListener('abort', handleAbort, { once: true });
      this.pendingRequests.set(requestId, {
        reject,
        resolve: (event) => resolve(event as Result),
        stopAbort: () => signal?.removeEventListener('abort', handleAbort),
      });
      this.post(command, transfer);
    });
  }

  private async requestOperation(
    operation: C64WorkerOperation,
    transfer: Transferable[],
    signal?: AbortSignal,
  ): Promise<C64WorkerOperationResult> {
    const requestId = this.takeRequestId();
    const event = await this.sendRequest<C64WorkerRequestCompletedEvent>(
      requestId,
      { operation, requestId, type: 'request' },
      transfer,
      signal,
    );
    return event.result;
  }

  private takeRequestId(): number {
    const result = this.nextRequestId;
    this.nextRequestId = result === Number.MAX_SAFE_INTEGER ? 1 : result + 1;
    return result;
  }

  private emitFatalError(error: Error): void {
    this.stateValue = 'paused';
    this.audioOutput.clear();
    this.emit('error', error);
  }

  private post(command: C64WorkerCommand, transfer: Transferable[] = []): void {
    if (this.disposed) return;
    this.worker.postMessage(command, transfer);
  }
}

async function initializeWorker(
  worker: WorkerPort,
  firmware: Awaited<ReturnType<typeof loadFirmware>>,
  signal?: AbortSignal,
): Promise<C64WorkerInitializedEvent> {
  const requestId = 1;
  signal?.throwIfAborted();
  return new Promise<C64WorkerInitializedEvent>((resolve, reject) => {
    const cleanup = (): void => {
      worker.removeEventListener('message', handleMessage);
      worker.removeEventListener('error', handleError);
      signal?.removeEventListener('abort', handleAbort);
    };
    const handleMessage = (event: MessageEvent<C64WorkerEvent>): void => {
      const message = event.data;
      if (message.type === 'initialized' && message.requestId === requestId) {
        cleanup();
        resolve(message);
      } else if (message.type === 'requestError' && message.requestId === requestId) {
        cleanup();
        reject(new Error(message.error));
      }
    };
    const handleError = (event: ErrorEvent): void => {
      cleanup();
      reject(new Error(event.message || 'C64 Wasm Worker initialization failed.'));
    };
    const handleAbort = (): void => {
      cleanup();
      reject(abortError(signal));
    };
    worker.addEventListener('message', handleMessage);
    worker.addEventListener('error', handleError);
    signal?.addEventListener('abort', handleAbort, { once: true });

    const basic = exactArrayBuffer(firmware.basic);
    const character = exactArrayBuffer(firmware.character);
    const kernal = exactArrayBuffer(firmware.kernal);
    const base = import.meta.env.BASE_URL;
    const wasmModuleUrl = new URL(`${base}${WASM_MODULE_PATH}`, window.location.href).href;
    worker.postMessage(
      {
        firmware: { basic, character, kernal },
        requestId,
        type: 'initialize',
        wasmModuleUrl,
      },
      [basic, character, kernal],
    );
  });
}

function exactArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  return copyToArrayBuffer(bytes);
}

function copyToArrayBuffer(input: ArrayBuffer | Uint8Array): ArrayBuffer {
  return input instanceof Uint8Array ? Uint8Array.from(input).buffer : input.slice(0);
}

function requireAcknowledgement(result: C64WorkerOperationResult): void {
  if (result.kind !== 'ack') {
    throw new Error(`Worker returned ${result.kind} for a mutating operation.`);
  }
}

function operationBytes(
  result: C64WorkerOperationResult,
  source: Extract<C64WorkerOperationResult, { readonly kind: 'bytes' }>['source'],
): Uint8Array {
  if (result.kind !== 'bytes' || result.source !== source) {
    throw new Error(`Worker did not return the expected ${source} bytes.`);
  }
  return new Uint8Array(result.bytes);
}

function abortError(signal: AbortSignal | undefined): Error {
  if (signal?.reason instanceof Error) return signal.reason;
  return new DOMException('Operation aborted.', 'AbortError');
}
