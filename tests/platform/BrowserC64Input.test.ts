// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - 浏览器输入适配器测试
//
//   文件:       BrowserC64Input.test.ts
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

// @vitest-environment jsdom

import { describe, expect, it, vi } from 'vitest';

import { C64KeyboardMatrix } from '../../src/devices/C64KeyboardMatrix';
import {
  RESTORE_NMI_PULSE_CYCLES,
  RestoreKeyNmiCircuit,
} from '../../src/devices/RestoreKeyNmiCircuit';
import { BrowserC64Input } from '../../src/platform/BrowserC64Input';
import {
  C64_CONTROL_PORT_DIGITAL_LINE,
  C64ControlPorts,
} from '../../src/peripherals/control/C64ControlPorts';

function createInput(
  joystickPort: 1 | 2 | null = 2,
  target: EventTarget = new EventTarget(),
): {
  readonly controlPorts: C64ControlPorts;
  readonly input: BrowserC64Input;
  readonly keyboard: C64KeyboardMatrix;
  readonly restore: RestoreKeyNmiCircuit;
  readonly target: EventTarget;
} {
  const controlPorts = new C64ControlPorts();
  const keyboard = new C64KeyboardMatrix();
  const restore = new RestoreKeyNmiCircuit();
  const input = new BrowserC64Input({
    controlPorts,
    joystickPort,
    keyboard,
    restoreKeyInput: restore,
  });
  input.attach(target);
  return { controlPorts, input, keyboard, restore, target };
}

function scanPortB(keyboard: C64KeyboardMatrix, portAOutput: number): number {
  return keyboard.resolvePortInputs({
    portA: {
      dataDirection: 0xff,
      externalInputPins: 0xff,
      outputPins: portAOutput,
    },
    portB: {
      dataDirection: 0x00,
      externalInputPins: 0xff,
      outputPins: 0xff,
    },
  }).portB;
}

