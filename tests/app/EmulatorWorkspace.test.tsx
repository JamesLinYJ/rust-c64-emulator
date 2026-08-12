// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - 移动端键盘捕获界面测试
//
//   文件:       EmulatorWorkspace.test.tsx
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

// @vitest-environment jsdom

import { act, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { EmulatorWorkspace } from '../../src/app/components/EmulatorWorkspace';

function Harness() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const keyboardInputRef = useRef<HTMLTextAreaElement>(null);
  const screenFrameRef = useRef<HTMLDivElement>(null);

  return (
    <EmulatorWorkspace
      audioOverrunSamples={0}
      audioStatus={{ state: 'inactive' }}
      audioUnderrunSamples={0}
      bootComplete
      canvasRef={canvasRef}
      displayScale="fit"
      framesPerSecond={50}
      keyboardInputRef={keyboardInputRef}
      message="BASIC 已就绪。"
      messageTone="normal"
      onEnableAudio={() => Promise.resolve()}
      onJoystickLinesChange={vi.fn()}
      onJoystickRelease={vi.fn()}
      onRetryInitialization={vi.fn()}
      overBudgetFrames={0}
      phase="running"
      programCounter="E5CD"
      renderP95Ms={1}
      sampledFrames={120}
      screenFrameRef={screenFrameRef}
      videoStandard="pal"
    />
  );
}

describe('EmulatorWorkspace mobile keyboard capture', () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="test-root"></div>';
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  });

  it('focuses a text input from a direct screen gesture and keeps its buffer empty', () => {
    const container = document.querySelector<HTMLElement>('#test-root');
    if (!container) throw new Error('Test root did not mount.');
    const root = createRoot(container);
    act(() => root.render(<Harness />));

    const screen = container.querySelector<HTMLElement>('.screen-frame');
    const keyboardInput = container.querySelector<HTMLTextAreaElement>('.c64-keyboard-capture');
    expect(screen).not.toBeNull();
    expect(keyboardInput).not.toBeNull();
    if (!screen || !keyboardInput) throw new Error('Keyboard capture controls did not mount.');

    act(() => {
      screen.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    });
    expect(document.activeElement).toBe(keyboardInput);
    expect(keyboardInput.inputMode).toBe('text');
    expect(keyboardInput.getAttribute('autocapitalize')).toBe('none');

    keyboardInput.value = 'a';
    act(() => {
      keyboardInput.dispatchEvent(new InputEvent('input', { bubbles: true, data: 'a' }));
    });
    expect(keyboardInput.value).toBe('');

    act(() => root.unmount());
  });
});
