// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - React 初始化恢复生命周期测试
//
//   文件:       useC64Emulator.test.tsx
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

// @vitest-environment jsdom

import { StrictMode, act, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  describeInitializationFailure,
  useC64Emulator,
  type C64EmulatorFactory,
} from '../../src/app/useC64Emulator';
import type { C64ProgramLoadOptions } from '../../src/core/C64Emulator';
import { PRG_START_MODE } from '../../src/media/PrgLoader';
import type { PcmAudioStreamMetrics } from '../../src/platform/PcmAudioWorkletProtocol';
import type { C64VideoStandard } from '../../src/video/C64VideoStandard';

const createEmulator = vi.fn<C64EmulatorFactory>();
type EmulatorInstance = Awaited<ReturnType<C64EmulatorFactory>>;
type FakeProgramLoader = (
  input: ArrayBuffer | Uint8Array,
  options: C64ProgramLoadOptions,
  signal: AbortSignal,
) => Promise<void>;

interface Deferred<Value> {
  readonly promise: Promise<Value>;
  readonly resolve: (value: Value) => void;
}

interface FakeEmulator {
  readonly audioStatus: { readonly state: 'inactive' };
  readonly audioStreamMetrics: PcmAudioStreamMetrics;
  readonly basicReady: boolean;
  readonly dispose: ReturnType<typeof vi.fn>;
  readonly enableAudio: ReturnType<typeof vi.fn>;
  readonly input: {
    readonly releaseJoystickSource: ReturnType<typeof vi.fn>;
    readonly setJoystickSourceLines: ReturnType<typeof vi.fn>;
  };
  readonly loadProgram: ReturnType<typeof vi.fn>;
  readonly loadProgramBytesAsync: ReturnType<typeof vi.fn<FakeProgramLoader>>;
  readonly on: ReturnType<typeof vi.fn>;
  readonly registers: { readonly programCounter: number };
  readonly reset: ReturnType<typeof vi.fn>;
  readonly start: ReturnType<typeof vi.fn>;
  readonly stepFrame: ReturnType<typeof vi.fn>;
  readonly toggle: ReturnType<typeof vi.fn>;
  state: 'paused' | 'running';
}

function createDeferred<Value>(): Deferred<Value> {
  let resolvePromise: ((value: Value) => void) | undefined;
  const promise = new Promise<Value>((resolve) => {
    resolvePromise = resolve;
  });
  return {
    promise,
    resolve: (value) => {
      if (!resolvePromise) throw new Error('Deferred promise was not initialized.');
      resolvePromise(value);
    },
  };
}

function createFakeEmulator(): FakeEmulator {
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const emit = (eventName: string, payload: unknown): void => {
    for (const listener of listeners.get(eventName) ?? []) listener(payload);
  };
  const emulator: FakeEmulator = {
    audioStatus: { state: 'inactive' },
    audioStreamMetrics: {
      bufferedSamples: 0,
      capacitySamples: 22_050,
      clearCount: 0,
      overrunSamples: 0,
      type: 'metrics',
      underrunSamples: 0,
    },
    basicReady: true,
    dispose: vi.fn(() => listeners.clear()),
    enableAudio: vi.fn(() => Promise.resolve({ state: 'running' })),
    input: {
      releaseJoystickSource: vi.fn(),
      setJoystickSourceLines: vi.fn(),
    },
    loadProgram: vi.fn(() => Promise.resolve()),
    loadProgramBytesAsync: vi.fn<FakeProgramLoader>(() => Promise.resolve()),
    on: vi.fn((eventName: string, listener: (payload: unknown) => void) => {
      const eventListeners = listeners.get(eventName) ?? new Set<(payload: unknown) => void>();
      eventListeners.add(listener);
      listeners.set(eventName, eventListeners);
      return () => eventListeners.delete(listener);
    }),
    registers: { programCounter: 0xfce2 },
    reset: vi.fn(),
    start: vi.fn(() => {
      emulator.state = 'running';
      emit('state', 'running');
      emit('frame', { renderTime: 1 });
    }),
    state: 'paused',
    stepFrame: vi.fn(),
    toggle: vi.fn(),
  };
  return emulator;
}

