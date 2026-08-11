// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - 浏览器应用组合入口
//
//   文件:       App.tsx
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { useCallback, useEffect, useRef, useState } from 'react';

import { AppHeader } from './components/AppHeader';
import { ControlPanel } from './components/ControlPanel';
import { EmulatorWorkspace, type DisplayScale } from './components/EmulatorWorkspace';
import type { BundledProgramDescriptor } from '../media/BundledProgramCatalog';
import { useC64Emulator } from './useC64Emulator';

export function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const keyboardInputRef = useRef<HTMLTextAreaElement>(null);
  const screenFrameRef = useRef<HTMLDivElement>(null);
  const [displayScale, setDisplayScale] = useState<DisplayScale>('fit');
  const [darkTheme, setDarkTheme] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const emulator = useC64Emulator(canvasRef, keyboardInputRef);
  const { loadBuiltInProgram, loadLocalProgram, reset, toggle } = emulator;

  const focusScreen = useCallback((): void => {
    keyboardInputRef.current?.focus({ preventScroll: true });
  }, []);

  const handleLoadProgram = useCallback(
    async (program: BundledProgramDescriptor): Promise<void> => {
      if (await loadBuiltInProgram(program)) focusScreen();
    },
    [focusScreen, loadBuiltInProgram],
  );

  const handleLoadFile = useCallback(
    async (file: File): Promise<void> => {
      if (await loadLocalProgram(file)) focusScreen();
    },
    [focusScreen, loadLocalProgram],
  );

  const handleRun = useCallback((): void => {
    if (emulator.phase === 'paused') toggle();
  }, [emulator.phase, toggle]);

  const handlePause = useCallback((): void => {
    if (emulator.phase === 'running') toggle();
  }, [emulator.phase, toggle]);

  const toggleTheme = useCallback((): void => {
    setDarkTheme((current) => !current);
  }, []);

  const toggleFullscreen = useCallback((): void => {
    const screenFrame = screenFrameRef.current;
    if (!screenFrame || !document.fullscreenEnabled) return;
    const operation = document.fullscreenElement
      ? document.exitFullscreen()
      : screenFrame.requestFullscreen();
    void operation.catch(() => undefined);
  }, []);

  useEffect(() => {
    const handleFullscreenChange = (): void => {
      setIsFullscreen(document.fullscreenElement === screenFrameRef.current);
    };
    document.addEventListener('fullscreenchange', handleFullscreenChange);
    return () => document.removeEventListener('fullscreenchange', handleFullscreenChange);
  }, []);

  return (
    <div className={`app-page${darkTheme ? ' app-page--dark' : ''}`}>
      <AppHeader
        darkTheme={darkTheme}
        isFullscreen={isFullscreen}
        onToggleFullscreen={toggleFullscreen}
        onToggleTheme={toggleTheme}
        phase={emulator.phase}
      />

      <main className="dashboard" aria-label="TypeScript Commodore 64 Emulator 运行控制台">
        <section id="console" className="console-card" aria-label="Commodore 64 主机">
          <EmulatorWorkspace
            audioStatus={emulator.audioStatus}
            bootComplete={emulator.bootComplete}
            canvasRef={canvasRef}
            framesPerSecond={emulator.framesPerSecond}
            keyboardInputRef={keyboardInputRef}
            message={emulator.message}
            messageTone={emulator.messageTone}
            overBudgetFrames={emulator.overBudgetFrames}
            phase={emulator.phase}
            onJoystickLinesChange={emulator.setJoystickSourceLines}
            onJoystickRelease={emulator.releaseJoystickSource}
            onEnableAudio={emulator.enableAudio}
            onRetryInitialization={emulator.retryInitialization}
            programCounter={emulator.programCounter}
            renderP95Ms={emulator.renderP95Ms}
            sampledFrames={emulator.sampledFrames}
            screenFrameRef={screenFrameRef}
            displayScale={displayScale}
          />
        </section>

        <ControlPanel
          displayScale={displayScale}
          isReady={emulator.isReady}
          onLoadFile={handleLoadFile}
          onLoadProgram={handleLoadProgram}
          onPause={handlePause}
          onReset={reset}
          onRun={handleRun}
          onScaleChange={setDisplayScale}
          onStepFrame={emulator.stepFrame}
          phase={emulator.phase}
        />
      </main>

      <footer className="app-footer">
        <span>PAL 硬件模型 · MIT License</span>
        <a
          href="https://github.com/JamesLinYJ/typescript-commodore-64-emulator"
          target="_blank"
          rel="noreferrer"
        >
          JamesLinYJ 原始项目
        </a>
      </footer>
    </div>
  );
}
