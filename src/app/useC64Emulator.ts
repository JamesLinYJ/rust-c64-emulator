// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - React 模拟器生命周期控制器
//
//   文件:       useC64Emulator.ts
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

import type {
  C64EmulatorOptions,
  C64ProgramLoadOptions,
  C64RemoteProgramLoadOptions,
} from '../core/C64Emulator';
import type { BundledProgramDescriptor } from '../media/BundledProgramCatalog';
import { PRG_START_MODE, type LoadedProgram } from '../media/PrgLoader';
import type { BrowserC64Input } from '../platform/BrowserC64Input';
import type { WebAudioOutputStatus } from '../platform/WebAudioOutput';
import { hex } from '../shared/numbers';
import { PAL_VIDEO_STANDARD } from '../video/palVideoStandard';

export type EmulatorPhase = 'error' | 'loading' | 'paused' | 'running';
export type MessageTone = 'error' | 'normal';

interface C64EmulatorBackendEvents {
  readonly audioState: WebAudioOutputStatus;
  readonly error: Error;
  readonly frame: { readonly frameNumber: number; readonly renderTime: number };
  readonly programLoaded: LoadedProgram;
  readonly state: 'paused' | 'running';
}

export interface C64EmulatorBackend {
  readonly audioStatus: WebAudioOutputStatus;
  readonly basicReady: boolean;
  readonly input: Pick<BrowserC64Input, 'releaseJoystickSource' | 'setJoystickSourceLines'>;
  readonly registers: { readonly programCounter: number };
  readonly state: 'paused' | 'running';
  dispose(): void;
  enableAudio(): Promise<WebAudioOutputStatus>;
  loadProgram(url: string, options?: C64RemoteProgramLoadOptions): Promise<LoadedProgram>;
  loadProgramBytesAsync(
    input: ArrayBuffer | Uint8Array,
    options?: C64ProgramLoadOptions,
    signal?: AbortSignal,
  ): Promise<LoadedProgram>;
  on<EventName extends keyof C64EmulatorBackendEvents>(
    eventName: EventName,
    listener: (payload: C64EmulatorBackendEvents[EventName]) => void,
  ): () => void;
  reset(): void;
  start(): void;
  stepFrame(): void;
  toggle(): void;
}

export type C64EmulatorFactory = (options: C64EmulatorOptions) => Promise<C64EmulatorBackend>;

interface EmulatorViewState {
  readonly bootComplete: boolean;
  readonly framesPerSecond: number | undefined;
  readonly message: string;
  readonly messageTone: MessageTone;
  readonly overBudgetFrames: number;
  readonly phase: EmulatorPhase;
  readonly programCounter: string;
  readonly renderP95Ms: number | undefined;
  readonly sampledFrames: number;
}

export interface C64EmulatorController extends EmulatorViewState {
  readonly audioStatus: WebAudioOutputStatus;
  readonly enableAudio: () => Promise<void>;
  readonly isReady: boolean;
  readonly loadBuiltInProgram: (program: BundledProgramDescriptor) => Promise<boolean>;
  readonly loadLocalProgram: (file: File) => Promise<boolean>;
  readonly releaseJoystickSource: (sourceId: number) => void;
  readonly reset: () => void;
  readonly retryInitialization: () => void;
  readonly stepFrame: () => void;
  readonly setJoystickSourceLines: (sourceId: number, groundedDigitalLines: number) => void;
  readonly toggle: () => void;
}

const INITIAL_MESSAGE = '模拟器初始化中。屏幕就绪后可选择内置程序，或载入本地 PRG。';
const RETRY_MESSAGE = '正在重新载入固件并初始化模拟器…';
const READY_MESSAGE = 'BASIC 已就绪。选择程序后点击“载入”，或直接拖入 PRG 文件。';
const FRAME_SAMPLE_LIMIT = 120;
const PAL_FRAME_BUDGET_MS = 1000 / PAL_VIDEO_STANDARD.timing.refreshRateHz;
const HTTP_STATUS_PATTERN = /\bHTTP\s+(\d{3})\b/iu;
const NETWORK_FAILURE_PATTERN = /failed to fetch|fetch failed|networkerror|load failed/iu;

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}

export function describeInitializationFailure(error: unknown): string {
  const detail = toError(error).message;
  const httpStatus = HTTP_STATUS_PATTERN.exec(detail)?.[1];
  if (httpStatus) {
    return `模拟器固件下载失败（HTTP ${httpStatus}）。请稍后重试。`;
  }
  if (error instanceof TypeError && NETWORK_FAILURE_PATTERN.test(detail)) {
    return '模拟器固件下载失败。请检查网络连接后重试。';
  }
  return '模拟器启动失败。请重试；如果问题持续，请刷新页面。';
}

const createBrowserC64Emulator: C64EmulatorFactory = async (options) => {
  const { C64WorkerEmulator } = await import('../platform/C64WorkerEmulator');
  return C64WorkerEmulator.create(options);
};

