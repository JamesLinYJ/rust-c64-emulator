// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - C64 键盘矩阵
//
//   文件:       C64KeyboardMatrix.ts
//
//   日期:       2026年07月17日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { byte } from '../shared/numbers';

type MatrixPosition = readonly [row: number, column: number];

export interface KeyboardPortInputs {
  readonly portA: number;
  readonly portB: number;
}

export interface KeyboardMatrixPortState {
  readonly dataDirection: number;
  readonly externalInputPins: number;
  readonly outputPins: number;
}

export interface KeyboardMatrixScanState {
  readonly portA: KeyboardMatrixPortState;
  readonly portB: KeyboardMatrixPortState;
}

export type KeyboardMatrixObserver = () => void;

export interface KeyboardMatrixSnapshot {
  readonly rowsByColumn: Uint8Array;
  readonly shiftLockPressed: boolean;
}

const MATRIX_SIDE_LENGTH = 8;
const KEY_MATRIX = new Map<string, MatrixPosition>([
  ['KeyA', [2, 1]],
  ['KeyB', [4, 3]],
  ['KeyC', [4, 2]],
  ['KeyD', [2, 2]],
  ['KeyE', [6, 1]],
  ['KeyF', [5, 2]],
  ['KeyG', [2, 3]],
  ['KeyH', [5, 3]],
  ['KeyI', [1, 4]],
  ['KeyJ', [2, 4]],
  ['KeyK', [5, 4]],
  ['KeyL', [2, 5]],
  ['KeyM', [4, 4]],
  ['KeyN', [7, 4]],
  ['KeyO', [6, 4]],
  ['KeyP', [1, 5]],
  ['KeyQ', [6, 7]],
  ['KeyR', [1, 2]],
  ['KeyS', [5, 1]],
  ['KeyT', [6, 2]],
  ['KeyU', [6, 3]],
  ['KeyV', [7, 3]],
  ['KeyW', [1, 1]],
  ['KeyX', [7, 2]],
  ['KeyY', [1, 3]],
  ['KeyZ', [4, 1]],
  ['Digit0', [3, 4]],
  ['Digit1', [0, 7]],
  ['Digit2', [3, 7]],
  ['Digit3', [0, 1]],
  ['Digit4', [3, 1]],
  ['Digit5', [0, 2]],
  ['Digit6', [3, 2]],
  ['Digit7', [0, 3]],
  ['Digit8', [3, 3]],
  ['Digit9', [0, 4]],
  ['Quote', [5, 5]],
  ['Semicolon', [2, 6]],
  ['Equal', [5, 6]],
  ['Comma', [7, 5]],
  ['Minus', [3, 5]],
  ['Period', [4, 5]],
  ['Slash', [7, 6]],
  ['BracketLeft', [0, 5]],
  ['BracketRight', [1, 6]],
  ['AltLeft', [5, 7]],
  ['AltRight', [5, 7]],
  ['ControlLeft', [2, 7]],
  ['ControlRight', [2, 7]],
  ['Home', [3, 6]],
  ['Enter', [1, 0]],
  ['NumpadEnter', [1, 0]],
  ['Delete', [0, 0]],
  ['Backspace', [0, 0]],
  ['Escape', [7, 7]],
  ['Space', [4, 7]],
  ['ArrowDown', [7, 0]],
  ['ArrowRight', [2, 0]],
  ['ShiftLeft', [7, 1]],
  ['ShiftRight', [4, 6]],
  ['ShiftLock', [7, 1]],
  ['F1', [4, 0]],
  ['F3', [5, 0]],
  ['F5', [6, 0]],
  ['F7', [3, 0]],
]);

/**
 * 8×8 无源键盘矩阵。该对象只保存键开关的闭合状态并求解行列导通关系，
 * 不依赖 DOM，也不把主机按键映射、操纵杆或 RESTORE 混入矩阵职责。
 */
export class C64KeyboardMatrix {
  private readonly pressedRowsByColumn = new Uint8Array(MATRIX_SIDE_LENGTH);
  private readonly pressCounts = new Uint16Array(MATRIX_SIDE_LENGTH * MATRIX_SIDE_LENGTH);
  private readonly pressedCodes = new Set<string>();
  private readonly observers = new Set<KeyboardMatrixObserver>();

  observeChanges(observer: KeyboardMatrixObserver): () => void {
    this.observers.add(observer);
    return () => this.observers.delete(observer);
  }

  supportsKey(code: string): boolean {
    return KEY_MATRIX.has(code);
  }

  snapshot(): KeyboardMatrixSnapshot {
    return {
      rowsByColumn: this.pressedRowsByColumn.slice(),
      shiftLockPressed: this.pressedCodes.has('ShiftLock'),
    };
  }

  releaseAll(): void {
    if (this.pressedCodes.size === 0) return;
    this.pressedCodes.clear();
    this.pressCounts.fill(0);
    this.pressedRowsByColumn.fill(0);
    this.notifyObservers();
  }

