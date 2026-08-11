// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - 模拟器屏幕与实时状态工作区
//
//   文件:       EmulatorWorkspace.tsx
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { RefreshCw, Volume2, VolumeX } from 'lucide-react';
import { useState, type CSSProperties, type FormEvent, type RefObject } from 'react';

import type { EmulatorPhase, MessageTone } from '../useC64Emulator';
import type { WebAudioOutputStatus } from '../../platform/WebAudioOutput';
import { PAL_VIDEO_STANDARD } from '../../video/palVideoStandard';
import { TouchControls } from './TouchControls';

interface ScreenStyle extends CSSProperties {
  '--screen-width': string;
}

export type DisplayScale = 'fit' | '1x' | '2x';

interface EmulatorWorkspaceProps {
  readonly audioStatus: WebAudioOutputStatus;
  readonly bootComplete: boolean;
  readonly canvasRef: RefObject<HTMLCanvasElement | null>;
  readonly displayScale: DisplayScale;
  readonly framesPerSecond: number | undefined;
  readonly keyboardInputRef: RefObject<HTMLTextAreaElement | null>;
  readonly message: string;
  readonly messageTone: MessageTone;
  readonly overBudgetFrames: number;
  readonly phase: EmulatorPhase;
  readonly onJoystickLinesChange: (sourceId: number, groundedDigitalLines: number) => void;
  readonly onJoystickRelease: (sourceId: number) => void;
  readonly onEnableAudio: () => Promise<void>;
  readonly onRetryInitialization: () => void;
  readonly programCounter: string;
  readonly renderP95Ms: number | undefined;
  readonly sampledFrames: number;
  readonly screenFrameRef: RefObject<HTMLDivElement | null>;
}

const PHASE_LABELS: Readonly<Record<EmulatorPhase, string>> = {
  error: '异常',
  loading: '初始化',
  paused: '已暂停',
  running: '就绪',
};

function focusKeyboardInput(keyboardInputRef: RefObject<HTMLTextAreaElement | null>): void {
  keyboardInputRef.current?.focus({ preventScroll: true });
}

function clearKeyboardInput(event: FormEvent<HTMLTextAreaElement>): void {
  event.currentTarget.value = '';
}

function AudioStatusControl({
  disabled,
  onEnableAudio,
  status,
}: {
  readonly disabled: boolean;
  readonly onEnableAudio: () => Promise<void>;
  readonly status: WebAudioOutputStatus;
}) {
  if (status.state === 'running') {
    return (
      <span className="audio-status-control is-running" role="status">
        <Volume2 aria-hidden="true" />
        声音已开启
      </span>
    );
  }
  if (status.state === 'unavailable') {
    return (
      <span className="audio-status-control is-unavailable" role="status">
        <VolumeX aria-hidden="true" />
        浏览器无音频支持
      </span>
    );
  }

  const label =
    status.state === 'error' ? '重试声音' : status.state === 'suspended' ? '恢复声音' : '启用声音';
  return (
    <button
      className={`audio-status-control${status.state === 'error' ? ' is-error' : ''}`}
      type="button"
      disabled={disabled}
      onClick={() => void onEnableAudio()}
    >
      <Volume2 aria-hidden="true" />
      {label}
    </button>
  );
}

