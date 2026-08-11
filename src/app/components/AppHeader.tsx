import { Maximize2, Minimize2, Moon, Sun } from 'lucide-react';

import type { EmulatorPhase } from '../useC64Emulator';
import { C64_VIDEO_STANDARDS, type C64VideoStandard } from '../../video/C64VideoStandard';

interface AppHeaderProps {
  readonly darkTheme: boolean;
  readonly isFullscreen: boolean;
  readonly onToggleFullscreen: () => void;
  readonly onToggleTheme: () => void;
  readonly phase: EmulatorPhase;
  readonly videoStandard: C64VideoStandard;
}

const PHASE_LABELS: Readonly<Record<EmulatorPhase, string>> = {
  error: '异常',
  loading: '启动中',
  paused: '已暂停',
  running: '运行中',
};

export function AppHeader({
  darkTheme,
  isFullscreen,
  onToggleFullscreen,
  onToggleTheme,
  phase,
  videoStandard,
}: AppHeaderProps) {
  const video = C64_VIDEO_STANDARDS[videoStandard];
  return (
    <header className="app-header">
      <div className="app-header__inner">
        <a className="app-title" href="#console" aria-label="返回运行控制台">
          <strong>RetroC64</strong>
          <span>{video.label} · Rust/Wasm Worker 模拟器</span>
        </a>

        <div className="header-actions">
          <span className={`global-status global-status--${phase}`} aria-live="polite">
            <span aria-hidden="true" />
            {PHASE_LABELS[phase]}
          </span>
          <button
            className="icon-button"
            type="button"
            aria-label={isFullscreen ? '退出全屏' : '全屏显示 C64'}
            aria-pressed={isFullscreen}
            onClick={onToggleFullscreen}
          >
            {isFullscreen ? <Minimize2 aria-hidden="true" /> : <Maximize2 aria-hidden="true" />}
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={darkTheme ? '切换到浅色主题' : '切换到深色主题'}
            aria-pressed={darkTheme}
            onClick={onToggleTheme}
          >
            {darkTheme ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
          </button>
        </div>
      </div>
    </header>
  );
}
