// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - AudioWorklet PCM 流输出生命周期
//
//   文件:       WebAudioOutput.ts
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import c64PcmAudioWorkletUrl from './C64PcmAudioWorklet.ts?worker&url';
import {
  C64_PCM_AUDIO_PROCESSOR_NAME,
  type PcmAudioStreamMetrics,
  type PcmAudioBufferRecycle,
  type PcmAudioWorkletCommand,
} from './PcmAudioWorkletProtocol';

const DEFAULT_SAMPLE_RATE_HZ = 44_100;

export interface PcmAudioMessagePort {
  addEventListener(type: 'message', listener: EventListenerOrEventListenerObject): void;
  close(): void;
  postMessage(message: unknown, transfer?: Transferable[]): void;
  removeEventListener(type: 'message', listener: EventListenerOrEventListenerObject): void;
  start(): void;
}

export interface PcmAudioNode {
  readonly port: PcmAudioMessagePort;
  disconnect(): void;
}

export interface WebAudioContext {
  readonly sampleRate: number;
  readonly state: AudioContextState;
  addEventListener(type: 'statechange', listener: () => void): void;
  addWorkletModule(url: string): Promise<void>;
  close(): Promise<void>;
  createPcmNode(processorName: string, options: AudioWorkletNodeOptions): PcmAudioNode;
  removeEventListener(type: 'statechange', listener: () => void): void;
  resume(): Promise<void>;
}

export type AudioContextFactory = (sampleRateHz: number) => WebAudioContext | undefined;
export type WebAudioOutputState = 'error' | 'inactive' | 'running' | 'suspended' | 'unavailable';

export interface WebAudioOutputOptions {
  readonly contextFactory?: AudioContextFactory;
  readonly sampleRateHz?: number;
  readonly workletModuleUrl?: string;
}

export interface WebAudioOutputStatus {
  readonly error?: Error;
  readonly state: WebAudioOutputState;
}

export type WebAudioOutputStatusObserver = (status: WebAudioOutputStatus) => void;

class BrowserWebAudioContext implements WebAudioContext {
  private readonly handleStateChange = (): void => {
    for (const listener of this.stateListeners) listener();
  };
  private readonly stateListeners = new Set<() => void>();

  constructor(private readonly context: AudioContext) {
    context.addEventListener('statechange', this.handleStateChange);
  }

  get sampleRate(): number {
    return this.context.sampleRate;
  }

  get state(): AudioContextState {
    return this.context.state;
  }

  addEventListener(_type: 'statechange', listener: () => void): void {
    this.stateListeners.add(listener);
  }

  addWorkletModule(url: string): Promise<void> {
    return this.context.audioWorklet.addModule(url);
  }

  async close(): Promise<void> {
    this.context.removeEventListener('statechange', this.handleStateChange);
    this.stateListeners.clear();
    await this.context.close();
  }

  createPcmNode(processorName: string, options: AudioWorkletNodeOptions): PcmAudioNode {
    const node = new AudioWorkletNode(this.context, processorName, options);
    node.connect(this.context.destination);
    return node;
  }

  removeEventListener(_type: 'statechange', listener: () => void): void {
    this.stateListeners.delete(listener);
  }

  resume(): Promise<void> {
    return this.context.resume();
  }
}

function createBrowserAudioContext(sampleRateHz: number): WebAudioContext | undefined {
  return typeof AudioContext === 'undefined'
    ? undefined
    : new BrowserWebAudioContext(
        new AudioContext({ latencyHint: 'interactive', sampleRate: sampleRateHz }),
      );
}

function emptyMetrics(): PcmAudioStreamMetrics {
  return {
    bufferedSamples: 0,
    capacitySamples: 0,
    clearCount: 0,
    overrunSamples: 0,
    type: 'metrics',
    underrunSamples: 0,
  };
}

function isPcmAudioStreamMetrics(value: unknown): value is PcmAudioStreamMetrics {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Partial<PcmAudioStreamMetrics>;
  return (
    candidate.type === 'metrics' &&
    typeof candidate.bufferedSamples === 'number' &&
    typeof candidate.capacitySamples === 'number' &&
    typeof candidate.clearCount === 'number' &&
    typeof candidate.overrunSamples === 'number' &&
    typeof candidate.underrunSamples === 'number'
  );
}

