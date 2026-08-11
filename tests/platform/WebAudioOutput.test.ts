// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - AudioWorklet PCM 流输出测试
//
//   文件:       WebAudioOutput.test.ts
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it, vi } from 'vitest';

import {
  WebAudioOutput,
  type AudioContextFactory,
  type PcmAudioNode,
  type WebAudioOutputOptions,
  type WebAudioOutputStatus,
} from '../../src/platform/WebAudioOutput';
import type {
  PcmAudioStreamMetrics,
  PcmAudioWorkletCommand,
} from '../../src/platform/PcmAudioWorkletProtocol';

class FakeMessagePort extends EventTarget {
  readonly messages: unknown[] = [];
  readonly close = vi.fn();
  readonly start = vi.fn();
  readonly postMessage = vi.fn((message: unknown): void => {
    this.messages.push(message);
  });

  emitMetrics(metrics: PcmAudioStreamMetrics): void {
    this.dispatchEvent(new MessageEvent('message', { data: metrics }));
  }

  emitRecycle(buffer: ArrayBuffer, recycleToken: number): void {
    this.dispatchEvent(
      new MessageEvent('message', { data: { buffer, recycleToken, type: 'recycle' } }),
    );
  }
}

class FakeAudioWorkletNode implements PcmAudioNode {
  readonly port = new FakeMessagePort();
  readonly disconnect = vi.fn();
}

class FakeAudioContext extends EventTarget {
  readonly addWorkletModule = vi.fn((): Promise<void> => Promise.resolve());
  readonly node = new FakeAudioWorkletNode();
  readonly createPcmNode = vi.fn((): PcmAudioNode => this.node);
  readonly sampleRate: number;
  state: AudioContextState = 'suspended';
  resumeError: Error | undefined;
  readonly close = vi.fn((): Promise<void> => {
    this.state = 'closed';
    return Promise.resolve();
  });
  readonly resume = vi.fn((): Promise<void> => {
    if (this.resumeError) return Promise.reject(this.resumeError);
    this.state = 'running';
    this.dispatchEvent(new Event('statechange'));
    return Promise.resolve();
  });

  constructor(sampleRate = 44_100) {
    super();
    this.sampleRate = sampleRate;
  }
}

interface AudioHarness {
  readonly contextFactory: ReturnType<typeof vi.fn<AudioContextFactory>>;
  readonly node: FakeAudioWorkletNode;
  readonly output: WebAudioOutput;
}

function createHarness(
  context = new FakeAudioContext(),
  target = new EventTarget(),
  overrides: Partial<WebAudioOutputOptions> = {},
): AudioHarness {
  const contextFactory = vi.fn<AudioContextFactory>(() => context);
  const output = new WebAudioOutput(target, {
    contextFactory,
    sampleRateHz: 44_100,
    workletModuleUrl: 'test-c64-pcm-worklet.js',
    ...overrides,
  });
  return { contextFactory, node: context.node, output };
}