export function EmulatorWorkspace({
  audioStatus,
  bootComplete,
  canvasRef,
  displayScale,
  framesPerSecond,
  keyboardInputRef,
  message,
  messageTone,
  overBudgetFrames,
  phase,
  onJoystickLinesChange,
  onJoystickRelease,
  onEnableAudio,
  onRetryInitialization,
  programCounter,
  renderP95Ms,
  sampledFrames,
  screenFrameRef,
}: EmulatorWorkspaceProps) {
  const [screenFocused, setScreenFocused] = useState(false);
  const scale = displayScale === '2x' ? 2 : 1;
  const screenStyle: ScreenStyle = {
    '--screen-width': `${PAL_VIDEO_STANDARD.output.width * scale}px`,
  };
  const bootMessage = phase === 'error' ? message : '正在读取 BASIC、KERNAL 与字符 ROM…';
  const performanceText =
    renderP95Ms === undefined
      ? '正在采样帧耗时'
      : `p95 ${renderP95Ms.toFixed(2)} ms / ${(1000 / PAL_VIDEO_STANDARD.timing.refreshRateHz).toFixed(2)} ms 预算`;

  return (
    <div className="emulator-workspace">
      <header className="console-overview">
        <div className="machine-identity">
          <h1>Commodore 64</h1>
          <span className={`machine-status machine-status--${phase}`}>
            <span aria-hidden="true" />
            {PHASE_LABELS[phase]}
          </span>
        </div>

        <dl className="machine-facts" aria-label="主机规格">
          <div>
            <dt>模式</dt>
            <dd>PAL</dd>
          </div>
          <div>
            <dt>内存</dt>
            <dd>64 KB</dd>
          </div>
          <div>
            <dt>C64 RAM</dt>
            <dd>38911 字节可用</dd>
          </div>
        </dl>
      </header>

      <div className="display-area">
        <div
          ref={screenFrameRef}
          className="screen-frame"
          aria-label="C64 屏幕，聚焦后可使用键盘"
          aria-describedby="c64-screen-help"
          aria-busy={phase === 'loading'}
          onBlur={() => setScreenFocused(false)}
          onFocus={() => setScreenFocused(true)}
          onPointerDown={() => focusKeyboardInput(keyboardInputRef)}
        >
          <textarea
            ref={keyboardInputRef}
            className="c64-keyboard-capture"
            aria-label="C64 键盘输入"
            aria-describedby="c64-screen-help"
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            enterKeyHint="enter"
            inputMode="text"
            onInput={clearKeyboardInput}
            rows={1}
            spellCheck={false}
          />
          <div
            className={`screen-host screen-host--${displayScale === 'fit' ? 'fit' : 'fixed'}`}
            style={screenStyle}
          >
            <div className="screen-stack">
              <canvas
                ref={canvasRef}
                className="emulator-canvas"
                aria-label="Commodore 64 视频输出"
                role="img"
              >
                当前浏览器不支持 Canvas，无法显示 C64 画面。
              </canvas>
              <div
                className={`boot-overlay${bootComplete ? ' is-complete' : ''}`}
                aria-hidden={bootComplete}
              >
                <span className="boot-overlay__scan" aria-hidden="true" />
                <p>{bootMessage}</p>
              </div>
            </div>
          </div>
        </div>
        <p id="c64-screen-help" className={`screen-focus-hint${screenFocused ? ' is-active' : ''}`}>
          {screenFocused ? (
            '键盘已接管 · 点击页面其他位置可释放焦点'
          ) : (
            <>
              <span className="screen-focus-hint__desktop">点击屏幕以启用 C64 键盘</span>
              <span className="screen-focus-hint__touch">
                使用下方触控摇杆移动与开火 · 点按屏幕启用键盘
              </span>
            </>
          )}
        </p>
      </div>

      <TouchControls
        disabled={!bootComplete || phase === 'error' || phase === 'loading'}
        onLinesChange={onJoystickLinesChange}
        onRelease={onJoystickRelease}
      />

      <div className={`emulator-feedback${messageTone === 'error' ? ' is-error' : ''}`}>
        <p
          aria-atomic="true"
          aria-live={messageTone === 'error' ? 'assertive' : 'polite'}
          role={messageTone === 'error' ? 'alert' : undefined}
        >
          {message}
        </p>
        {phase === 'error' ? (
          <button
            className="button button--outline-primary initialization-retry"
            type="button"
            onClick={onRetryInitialization}
          >
            <RefreshCw aria-hidden="true" />
            重新初始化
          </button>
        ) : null}
        <AudioStatusControl
          disabled={phase === 'loading' || phase === 'error'}
          onEnableAudio={onEnableAudio}
          status={audioStatus}
        />
        <div className="runtime-telemetry" aria-label="实时执行数据">
          <span>
            PAL {PAL_VIDEO_STANDARD.timing.refreshRateHz.toFixed(2)} Hz · 呈现{' '}
            {framesPerSecond ?? '—'} FPS · {performanceText}
            {sampledFrames > 0 ? ` · 超预算 ${overBudgetFrames}/${sampledFrames}` : ''}
          </span>
          <details>
            <summary>诊断</summary>
            <span>PC ${programCounter}</span>
          </details>
        </div>
      </div>
    </div>
  );
}