function isPcmAudioBufferRecycle(value: unknown): value is PcmAudioBufferRecycle {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Partial<PcmAudioBufferRecycle>;
  return (
    candidate.type === 'recycle' &&
    candidate.buffer instanceof ArrayBuffer &&
    typeof candidate.recycleToken === 'number'
  );
}

export class WebAudioOutput {
  private activationPromise: Promise<WebAudioOutputStatus> | undefined;
  private context: WebAudioContext | undefined;
  private readonly contextFactory: AudioContextFactory;
  private disposed = false;
  private metricsValue = emptyMetrics();
  private node: PcmAudioNode | undefined;
  private nextRecycleToken = 1;
  private readonly recycleCallbacks = new Map<number, (buffer: ArrayBuffer) => void>();
  private readonly sampleRateHz: number;
  private readonly statusObservers = new Set<WebAudioOutputStatusObserver>();
  private statusValue: WebAudioOutputStatus = { state: 'inactive' };
  private readonly workletModuleUrl: string;

  private readonly handleActivation = (): void => {
    void this.activate();
  };

  private readonly handleContextStateChange = (): void => {
    // 浏览器进入后台会挂起 AudioContext；清空工作线程中的旧样本，避免恢复后补播陈旧声音。
    if (this.context?.state === 'suspended') this.clear();
    this.synchronizeStatusWithContext();
  };

  private readonly handleWorkletMessage = (event: Event): void => {
    if (!(event instanceof MessageEvent)) return;
    if (isPcmAudioStreamMetrics(event.data)) {
      this.metricsValue = event.data;
      return;
    }
    if (!isPcmAudioBufferRecycle(event.data)) return;
    const recycle = this.recycleCallbacks.get(event.data.recycleToken);
    if (!recycle) return;
    this.recycleCallbacks.delete(event.data.recycleToken);
    recycle(event.data.buffer);
  };

  constructor(
    private readonly activationTarget: EventTarget,
    options: WebAudioOutputOptions = {},
  ) {
    this.contextFactory = options.contextFactory ?? createBrowserAudioContext;
    this.sampleRateHz = options.sampleRateHz ?? DEFAULT_SAMPLE_RATE_HZ;
    this.workletModuleUrl = options.workletModuleUrl ?? c64PcmAudioWorkletUrl;
    if (!Number.isFinite(this.sampleRateHz) || this.sampleRateHz <= 0) {
      throw new RangeError('Web Audio sample rate must be a positive finite number.');
    }
    activationTarget.addEventListener('pointerdown', this.handleActivation);
    activationTarget.addEventListener('keydown', this.handleActivation);
  }

  get status(): WebAudioOutputStatus {
    return this.statusValue;
  }

  get streamMetrics(): PcmAudioStreamMetrics {
    return this.metricsValue;
  }

  observeStatus(observer: WebAudioOutputStatusObserver): () => void {
    this.statusObservers.add(observer);
    observer(this.statusValue);
    return () => this.statusObservers.delete(observer);
  }

  activate(): Promise<WebAudioOutputStatus> {
    const existingActivation = this.activationPromise;
    if (existingActivation) return existingActivation;

    const activation = this.performActivation();
    this.activationPromise = activation;
    void activation.then(
      () => this.releaseActivation(activation),
      () => this.releaseActivation(activation),
    );
    return activation;
  }

  enqueue(samples: Float32Array, sampleRate: number): void {
    const transferableBuffer =
      samples.buffer instanceof ArrayBuffer &&
      samples.byteOffset === 0 &&
      samples.byteLength === samples.buffer.byteLength
        ? samples.buffer
        : Float32Array.from(samples).buffer;
    this.enqueueTransfer(transferableBuffer, samples.length, sampleRate, () => undefined);
  }