function Harness({
  localProgram,
  videoStandard = 'pal',
}: {
  readonly localProgram?: File;
  readonly videoStandard?: C64VideoStandard;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const keyboardTargetRef = useRef<HTMLDivElement>(null);
  const controller = useC64Emulator(canvasRef, keyboardTargetRef, createEmulator, videoStandard);
  return (
    <div ref={keyboardTargetRef}>
      <canvas ref={canvasRef} />
      <output data-phase={controller.phase}>{controller.message}</output>
      <button type="button" onClick={controller.retryInitialization}>
        Test retry
      </button>
      {localProgram ? (
        <button type="button" onClick={() => void controller.loadLocalProgram(localProgram)}>
          Test local PRG
        </button>
      ) : null}
    </div>
  );
}

describe('useC64Emulator initialization recovery', () => {
  beforeEach(() => {
    createEmulator.mockReset();
    document.body.innerHTML = '<div id="test-root"></div>';
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  });

  it('restarts at one effect boundary and disposes every superseded emulator in StrictMode', async () => {
    const staleEmulator = createFakeEmulator();
    const activeEmulator = createFakeEmulator();
    const staleCreation = createDeferred<EmulatorInstance>();
    const signals: AbortSignal[] = [];
    createEmulator.mockImplementation((options) => {
      if (!options.signal) throw new Error('Initialization did not receive an AbortSignal.');
      signals.push(options.signal);
      if (signals.length === 1) return staleCreation.promise;
      if (signals.length === 2) return Promise.reject(new TypeError('Failed to fetch'));
      return Promise.resolve(activeEmulator as unknown as EmulatorInstance);
    });

    const container = document.querySelector<HTMLElement>('#test-root');
    if (!container) throw new Error('Test root did not mount.');
    const root = createRoot(container);
    await act(async () => {
      root.render(
        <StrictMode>
          <Harness />
        </StrictMode>,
      );
      await Promise.resolve();
    });
    await act(async () => {
      await vi.waitFor(() => expect(createEmulator).toHaveBeenCalledTimes(2));
    });
    const status = container.querySelector('output');
    expect(status?.dataset['phase']).toBe('error');

    expect(status?.textContent).toBe('模拟器固件下载失败。请检查网络连接后重试。');
    expect(status?.textContent).not.toContain('Failed to fetch');

    await act(async () => {
      container.querySelector<HTMLButtonElement>('button')?.click();
      await Promise.resolve();
    });
    expect(createEmulator).toHaveBeenCalledTimes(3);
    expect(status?.dataset['phase']).toBe('running');

    expect(signals).toHaveLength(3);
    expect(signals[0]?.aborted).toBe(true);
    expect(signals[1]?.aborted).toBe(true);
    expect(signals[2]?.aborted).toBe(false);
    expect(activeEmulator.start).toHaveBeenCalledOnce();
    expect(activeEmulator.on).toHaveBeenCalledTimes(5);
    expect(document.querySelectorAll('canvas')).toHaveLength(1);

    await act(async () => {
      staleCreation.resolve(staleEmulator as unknown as EmulatorInstance);
      await staleCreation.promise;
    });
    expect(staleEmulator.dispose).toHaveBeenCalledOnce();
    expect(staleEmulator.start).not.toHaveBeenCalled();

    act(() => root.unmount());
    expect(activeEmulator.dispose).toHaveBeenCalledOnce();
    expect(signals[2]?.aborted).toBe(true);
  });

  it('disposes an emulator that resolves after its initialization was unmounted', async () => {
    const staleEmulator = createFakeEmulator();
    const staleCreation = createDeferred<EmulatorInstance>();
    let initializationSignal: AbortSignal | undefined;
    createEmulator.mockImplementation((options) => {
      initializationSignal = options.signal;
      return staleCreation.promise;
    });

    const container = document.querySelector<HTMLElement>('#test-root');
    if (!container) throw new Error('Test root did not mount.');
    const root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });
    await act(async () => {
      await vi.waitFor(() => expect(createEmulator).toHaveBeenCalledOnce());
    });

    act(() => root.unmount());
    expect(initializationSignal?.aborted).toBe(true);

    await act(async () => {
      staleCreation.resolve(staleEmulator as unknown as EmulatorInstance);
      await staleCreation.promise;
    });
    expect(staleEmulator.dispose).toHaveBeenCalledOnce();
    expect(staleEmulator.start).not.toHaveBeenCalled();
  });

  it('reinitializes the Worker backend when the video standard changes', async () => {
    const palEmulator = createFakeEmulator();
    const ntscEmulator = createFakeEmulator();
    createEmulator
      .mockResolvedValueOnce(palEmulator as unknown as EmulatorInstance)
      .mockResolvedValueOnce(ntscEmulator as unknown as EmulatorInstance);

    const container = document.querySelector<HTMLElement>('#test-root');
    if (!container) throw new Error('Test root did not mount.');
    const root = createRoot(container);
    await act(async () => {
      root.render(<Harness videoStandard="pal" />);
      await Promise.resolve();
    });
    await act(async () => {
      await vi.waitFor(() => expect(palEmulator.start).toHaveBeenCalledOnce());
    });
    expect(createEmulator.mock.calls[0]?.[0].videoStandard).toBe('pal');

    await act(async () => {
      root.render(<Harness videoStandard="ntsc" />);
      await Promise.resolve();
    });
    await act(async () => {
      await vi.waitFor(() => expect(ntscEmulator.start).toHaveBeenCalledOnce());
    });
    expect(palEmulator.dispose).toHaveBeenCalledOnce();
    expect(createEmulator.mock.calls[1]?.[0].videoStandard).toBe('ntsc');

    act(() => root.unmount());
    expect(ntscEmulator.dispose).toHaveBeenCalledOnce();
  });

  it('uses BASIC RUN explicitly for local PRGs and exposes an incompatible load address', async () => {
    const emulator = createFakeEmulator();
    const rejection = new Error(
      'BASIC RUN requires a PRG load address of $0801; machine-code.prg starts at $C000.',
    );
    emulator.loadProgramBytesAsync.mockRejectedValueOnce(rejection);
    createEmulator.mockResolvedValue(emulator as unknown as EmulatorInstance);
    const localProgram = new File([Uint8Array.of(0x00, 0xc0, 0x60)], 'machine-code.prg');

    const container = document.querySelector<HTMLElement>('#test-root');
    if (!container) throw new Error('Test root did not mount.');
    const root = createRoot(container);
    await act(async () => {
      root.render(<Harness localProgram={localProgram} />);
      await Promise.resolve();
    });
    await act(async () => {
      await vi.waitFor(() => expect(emulator.start).toHaveBeenCalledOnce());
    });

    await act(async () => {
      container.querySelector<HTMLButtonElement>('button:last-of-type')?.click();
      await vi.waitFor(() => expect(emulator.loadProgramBytesAsync).toHaveBeenCalledOnce());
    });

    const call = emulator.loadProgramBytesAsync.mock.calls[0];
    if (!call) throw new Error('Local PRG loader was not called.');
    const [bytes, options, signal] = call;
    expect(bytes).toBeInstanceOf(ArrayBuffer);
    expect(new Uint8Array(bytes)).toEqual(Uint8Array.of(0x00, 0xc0, 0x60));
    expect(options).toEqual({ startMode: PRG_START_MODE.basicRun });
    expect(signal).toBeInstanceOf(AbortSignal);
    expect(container.querySelector('output')?.textContent).toBe(rejection.message);
    expect(container.querySelector('output')?.dataset['phase']).toBe('running');

    act(() => root.unmount());
  });

  it('maps only initialization failures to actionable Chinese messages', () => {
    expect(describeInitializationFailure(new TypeError('Failed to fetch'))).toBe(
      '模拟器固件下载失败。请检查网络连接后重试。',
    );
    expect(
      describeInitializationFailure(
        new Error('Unable to load /firmware/basic.bin: HTTP 503 Service Unavailable.'),
      ),
    ).toBe('模拟器固件下载失败（HTTP 503）。请稍后重试。');
    expect(describeInitializationFailure(new Error('Unexpected decoder failure.'))).toBe(
      '模拟器启动失败。请重试；如果问题持续，请刷新页面。',
    );
  });
});