export function useC64Emulator(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  keyboardTargetRef: RefObject<HTMLElement | null>,
  createEmulator: C64EmulatorFactory = createBrowserC64Emulator,
): C64EmulatorController {
  const createEmulatorRef = useRef(createEmulator);
  const emulatorRef = useRef<C64EmulatorBackend | null>(null);
  const bootCompleteRef = useRef(false);
  const programRequestRef = useRef<AbortController | null>(null);
  const operationIdRef = useRef(0);
  const [initializationAttempt, setInitializationAttempt] = useState(0);
  const [phase, setPhase] = useState<EmulatorPhase>('loading');
  const [framesPerSecond, setFramesPerSecond] = useState<number>();
  const [programCounter, setProgramCounter] = useState('0000');
  const [bootComplete, setBootComplete] = useState(false);
  const [message, setMessage] = useState(INITIAL_MESSAGE);
  const [messageTone, setMessageTone] = useState<MessageTone>('normal');
  const [renderP95Ms, setRenderP95Ms] = useState<number>();
  const [overBudgetFrames, setOverBudgetFrames] = useState(0);
  const [sampledFrames, setSampledFrames] = useState(0);
  const [audioStatus, setAudioStatus] = useState<WebAudioOutputStatus>({ state: 'inactive' });

  const showMessage = useCallback((nextMessage: string): void => {
    setMessage(nextMessage);
    setMessageTone('normal');
  }, []);

  const showOperationError = useCallback((error: unknown): void => {
    setMessage(toError(error).message);
    setMessageTone('error');
  }, []);

  const showFatalError = useCallback((error: unknown): void => {
    setPhase('error');
    setMessage(toError(error).message);
    setMessageTone('error');
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    const keyboardTarget = keyboardTargetRef.current;
    if (!canvas || !keyboardTarget) {
      showFatalError(new Error('模拟器界面未能正确挂载。'));
      return;
    }

    const initialization = new AbortController();
    let disposed = false;
    let emulator: C64EmulatorBackend | null = null;
    let framesSinceUpdate = 0;
    let lastFrameUpdate = performance.now();
    const renderTimes: number[] = [];
    bootCompleteRef.current = false;
    setBootComplete(false);
    setFramesPerSecond(undefined);
    setMessage(initializationAttempt === 0 ? INITIAL_MESSAGE : RETRY_MESSAGE);
    setMessageTone('normal');
    setOverBudgetFrames(0);
    setPhase('loading');
    setProgramCounter('0000');
    setRenderP95Ms(undefined);
    setSampledFrames(0);
    setAudioStatus({ state: 'inactive' });

    const initialize = async (): Promise<void> => {
      try {
        const base = import.meta.env.BASE_URL;
        emulator = await createEmulatorRef.current({
          canvas,
          firmwareUrls: {
            basic: `${base}firmware/basic.901226-01.bin`,
            character: `${base}firmware/characters.901225-01.bin`,
            kernal: `${base}firmware/kernal.901227-03.bin`,
          },
          keyboardTarget,
          signal: initialization.signal,
        });

        if (disposed) {
          emulator.dispose();
          return;
        }

        emulatorRef.current = emulator;
        setAudioStatus(emulator.audioStatus);
        emulator.on('audioState', setAudioStatus);
        emulator.on('state', (nextState) => {
          if (nextState === 'running') {
            framesSinceUpdate = 0;
            lastFrameUpdate = performance.now();
            setFramesPerSecond(undefined);
          }
          setPhase(nextState);
        });
        emulator.on('frame', ({ renderTime }) => {
          renderTimes.push(renderTime);
          if (renderTimes.length > FRAME_SAMPLE_LIMIT) renderTimes.shift();

          if (emulator?.state === 'running') {
            framesSinceUpdate += 1;
            const now = performance.now();
            const elapsed = now - lastFrameUpdate;
            if (elapsed >= 500) {
              const sortedRenderTimes = [...renderTimes].sort((left, right) => left - right);
              const p95Index = Math.max(0, Math.ceil(sortedRenderTimes.length * 0.95) - 1);
              setFramesPerSecond(Math.round((framesSinceUpdate * 1000) / elapsed));
              setRenderP95Ms(sortedRenderTimes[p95Index]);
              setOverBudgetFrames(
                renderTimes.filter((renderTimeMs) => renderTimeMs > PAL_FRAME_BUDGET_MS).length,
              );
              setSampledFrames(renderTimes.length);
              setProgramCounter(hex(emulator.registers.programCounter, 4));
              framesSinceUpdate = 0;
              lastFrameUpdate = now;
            }
          } else {
            setProgramCounter(hex(emulator?.registers.programCounter ?? 0, 4));
          }

          if (!bootCompleteRef.current && emulator?.basicReady) {
            bootCompleteRef.current = true;
            setBootComplete(true);
            showMessage(READY_MESSAGE);
          }
        });
        emulator.on('programLoaded', ({ loadAddress, size }) => {
          showMessage(`已载入 ${size.toLocaleString('zh-CN')} 字节至 $${hex(loadAddress, 4)}。`);
        });
        emulator.on('error', showFatalError);

        emulator.start();
        showMessage('BASIC 正在启动。选择程序后点击“载入”，或直接拖入 PRG 文件。');
      } catch (error: unknown) {
        if (!initialization.signal.aborted) {
          showFatalError(new Error(describeInitializationFailure(error)));
        }
      }
    };

    void initialize();

    return () => {
      disposed = true;
      initialization.abort();
      const programRequest = programRequestRef.current;
      programRequest?.abort();
      if (programRequestRef.current === programRequest) programRequestRef.current = null;
      operationIdRef.current += 1;
      if (emulatorRef.current === emulator) emulatorRef.current = null;
      emulator?.dispose();
    };
  }, [canvasRef, initializationAttempt, keyboardTargetRef, showFatalError, showMessage]);

  const loadBuiltInProgram = useCallback(
    async (program: BundledProgramDescriptor): Promise<boolean> => {
      const emulator = emulatorRef.current;
      if (!emulator) return false;

      programRequestRef.current?.abort();
      const request = new AbortController();
      programRequestRef.current = request;
      const operationId = ++operationIdRef.current;
      showMessage(`正在载入 ${program.title}…`);

      try {
        await emulator.loadProgram(`${import.meta.env.BASE_URL}programs/${program.file}`, {
          expectedSha256: program.sha256,
          signal: request.signal,
        });
        return operationIdRef.current === operationId;
      } catch (error: unknown) {
        if (!isAbortError(error)) showOperationError(error);
        return false;
      } finally {
        if (programRequestRef.current === request) programRequestRef.current = null;
      }
    },
    [showMessage, showOperationError],
  );

  const loadLocalProgram = useCallback(
    async (file: File): Promise<boolean> => {
      const emulator = emulatorRef.current;
      if (!emulator) return false;
      if (!file.name.toLowerCase().endsWith('.prg')) {
        showOperationError(new Error('请选择扩展名为 .prg 的 Commodore 64 程序。'));
        return false;
      }

      programRequestRef.current?.abort();
      const request = new AbortController();
      programRequestRef.current = request;
      const operationId = ++operationIdRef.current;
      showMessage(`正在读取 ${file.name}…`);

      try {
        const bytes = await file.arrayBuffer();
        if (
          request.signal.aborted ||
          operationIdRef.current !== operationId ||
          emulatorRef.current !== emulator
        ) {
          return false;
        }
        await emulator.loadProgramBytesAsync(
          bytes,
          { startMode: PRG_START_MODE.basicRun },
          request.signal,
        );
        return true;
      } catch (error: unknown) {
        if (!isAbortError(error)) showOperationError(error);
        return false;
      } finally {
        if (programRequestRef.current === request) programRequestRef.current = null;
      }
    },
    [showMessage, showOperationError],
  );

  const toggle = useCallback((): void => {
    emulatorRef.current?.toggle();
  }, []);

  const enableAudio = useCallback(async (): Promise<void> => {
    const emulator = emulatorRef.current;
    if (!emulator) return;

    const status = await emulator.enableAudio();
    if (emulatorRef.current !== emulator) return;
    setAudioStatus(status);
    switch (status.state) {
      case 'running':
        showMessage('声音已启用。');
        break;
      case 'unavailable':
        showOperationError(new Error('当前浏览器不支持 Web Audio，模拟器将静音运行。'));
        break;
      case 'error':
        showOperationError(
          new Error(`声音未能启用：${status.error?.message ?? '浏览器拒绝了音频请求。'}`),
        );
        break;
      default:
        break;
    }
  }, [showMessage, showOperationError]);

  const setJoystickSourceLines = useCallback(
    (sourceId: number, groundedDigitalLines: number): void => {
      emulatorRef.current?.input.setJoystickSourceLines(sourceId, groundedDigitalLines);
    },
    [],
  );

  const releaseJoystickSource = useCallback((sourceId: number): void => {
    emulatorRef.current?.input.releaseJoystickSource(sourceId);
  }, []);

  const reset = useCallback((): void => {
    const emulator = emulatorRef.current;
    if (!emulator) return;
    bootCompleteRef.current = false;
    setBootComplete(false);
    emulator.reset();
    setProgramCounter(hex(emulator.registers.programCounter, 4));
    showMessage('CPU 已复位，内存内容保持不变。');
  }, [showMessage]);

  const retryInitialization = useCallback((): void => {
    setPhase('loading');
    setMessage(RETRY_MESSAGE);
    setMessageTone('normal');
    setInitializationAttempt((current) => current + 1);
  }, []);

  const stepFrame = useCallback((): void => {
    const emulator = emulatorRef.current;
    if (!emulator || emulator.state === 'running') return;
    emulator.stepFrame();
    setProgramCounter(hex(emulator.registers.programCounter, 4));
    showMessage('已执行一帧。');
  }, [showMessage]);

  const isReady = bootComplete && (phase === 'paused' || phase === 'running');

  return {
    audioStatus,
    bootComplete,
    enableAudio,
    framesPerSecond,
    isReady,
    loadBuiltInProgram,
    loadLocalProgram,
    message,
    messageTone,
    overBudgetFrames,
    phase,
    programCounter,
    releaseJoystickSource,
    renderP95Ms,
    reset,
    retryInitialization,
    sampledFrames,
    stepFrame,
    setJoystickSourceLines,
    toggle,
  };
}