  enqueueTransfer(
    buffer: ArrayBuffer,
    sampleCount: number,
    sampleRate: number,
    recycle: (buffer: ArrayBuffer) => void,
  ): boolean {
    if (
      !Number.isSafeInteger(sampleCount) ||
      sampleCount < 0 ||
      sampleCount * Float32Array.BYTES_PER_ELEMENT > buffer.byteLength
    ) {
      throw new RangeError(`PCM sample count ${sampleCount} exceeds its transferred buffer.`);
    }
    if (sampleRate !== this.sampleRateHz) {
      throw new Error(
        `PCM sample rate ${sampleRate} Hz does not match AudioWorklet rate ${this.sampleRateHz} Hz.`,
      );
    }
    const context = this.context;
    const node = this.node;
    if (context?.state !== 'running' || !node || sampleCount === 0) {
      recycle(buffer);
      return false;
    }

    const recycleToken = this.nextRecycleToken;
    this.nextRecycleToken =
      this.nextRecycleToken === Number.MAX_SAFE_INTEGER ? 1 : recycleToken + 1;
    this.recycleCallbacks.set(recycleToken, recycle);
    const command: PcmAudioWorkletCommand = {
      buffer,
      recycleToken,
      sampleCount,
      type: 'samples',
    };
    try {
      node.port.postMessage(command, [buffer]);
    } catch (error: unknown) {
      this.recycleCallbacks.delete(recycleToken);
      recycle(buffer);
      throw error;
    }
    return true;
  }

  clear(): void {
    this.node?.port.postMessage({ type: 'clear' } satisfies PcmAudioWorkletCommand);
    this.metricsValue = { ...this.metricsValue, bufferedSamples: 0 };
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.activationTarget.removeEventListener('pointerdown', this.handleActivation);
    this.activationTarget.removeEventListener('keydown', this.handleActivation);
    this.clear();
    this.node?.port.removeEventListener('message', this.handleWorkletMessage);
    this.node?.port.close();
    this.node?.disconnect();
    this.node = undefined;
    this.context?.removeEventListener('statechange', this.handleContextStateChange);
    if (this.context) void this.context.close().catch(() => undefined);
    this.context = undefined;
    this.statusObservers.clear();
  }

  private async performActivation(): Promise<WebAudioOutputStatus> {
    if (this.disposed) {
      return this.setStatus('error', new Error('Audio output has already been disposed.'));
    }

    let context = this.context;
    if (!context) {
      try {
        context = this.contextFactory(this.sampleRateHz);
      } catch (error: unknown) {
        return this.setStatus('error', this.toError(error));
      }
      if (!context) return this.setStatus('unavailable');
      this.context = context;
      context.addEventListener('statechange', this.handleContextStateChange);
    }

    if (context.sampleRate !== this.sampleRateHz) {
      return this.setStatus(
        'error',
        new Error(
          `Browser created a ${context.sampleRate} Hz AudioContext; the SID stream requires ${this.sampleRateHz} Hz.`,
        ),
      );
    }

    try {
      await this.initializeWorklet(context);
    } catch (error: unknown) {
      return this.setStatus('error', this.toError(error));
    }

    if (context.state === 'suspended') {
      this.setStatus('suspended');
      try {
        await context.resume();
      } catch (error: unknown) {
        return this.setStatus('error', this.toError(error));
      }
    }
    return this.synchronizeStatusWithContext();
  }

  private async initializeWorklet(context: WebAudioContext): Promise<void> {
    if (this.node) return;
    await context.addWorkletModule(this.workletModuleUrl);
    if (this.disposed) throw new Error('Audio output was disposed during activation.');

    const node = context.createPcmNode(C64_PCM_AUDIO_PROCESSOR_NAME, {
      channelCount: 1,
      channelCountMode: 'explicit',
      channelInterpretation: 'speakers',
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    node.port.addEventListener('message', this.handleWorkletMessage);
    node.port.start();
    this.node = node;
  }

  private releaseActivation(activation: Promise<WebAudioOutputStatus>): void {
    if (this.activationPromise === activation) this.activationPromise = undefined;
  }

  private synchronizeStatusWithContext(): WebAudioOutputStatus {
    switch (this.context?.state) {
      case 'running':
        return this.setStatus('running');
      case 'suspended':
        return this.setStatus('suspended');
      case 'closed':
        return this.setStatus('error', new Error('Browser audio context was closed.'));
      default:
        return this.setStatus('inactive');
    }
  }

  private setStatus(state: WebAudioOutputState, error?: Error): WebAudioOutputStatus {
    if (this.statusValue.state === state && this.statusValue.error === error)
      return this.statusValue;
    this.statusValue = error === undefined ? { state } : { error, state };
    for (const observer of this.statusObservers) observer(this.statusValue);
    return this.statusValue;
  }

  private toError(error: unknown): Error {
    return error instanceof Error ? error : new Error(String(error));
  }
}