describe('WebAudioOutput', () => {
  it('loads and connects one AudioWorklet after the first host gesture', async () => {
    const target = new EventTarget();
    const context = new FakeAudioContext();
    const { contextFactory, output } = createHarness(context, target);
    const statuses: WebAudioOutputStatus[] = [];
    output.observeStatus((status) => statuses.push(status));

    expect(output.status.state).toBe('inactive');
    expect(contextFactory).not.toHaveBeenCalled();
    target.dispatchEvent(new Event('pointerdown'));

    await vi.waitFor(() => expect(output.status.state).toBe('running'));
    expect(contextFactory).toHaveBeenCalledWith(44_100);
    expect(context.addWorkletModule).toHaveBeenCalledWith('test-c64-pcm-worklet.js');
    expect(context.createPcmNode).toHaveBeenCalledOnce();
    expect(context.resume).toHaveBeenCalledOnce();
    expect(statuses.map(({ state }) => state)).toEqual(['inactive', 'suspended', 'running']);
    output.dispose();
  });

  it('turns resume rejection into an observable retryable error without duplicating the node', async () => {
    const context = new FakeAudioContext();
    context.resumeError = new Error('gesture was rejected');
    const { output } = createHarness(context);

    await expect(output.activate()).resolves.toMatchObject({
      error: context.resumeError,
      state: 'error',
    });

    context.resumeError = undefined;
    await expect(output.activate()).resolves.toMatchObject({ state: 'running' });
    expect(context.resume).toHaveBeenCalledTimes(2);
    expect(context.addWorkletModule).toHaveBeenCalledOnce();
    expect(context.createPcmNode).toHaveBeenCalledOnce();
    output.dispose();
  });

  it('coalesces concurrent activation requests into one worklet initialization', async () => {
    const context = new FakeAudioContext();
    let finishModuleLoad: (() => void) | undefined;
    context.addWorkletModule.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishModuleLoad = resolve;
        }),
    );
    const { output } = createHarness(context);

    const firstActivation = output.activate();
    const secondActivation = output.activate();

    expect(secondActivation).toBe(firstActivation);
    expect(context.addWorkletModule).toHaveBeenCalledOnce();
    finishModuleLoad?.();
    await expect(Promise.all([firstActivation, secondActivation])).resolves.toEqual([
      { state: 'running' },
      { state: 'running' },
    ]);
    expect(context.createPcmNode).toHaveBeenCalledOnce();
    expect(context.resume).toHaveBeenCalledOnce();
    output.dispose();
  });

  it('does not stream samples before activation or while the context is suspended', async () => {
    const context = new FakeAudioContext();
    context.resumeError = new Error('remain suspended');
    const { node, output } = createHarness(context);

    output.enqueue(new Float32Array(882), 44_100);
    expect(node.port.messages).toHaveLength(0);
    await output.activate();
    output.enqueue(new Float32Array(882), 44_100);
    expect(node.port.messages).toHaveLength(0);
    output.dispose();
  });

  it('posts batched PCM chunks and clears the bounded worklet stream', async () => {
    const context = new FakeAudioContext();
    context.state = 'running';
    const { node, output } = createHarness(context);
    await output.activate();

    const first = Float32Array.of(-0.5, 0, 0.5);
    const second = Float32Array.of(0.25, -0.25);
    output.enqueue(first, 44_100);
    output.enqueue(second, 44_100);
    output.clear();

    const commands = node.port.messages as PcmAudioWorkletCommand[];
    expect(commands.map(({ type }) => type)).toEqual(['samples', 'samples', 'clear']);
    expect(commands[0]).toMatchObject({ buffer: first.buffer, sampleCount: 3, type: 'samples' });
    expect(commands[1]).toMatchObject({ buffer: second.buffer, sampleCount: 2, type: 'samples' });
    expect(output.streamMetrics.bufferedSamples).toBe(0);
    expect(context.resume).not.toHaveBeenCalled();
    output.dispose();
  });

  it('returns transferred Worker PCM ownership only after the worklet copies it', async () => {
    const context = new FakeAudioContext();
    context.state = 'running';
    const { node, output } = createHarness(context);
    await output.activate();
    const buffer = new ArrayBuffer(32);
    const recycle = vi.fn();

    expect(output.enqueueTransfer(buffer, 4, 44_100, recycle)).toBe(true);
    const command = node.port.messages[0] as PcmAudioWorkletCommand;
    expect(command).toMatchObject({ buffer, sampleCount: 4, type: 'samples' });
    if (command.type !== 'samples') throw new Error('Expected a PCM sample command.');
    expect(recycle).not.toHaveBeenCalled();

    node.port.emitRecycle(buffer, command.recycleToken);
    expect(recycle).toHaveBeenCalledWith(buffer);
    node.port.emitRecycle(buffer, command.recycleToken);
    expect(recycle).toHaveBeenCalledOnce();
    output.dispose();
  });

  it('drops stale PCM when the browser suspends audio and resumes the same worklet cleanly', async () => {
    const context = new FakeAudioContext();
    context.state = 'running';
    const { node, output } = createHarness(context);
    await output.activate();

    const beforeBackground = Float32Array.of(0.25, -0.25);
    output.enqueue(beforeBackground, 44_100);
    context.state = 'suspended';
    context.dispatchEvent(new Event('statechange'));
    output.enqueue(Float32Array.of(0.75), 44_100);

    expect(output.status.state).toBe('suspended');
    expect((node.port.messages as PcmAudioWorkletCommand[]).map(({ type }) => type)).toEqual([
      'samples',
      'clear',
    ]);

    await expect(output.activate()).resolves.toEqual({ state: 'running' });
    const afterBackground = Float32Array.of(-0.5, 0.5);
    output.enqueue(afterBackground, 44_100);

    const commands = node.port.messages as PcmAudioWorkletCommand[];
    expect(commands.map(({ type }) => type)).toEqual(['samples', 'clear', 'samples']);
    expect(commands[2]).toMatchObject({
      buffer: afterBackground.buffer,
      sampleCount: 2,
      type: 'samples',
    });
    expect(context.addWorkletModule).toHaveBeenCalledOnce();
    expect(context.createPcmNode).toHaveBeenCalledOnce();
    expect(context.resume).toHaveBeenCalledOnce();
    output.dispose();
  });

  it('publishes explicit overrun and underrun metrics from the worklet', async () => {
    const context = new FakeAudioContext();
    context.state = 'running';
    const { node, output } = createHarness(context);
    await output.activate();

    node.port.emitMetrics({
      bufferedSamples: 512,
      capacitySamples: 22_050,
      clearCount: 3,
      overrunSamples: 17,
      type: 'metrics',
      underrunSamples: 128,
    });

    expect(output.streamMetrics).toEqual({
      bufferedSamples: 512,
      capacitySamples: 22_050,
      clearCount: 3,
      overrunSamples: 17,
      type: 'metrics',
      underrunSamples: 128,
    });
    output.dispose();
  });

  it('rejects a mismatched hardware sample rate instead of silently resampling', async () => {
    const context = new FakeAudioContext(48_000);
    const { output } = createHarness(context);

    const status = await output.activate();
    expect(status.state).toBe('error');
    expect(status.error?.message).toContain('48000');
    expect(context.addWorkletModule).not.toHaveBeenCalled();
    expect(context.createPcmNode).not.toHaveBeenCalled();
    output.dispose();
  });

  it('reports module-load failure without falling back to per-buffer sources', async () => {
    const context = new FakeAudioContext();
    const moduleError = new Error('worklet module rejected');
    context.addWorkletModule.mockRejectedValueOnce(moduleError);
    const { output } = createHarness(context);

    await expect(output.activate()).resolves.toEqual({ error: moduleError, state: 'error' });
    expect(context.createPcmNode).not.toHaveBeenCalled();
    output.dispose();
  });

  it('reports an unavailable Web Audio implementation without throwing', async () => {
    const output = new WebAudioOutput(new EventTarget(), {
      contextFactory: () => undefined,
      workletModuleUrl: 'test-c64-pcm-worklet.js',
    });

    await expect(output.activate()).resolves.toEqual({ state: 'unavailable' });
    expect(output.status.state).toBe('unavailable');
    output.dispose();
  });
});