describe('BrowserC64Input', () => {
  it('maps a direction key to only the selected joystick port', () => {
    const { controlPorts, input, keyboard, target } = createInput(2);

    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'ArrowUp' }));

    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(
      C64_CONTROL_PORT_DIGITAL_LINE.up,
    );
    expect(scanPortB(keyboard, 0xfe)).toBe(0xff);

    target.dispatchEvent(new KeyboardEvent('keyup', { code: 'ArrowUp' }));
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    input.dispose();
  });

  it('synthesizes shifted C64 keys when joystick keyboard mapping is disabled', () => {
    const { input, keyboard, target } = createInput(null);

    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'ArrowUp' }));
    expect(scanPortB(keyboard, 0xfe)).toBe(0x7f);
    expect(scanPortB(keyboard, 0xfd)).toBe(0x7f);

    target.dispatchEvent(new KeyboardEvent('keyup', { code: 'ArrowUp' }));
    expect(scanPortB(keyboard, 0xfe)).toBe(0xff);
    expect(scanPortB(keyboard, 0xfd)).toBe(0xff);
    input.dispose();
  });

  it('reference-counts host keys that share one physical matrix switch', () => {
    const { input, keyboard, target } = createInput(null);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'AltLeft' }));
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'AltRight' }));
    target.dispatchEvent(new KeyboardEvent('keyup', { code: 'AltLeft' }));

    expect(scanPortB(keyboard, 0x7f)).toBe(0xdf);

    target.dispatchEvent(new KeyboardEvent('keyup', { code: 'AltRight' }));
    expect(scanPortB(keyboard, 0x7f)).toBe(0xff);
    input.dispose();
  });

  it('maps Caps Lock to the mechanically latched C64 Shift Lock switch', () => {
    const { input, keyboard, target } = createInput(null);

    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'CapsLock' }));
    target.dispatchEvent(new KeyboardEvent('keyup', { code: 'CapsLock' }));
    expect(scanPortB(keyboard, 0xfd)).toBe(0x7f);

    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'CapsLock' }));
    target.dispatchEvent(new KeyboardEvent('keyup', { code: 'CapsLock' }));
    expect(scanPortB(keyboard, 0xfd)).toBe(0xff);

    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'CapsLock' }));
    input.detach();
    expect(scanPortB(keyboard, 0xfd)).toBe(0xff);
    input.dispose();
  });

  it('routes PageUp edges to RESTORE without retriggering key-repeat events', () => {
    const { input, restore, target } = createInput(null);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'PageUp' }));
    expect(restore.nmiAsserted).toBe(true);

    restore.tick(RESTORE_NMI_PULSE_CYCLES);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'PageUp', repeat: true }));
    expect(restore.nmiAsserted).toBe(false);

    target.dispatchEvent(new KeyboardEvent('keyup', { code: 'PageUp' }));
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'PageUp' }));
    expect(restore.nmiAsserted).toBe(true);
    input.dispose();
  });

  it('releases held lines before moving the browser joystick to another port', () => {
    const { controlPorts, input, target } = createInput(2);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'Space' }));
    input.setJoystickSourceLines(
      91,
      C64_CONTROL_PORT_DIGITAL_LINE.up | C64_CONTROL_PORT_DIGITAL_LINE.left,
    );

    input.setActiveJoystickPort(1);

    expect(controlPorts.port2.deviceAttached).toBe(false);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    expect(controlPorts.port1.deviceAttached).toBe(true);
    expect(controlPorts.port1.deviceSignals.groundedDigitalLines).toBe(0);
    expect(input.releaseJoystickSource(91)).toBe(false);
    input.dispose();
  });

  it('merges independent direct joystick source masks until each source is released', () => {
    const { controlPorts, input } = createInput(2);
    const upLeft = C64_CONTROL_PORT_DIGITAL_LINE.up | C64_CONTROL_PORT_DIGITAL_LINE.left;

    expect(input.setJoystickSourceLines(11, upLeft)).toBe(true);
    expect(input.setJoystickSourceLines(12, C64_CONTROL_PORT_DIGITAL_LINE.fire)).toBe(true);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(
      upLeft | C64_CONTROL_PORT_DIGITAL_LINE.fire,
    );

    expect(input.releaseJoystickSource(12)).toBe(true);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(upLeft);
    expect(input.releaseJoystickSource(12)).toBe(false);
    expect(input.releaseJoystickSource(11)).toBe(true);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    input.dispose();
  });

  it('atomically slides one direct joystick source between diagonal masks', () => {
    const { controlPorts, input } = createInput(2);
    const upLeft = C64_CONTROL_PORT_DIGITAL_LINE.up | C64_CONTROL_PORT_DIGITAL_LINE.left;
    const upRight = C64_CONTROL_PORT_DIGITAL_LINE.up | C64_CONTROL_PORT_DIGITAL_LINE.right;

    input.setJoystickSourceLines(27, upLeft);
    input.setJoystickSourceLines(27, upRight);

    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(upRight);
    input.releaseJoystickSource(27);
    input.dispose();
  });

  it('rejects direct joystick input when no joystick port is attached', () => {
    const { input } = createInput(null);

    expect(input.setJoystickSourceLines(1, C64_CONTROL_PORT_DIGITAL_LINE.fire)).toBe(false);
    expect(input.releaseJoystickSource(1)).toBe(false);
    input.dispose();
  });

  it('rejects bits outside the five control-port digital lines', () => {
    const { input } = createInput(2);

    expect(() => input.setJoystickSourceLines(1, 0x20)).toThrow(RangeError);
    expect(() => input.setJoystickSourceLines(1, 1.5)).toThrow(RangeError);
    input.dispose();
  });

  it('releases keyboard, joystick and RESTORE bindings when the screen loses focus', () => {
    const screen = document.createElement('div');
    const { controlPorts, input, keyboard, restore, target } = createInput(2, screen);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyA' }));
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'ArrowUp' }));
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'PageUp' }));
    restore.tick(RESTORE_NMI_PULSE_CYCLES);

    expect(scanPortB(keyboard, 0xfd)).toBe(0xfb);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(
      C64_CONTROL_PORT_DIGITAL_LINE.up,
    );

    target.dispatchEvent(new FocusEvent('blur'));

    expect(scanPortB(keyboard, 0xfd)).toBe(0xff);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'PageUp' }));
    expect(restore.nmiAsserted).toBe(true);
    input.dispose();
  });

  it('preserves direct joystick sources when only the keyboard target loses focus', () => {
    const screen = document.createElement('div');
    const { controlPorts, input, target } = createInput(2, screen);
    const downRight = C64_CONTROL_PORT_DIGITAL_LINE.down | C64_CONTROL_PORT_DIGITAL_LINE.right;
    input.setJoystickSourceLines(31, downRight);

    target.dispatchEvent(new FocusEvent('blur'));

    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(downRight);
    expect(input.releaseJoystickSource(31)).toBe(true);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    input.dispose();
  });

  it('releases every direct joystick source when detached from its keyboard target', () => {
    const screen = document.createElement('div');
    const { controlPorts, input } = createInput(2, screen);
    input.setJoystickSourceLines(
      41,
      C64_CONTROL_PORT_DIGITAL_LINE.down | C64_CONTROL_PORT_DIGITAL_LINE.fire,
    );

    input.detach();

    expect(input.isAttached).toBe(false);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    expect(input.releaseJoystickSource(41)).toBe(false);
    input.dispose();
  });

  it('releases active host bindings when the containing window loses focus', () => {
    const screen = document.createElement('div');
    const { controlPorts, input, keyboard, target } = createInput(2, screen);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyA' }));
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'ArrowLeft' }));
    input.setJoystickSourceLines(44, C64_CONTROL_PORT_DIGITAL_LINE.fire);

    window.dispatchEvent(new FocusEvent('blur'));

    expect(scanPortB(keyboard, 0xfd)).toBe(0xff);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    expect(input.releaseJoystickSource(44)).toBe(false);
    input.dispose();
  });

  it('releases active host bindings only when the document becomes hidden', () => {
    const visibilityState = vi.spyOn(document, 'visibilityState', 'get');
    const { controlPorts, input, keyboard, target } = createInput(2, document);
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyA' }));
    target.dispatchEvent(new KeyboardEvent('keydown', { code: 'ArrowRight' }));
    input.setJoystickSourceLines(45, C64_CONTROL_PORT_DIGITAL_LINE.fire);

    visibilityState.mockReturnValue('visible');
    document.dispatchEvent(new Event('visibilitychange'));
    expect(scanPortB(keyboard, 0xfd)).toBe(0xfb);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(
      C64_CONTROL_PORT_DIGITAL_LINE.right | C64_CONTROL_PORT_DIGITAL_LINE.fire,
    );

    visibilityState.mockReturnValue('hidden');
    document.dispatchEvent(new Event('visibilitychange'));
    expect(scanPortB(keyboard, 0xfd)).toBe(0xff);
    expect(controlPorts.port2.deviceSignals.groundedDigitalLines).toBe(0);
    expect(input.releaseJoystickSource(45)).toBe(false);

    visibilityState.mockRestore();
    input.dispose();
  });

  it('turns mobile beforeinput text into a complete C64 matrix pulse', () => {
    vi.useFakeTimers();
    const target = document.createElement('textarea');
    const { input, keyboard } = createInput(null, target);
    try {
      const event = new InputEvent('beforeinput', {
        bubbles: true,
        cancelable: true,
        data: 'a',
        inputType: 'insertText',
      });

      expect(target.dispatchEvent(event)).toBe(false);
      expect(event.defaultPrevented).toBe(true);
      expect(scanPortB(keyboard, 0xfd)).toBe(0xfb);

      vi.runAllTimers();
      expect(scanPortB(keyboard, 0xfd)).toBe(0xff);
    } finally {
      input.dispose();
      vi.useRealTimers();
    }
  });

  it('uses C64 shifted chords for punctuation produced by a soft keyboard', () => {
    vi.useFakeTimers();
    const target = document.createElement('textarea');
    const { input, keyboard } = createInput(null, target);
    try {
      target.dispatchEvent(
        new InputEvent('beforeinput', {
          bubbles: true,
          cancelable: true,
          data: '?',
          inputType: 'insertText',
        }),
      );

      expect(scanPortB(keyboard, 0xfd)).toBe(0x7f);
      expect(scanPortB(keyboard, 0xbf)).toBe(0x7f);
      vi.runAllTimers();
      expect(scanPortB(keyboard, 0xfd)).toBe(0xff);
      expect(scanPortB(keyboard, 0xbf)).toBe(0xff);
    } finally {
      input.dispose();
      vi.useRealTimers();
    }
  });

  it('queues mobile Enter and Delete operations without accepting intermediate composition', () => {
    vi.useFakeTimers();
    const target = document.createElement('textarea');
    const { input, keyboard } = createInput(null, target);
    try {
      const composition = new InputEvent('beforeinput', {
        bubbles: true,
        cancelable: true,
        data: 'a',
        inputType: 'insertCompositionText',
        isComposing: true,
      });
      target.dispatchEvent(composition);
      expect(composition.defaultPrevented).toBe(false);
      expect(scanPortB(keyboard, 0xfd)).toBe(0xff);

      target.dispatchEvent(
        new InputEvent('beforeinput', {
          bubbles: true,
          cancelable: true,
          inputType: 'insertLineBreak',
        }),
      );
      target.dispatchEvent(
        new InputEvent('beforeinput', {
          bubbles: true,
          cancelable: true,
          inputType: 'deleteContentBackward',
        }),
      );
      expect(scanPortB(keyboard, 0xfe)).toBe(0xfd);

      vi.advanceTimersToNextTimer();
      expect(scanPortB(keyboard, 0xfe)).toBe(0xff);
      vi.advanceTimersToNextTimer();
      expect(scanPortB(keyboard, 0xfe)).toBe(0xfe);
      vi.runAllTimers();
      expect(scanPortB(keyboard, 0xfe)).toBe(0xff);
    } finally {
      input.dispose();
      vi.useRealTimers();
    }
  });
});