  setKeyState(code: string, pressed: boolean): boolean {
    const position = KEY_MATRIX.get(code);
    if (!position) return false;

    const wasPressed = this.pressedCodes.has(code);
    if (wasPressed === pressed) return true;
    if (pressed) this.pressedCodes.add(code);
    else this.pressedCodes.delete(code);

    const [row, column] = position;
    const countIndex = column * MATRIX_SIDE_LENGTH + row;
    const previousCount = this.pressCounts[countIndex] ?? 0;
    const nextCount = pressed ? previousCount + 1 : previousCount - 1;
    if (nextCount < 0) {
      throw new Error(`Keyboard matrix press count became negative for ${code}.`);
    }
    this.pressCounts[countIndex] = nextCount;

    const rowMask = 1 << row;
    const currentRows = this.pressedRowsByColumn[column] ?? 0;
    const nextRows = nextCount > 0 ? currentRows | rowMask : currentRows & ~rowMask;
    if (nextRows === currentRows) return true;
    this.pressedRowsByColumn[column] = nextRows;
    this.notifyObservers();
    return true;
  }

  resolvePortInputs(state: KeyboardMatrixScanState): KeyboardPortInputs {
    const portAState = normalizePortState(state.portA);
    const portBState = normalizePortState(state.portB);
    let portA = portAState.outputPins & portAState.externalInputPins;
    let portB = portBState.outputPins & portBState.externalInputPins;

    for (const component of this.connectedComponents()) {
      const lowPortA = component.columns & ~portA;
      const lowPortB = component.rows & ~portB;
      if (lowPortB !== 0) {
        // CIA1 的 PB 低电平驱动级强于 PA 高电平驱动级，因而反向扫描时低电平占优。
        portA &= ~component.columns;
        portB &= ~component.rows;
        continue;
      }
      if (lowPortA === 0) continue;

      portA &= ~component.columns;
      // PB 输入脚会被 PA 的低电平拉低；PB 高电平输出脚通常能维持自身电平。
      portB &= ~(component.rows & ~portBState.dataDirection);
      for (let row = 0; row < MATRIX_SIDE_LENGTH; row += 1) {
        const rowMask = 1 << row;
        if ((component.rows & rowMask & portBState.dataDirection & portB) === 0) continue;
        if (this.canPortALowOverpowerPortBHigh(row, portA)) portB &= ~rowMask;
      }
    }

    return { portA: byte(portA), portB: byte(portB) };
  }

  private connectedComponents(): readonly { readonly columns: number; readonly rows: number }[] {
    const components: { readonly columns: number; readonly rows: number }[] = [];
    let visitedColumns = 0;
    for (let startColumn = 0; startColumn < MATRIX_SIDE_LENGTH; startColumn += 1) {
      const startMask = 1 << startColumn;
      if (
        (visitedColumns & startMask) !== 0 ||
        (this.pressedRowsByColumn[startColumn] ?? 0) === 0
      ) {
        continue;
      }

      let columns = startMask;
      let rows = 0;
      let changed = true;
      while (changed) {
        const previousColumns = columns;
        const previousRows = rows;
        for (let column = 0; column < MATRIX_SIDE_LENGTH; column += 1) {
          if ((columns & (1 << column)) !== 0) {
            rows |= this.pressedRowsByColumn[column] ?? 0;
          }
        }
        for (let column = 0; column < MATRIX_SIDE_LENGTH; column += 1) {
          if (((this.pressedRowsByColumn[column] ?? 0) & rows) !== 0) columns |= 1 << column;
        }
        changed = columns !== previousColumns || rows !== previousRows;
      }
      visitedColumns |= columns;
      components.push({ columns, rows });
    }
    return components;
  }

  private canPortALowOverpowerPortBHigh(row: number, portA: number): boolean {
    const rowMask = 1 << row;
    let groundedConnectionCount = 0;
    for (let column = 0; column < MATRIX_SIDE_LENGTH; column += 1) {
      const columnMask = 1 << column;
      if (((this.pressedRowsByColumn[column] ?? 0) & rowMask) !== 0 && (portA & columnMask) === 0) {
        groundedConnectionCount += 1;
      }
    }
    if (groundedConnectionCount >= 2) return true;

    const shiftLockPosition = KEY_MATRIX.get('ShiftLock');
    if (!shiftLockPosition || !this.pressedCodes.has('ShiftLock')) return false;
    const [shiftLockRow, shiftLockColumn] = shiftLockPosition;
    return row === shiftLockRow && (portA & (1 << shiftLockColumn)) === 0;
  }

  private notifyObservers(): void {
    for (const observer of [...this.observers]) observer();
  }
}

function normalizePortState(state: KeyboardMatrixPortState): KeyboardMatrixPortState {
  return {
    dataDirection: byte(state.dataDirection),
    externalInputPins: byte(state.externalInputPins),
    outputPins: byte(state.outputPins),
  };
}
