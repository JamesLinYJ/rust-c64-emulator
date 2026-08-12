// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 寄存器与芯片状态
//
//   文件:       VicII.ts
//
//   日期:       2026年07月17日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { byte } from '../shared/numbers';
import { packRgbaPixel } from '../shared/RgbaPixel';
import { IoDevice } from './IoDevice';
import { VicBorderController } from './VicBorderController';
import {
  createVicCycleResultBuffer,
  VicCycleSequencer,
  type VicCycleResult,
  type VicCycleResultBuffer,
  type VicCycleSignals,
} from './VicCycleSequencer';
import { VicSprite } from './VicSprite';
import { VicFetchPipeline, type VicFetchSnapshot } from './VicFetchPipeline';
import type { VicMemoryBus } from './VicMemoryBus';
import { VicPixelPipeline } from './VicPixelPipeline';
import { PAL_VIC_TIMING } from './VicTiming';
import {
  VIC_DISPLAY,
  VIC_BACKGROUND_COLOR_COUNT,
  VIC_INTERRUPT_BIT,
  VIC_MASK,
  VIC_REGISTER,
  VIC_REGISTER_COUNT,
  VIC_REGISTER_READ_MASK,
  VIC_SCREEN_CONTROL_1_BIT,
  VIC_SCREEN_CONTROL_2_BIT,
  VIC_SPRITE_COUNT,
} from './vicRegisters';

export const C64_COLOR = {
  black: packRgbaPixel(0x00, 0x00, 0x00),
  white: packRgbaPixel(0xff, 0xff, 0xff),
  red: packRgbaPixel(0xe0, 0x40, 0x40),
  cyan: packRgbaPixel(0x60, 0xff, 0xff),
  purple: packRgbaPixel(0xe0, 0x60, 0xe0),
  green: packRgbaPixel(0x40, 0xe0, 0x40),
  blue: packRgbaPixel(0x40, 0x40, 0xe0),
  yellow: packRgbaPixel(0xff, 0xff, 0x40),
  orange: packRgbaPixel(0xe0, 0xa0, 0x40),
  brown: packRgbaPixel(0x9c, 0x74, 0x48),
  lightRed: packRgbaPixel(0xff, 0xa0, 0xa0),
  darkGray: packRgbaPixel(0x54, 0x54, 0x54),
  gray: packRgbaPixel(0x88, 0x88, 0x88),
  lightGreen: packRgbaPixel(0xa0, 0xff, 0xa0),
  lightBlue: packRgbaPixel(0xa0, 0xa0, 0xff),
  lightGray: packRgbaPixel(0xc0, 0xc0, 0xc0),
} as const;

export const C64_PALETTE = [
  C64_COLOR.black,
  C64_COLOR.white,
  C64_COLOR.red,
  C64_COLOR.cyan,
  C64_COLOR.purple,
  C64_COLOR.green,
  C64_COLOR.blue,
  C64_COLOR.yellow,
  C64_COLOR.orange,
  C64_COLOR.brown,
  C64_COLOR.lightRed,
  C64_COLOR.darkGray,
  C64_COLOR.gray,
  C64_COLOR.lightGreen,
  C64_COLOR.lightBlue,
  C64_COLOR.lightGray,
] as const;

export interface VicRasterLineEvent {
  readonly frameCompleted: boolean;
  readonly rasterLine: number;
}

export type VicRasterLineObserver = (event: VicRasterLineEvent) => void;

export interface VicRasterLineSnapshot {
  readonly borderColors: Uint32Array;
  readonly borderPixelMasks: Uint8Array;
  readonly fetchState: VicFetchSnapshot;
  readonly pixels: Uint32Array;
}

const DEFAULT_SPRITE_COLORS = [
  C64_COLOR.white,
  C64_COLOR.red,
  C64_COLOR.cyan,
  C64_COLOR.purple,
  C64_COLOR.green,
  C64_COLOR.blue,
  C64_COLOR.yellow,
  C64_COLOR.gray,
] as const;

export class VicII extends IoDevice implements VicCycleSignals {
  readonly palette = C64_PALETTE;
  readonly backgroundColors = Array<number>(VIC_BACKGROUND_COLOR_COUNT).fill(C64_COLOR.black);
  readonly sprites = Array.from(
    { length: VIC_SPRITE_COUNT },
    (_, index) => new VicSprite(DEFAULT_SPRITE_COLORS[index] ?? C64_COLOR.white),
  );

  characterMemoryAddress = 0;
  bitmapMemoryAddress = 0;
  screenMemoryAddress = 0;
  verticalScroll = 0;
  horizontalScroll = 0;
  screenHeight: 24 | 25 = VIC_DISPLAY.standardRowCount;
  screenWidth: 38 | 40 = VIC_DISPLAY.standardColumnCount;
  screenVisible = false;
  rasterTrigger = 0;
  displayMode = 0;
  displayModeValid = true;
  bitmapMode = false;
  multicolorMode = false;
  extendedBackgroundMode = false;
  borderColor = this.paletteColor(0);
  spritesEnabled = false;
  spriteMulticolor0 = this.paletteColor(4);
  spriteMulticolor1 = this.paletteColor(0);

  private readonly cycleSequencer = new VicCycleSequencer();
  private readonly cycleResult = createVicCycleResultBuffer();
  private readonly borderController = new VicBorderController();
  private readonly fetchPipeline = new VicFetchPipeline();
  private readonly pixelPipeline = new VicPixelPipeline();
  private readonly lineBorderColors = new Uint32Array(PAL_VIC_TIMING.cyclesPerRasterLine);
  private readonly lineBorderPixelMasks = new Uint8Array(PAL_VIC_TIMING.cyclesPerRasterLine);
  private readonly rasterLineObservers = new Set<VicRasterLineObserver>();
  private rasterPosition = 0;
  private rasterInterruptMatched = false;
  private interruptMask = 0;
  private interruptLatches = 0;
  private spriteSpriteCollision = 0;
  private spriteForegroundCollision = 0;
  private lightPenLatched = false;
  private lightPenInputHigh = true;
  private lightPenTriggerCyclesRemaining = 0;

  constructor(debug = false) {
    super('VIC-II', VIC_REGISTER_COUNT, debug);
    this.installRegisterMap();
    this.reset();
  }

  reset(): void {
    this.registers.fill(0);
    this.backgroundColors.fill(C64_COLOR.black);
    for (let index = 0; index < this.sprites.length; index += 1) {
      this.sprites[index]?.reset(DEFAULT_SPRITE_COLORS[index] ?? C64_COLOR.white);
    }
    this.characterMemoryAddress = 0;
    this.bitmapMemoryAddress = 0;
    this.screenMemoryAddress = 0;
    this.verticalScroll = 0;
    this.horizontalScroll = 0;
    this.screenHeight = VIC_DISPLAY.standardRowCount;
    this.screenWidth = VIC_DISPLAY.standardColumnCount;
    this.screenVisible = false;
    this.rasterTrigger = 0;
    this.displayMode = 0;
    this.displayModeValid = true;
    this.bitmapMode = false;
    this.multicolorMode = false;
    this.extendedBackgroundMode = false;
    this.borderColor = this.paletteColor(0);
    this.spritesEnabled = false;
    this.spriteMulticolor0 = this.paletteColor(4);
    this.spriteMulticolor1 = this.paletteColor(0);
    this.rasterPosition = 0;
    this.rasterInterruptMatched = false;
    this.interruptMask = 0;
    this.interruptLatches = 0;
    this.spriteSpriteCollision = 0;
    this.spriteForegroundCollision = 0;
    this.lightPenLatched = false;
    this.lightPenTriggerCyclesRemaining = this.lightPenInputHigh
      ? 0
      : PAL_VIC_TIMING.lightPen.triggerDelayCycles;
    this.cycleSequencer.reset();
    this.borderController.reset();
    this.fetchPipeline.reset();
    this.pixelPipeline.reset(this.borderColor);
    this.lineBorderColors.fill(this.borderColor);
    this.lineBorderPixelMasks.fill(0xff);
  }

  get baLow(): boolean {
    return this.cycleSequencer.baLow;
  }

  get aecLow(): boolean {
    return this.cycleSequencer.aecLow;
  }

  get badLine(): boolean {
    return this.cycleSequencer.badLine;
  }

  get currentRasterCycle(): number {
    return this.cycleSequencer.cycle;
  }

  get currentRasterLine(): number {
    return this.rasterPosition;
  }

  get displayEnabled(): boolean {
    return this.screenVisible;
  }

  get spriteDmaMask(): number {
    return this.cycleSequencer.spriteDmaMask;
  }

  get phi1DataBusValue(): number {
    return this.fetchPipeline.phi1DataBusValue;
  }

  get spriteEnableMask(): number {
    return this.registers[VIC_REGISTER.spriteEnable] ?? 0;
  }

  get spriteVerticalExpansionMask(): number {
    return this.registers[VIC_REGISTER.spriteExpandVertical] ?? 0;
  }

  spriteY(spriteIndex: number): number {
    return this.sprites[spriteIndex]?.y ?? 0;
  }

  tickCycle(memory: VicMemoryBus): VicCycleResult {
    const result = this.advanceCycle(memory);
    return {
      ...result,
      matrixAccess: result.matrixAccess === undefined ? undefined : { ...result.matrixAccess },
      spriteDataOffsets: { ...result.spriteDataOffsets },
    };
  }

  /** 推进一个芯片周期但不生成可保留的诊断快照，供整机主循环使用。 */
  clockCycle(memory: VicMemoryBus): void {
    this.advanceCycle(memory);
  }

  private advanceCycle(memory: VicMemoryBus): VicCycleResultBuffer {
    const result = this.cycleResult;
    this.cycleSequencer.tickInto(this, result);
    this.fetchPipeline.executeCycle(result, this, memory);
    const cycleIndex = result.cycle - 1;
    const borderPixelMask = this.borderController.tickCycle(
      this.screenWidth === VIC_DISPLAY.standardColumnCount,
      this.screenVisible,
      result.cycle,
      result.rasterLine,
      this.screenHeight === VIC_DISPLAY.standardRowCount,
    );
    this.lineBorderPixelMasks[cycleIndex] = borderPixelMask;
    this.lineBorderColors[cycleIndex] = this.borderColor;
    this.pixelPipeline.clockCycle(result, borderPixelMask, this, this.fetchPipeline, this);
    this.rasterPosition = result.rasterLine;
    if (result.frameStarted) {
      this.lightPenLatched = false;
      if (!this.lightPenInputHigh) {
        this.lightPenTriggerCyclesRemaining = PAL_VIC_TIMING.lightPen.triggerDelayCycles;
      }
    }
    this.clockLightPenTrigger(result.cycle, result.rasterLine);
    this.updateRasterInterruptComparison();
    if (result.completedRasterLine !== undefined) {
      this.notifyRasterLineObservers(result.completedRasterLine);
    }
    return result;
  }

  /** 将当前完整光栅线直接复制到调用方缓冲区；显示热路径不需要构造诊断快照。 */
  copyRasterLinePixelsTo(target: Uint32Array, targetOffset: number): void {
    this.pixelPipeline.copyPixelsTo(target, targetOffset);
  }

  captureRasterLineState(): VicRasterLineSnapshot {
    return {
      borderColors: this.lineBorderColors.slice(),
      borderPixelMasks: this.lineBorderPixelMasks.slice(),
      fetchState: this.fetchPipeline.snapshot(),
      pixels: this.pixelPipeline.snapshot(),
    };
  }

  observeRasterLines(observer: VicRasterLineObserver): () => void {
    this.rasterLineObservers.add(observer);
    return () => this.rasterLineObservers.delete(observer);
  }

  isRasterInterruptPending(): boolean {
    return this.interruptPending;
  }

  get interruptPending(): boolean {
    return (this.interruptLatches & this.interruptMask & VIC_MASK.interruptSources) !== 0;
  }

  recordSpriteSpriteCollision(spriteMask: number): void {
    const normalizedMask = byte(spriteMask);
    if (normalizedMask === 0) return;
    const registerWasEmpty = this.spriteSpriteCollision === 0;
    this.spriteSpriteCollision |= normalizedMask;
    // 碰撞 IRQ 检测的是寄存器 0→非 0 的边沿；新增位不会在未读清寄存器时再次触发。
    if (registerWasEmpty) this.interruptLatches |= VIC_INTERRUPT_BIT.spriteSpriteCollision;
    this.forEachSpriteBit(normalizedMask, (sprite, set) => {
      if (set) sprite.collisionWithSprite = true;
    });
  }

  recordSpriteForegroundCollision(spriteMask: number): void {
    const normalizedMask = byte(spriteMask);
    if (normalizedMask === 0) return;
    const registerWasEmpty = this.spriteForegroundCollision === 0;
    this.spriteForegroundCollision |= normalizedMask;
    if (registerWasEmpty) this.interruptLatches |= VIC_INTERRUPT_BIT.spriteForegroundCollision;
    this.forEachSpriteBit(normalizedMask, (sprite, set) => {
      if (set) sprite.collisionWithForeground = true;
    });
  }

  latchLightPen(x: number, y: number): void {
    if (this.lightPenLatched) return;
    this.lightPenLatched = true;
    this.registers[VIC_REGISTER.lightPenX] = byte(Math.trunc(x / 2));
    this.registers[VIC_REGISTER.lightPenY] = byte(y);
    this.interruptLatches |= VIC_INTERRUPT_BIT.lightPen;
  }

  /** 控制口 1 的 FIRE 与 VIC-II `/LP` 共线；下降沿在一个芯片周期后锁存光栅。 */
  setLightPenInputHigh(high: boolean): void {
    if (this.lightPenInputHigh === high) return;
    this.lightPenInputHigh = high;
    if (!high) {
      this.lightPenTriggerCyclesRemaining = PAL_VIC_TIMING.lightPen.triggerDelayCycles;
    }
  }

  private clockLightPenTrigger(rasterCycle: number, rasterLine: number): void {
    if (this.lightPenTriggerCyclesRemaining === 0) return;
    this.lightPenTriggerCyclesRemaining -= 1;
    if (this.lightPenTriggerCyclesRemaining !== 0 || this.lightPenLatched) return;

    const timing = PAL_VIC_TIMING.lightPen;
    const horizontalPixels =
      (timing.horizontalOriginPixels + (rasterCycle - 1) * 8) %
      timing.horizontalPositionModuloPixels;
    // LPX 锁存的是按芯片周期量化的横向计数器，再叠加 6569R3 的相位位；若直接使用
    // 未量化的 φ1 像素坐标，低三位与相位偏移会被重复计入。
    const horizontalCounterPixels =
      horizontalPixels - (horizontalPixels % timing.horizontalCounterGranularityPixels);
    this.lightPenLatched = true;
    this.registers[VIC_REGISTER.lightPenX] = byte(
      Math.trunc(horizontalCounterPixels / 2) + timing.mos6569R3RegisterOffset,
    );
    this.registers[VIC_REGISTER.lightPenY] = byte(rasterLine);
    this.interruptLatches |= VIC_INTERRUPT_BIT.lightPen;
  }

  private installRegisterMap(): void {
    for (let spriteIndex = 0; spriteIndex < VIC_SPRITE_COUNT; spriteIndex += 1) {
      const xRegister = spriteIndex * 2;
      const yRegister = xRegister + 1;
      this.mapRegister(xRegister, {
        write: (index, value) => {
          const sprite = this.sprites[spriteIndex];
          if (!sprite) return;
          sprite.x = (sprite.x & 0x100) | value;
          this.writeDefault(index, value);
        },
      });
      this.mapRegister(yRegister, {
        write: (index, value) => {
          const sprite = this.sprites[spriteIndex];
          if (!sprite) return;
          sprite.y = value;
          this.writeDefault(index, value);
        },
      });
    }

    this.mapRegister(VIC_REGISTER.spriteXMostSignificantBits, {
      write: (index, value) => {
        this.writeDefault(index, value);
        this.forEachSpriteBit(value, (sprite, set) => {
          sprite.x = set ? sprite.x | VIC_MASK.rasterHigh : sprite.x & 0xff;
        });
      },
    });
    this.mapRegister(VIC_REGISTER.screenControl1, {
      read: (index) =>
        ((this.registers[index] ?? 0) & ~VIC_SCREEN_CONTROL_1_BIT.rasterCounterHigh) |
        ((this.rasterPosition & VIC_MASK.rasterHigh) >> 1),
      write: (index, value) => this.writeScreenControl1(index, value),
    });
    this.mapRegister(VIC_REGISTER.rasterCounter, {
      read: () => byte(this.rasterPosition),
      write: (_index, value) => {
        this.rasterTrigger = (this.rasterTrigger & VIC_MASK.rasterHigh) | value;
        this.updateRasterInterruptComparison();
      },
    });
    this.mapRegister(VIC_REGISTER.lightPenX, {
      read: (index) => this.readDefault(index),
      write: () => undefined,
    });
    this.mapRegister(VIC_REGISTER.lightPenY, {
      read: (index) => this.readDefault(index),
      write: () => undefined,
    });
    this.mapRegister(VIC_REGISTER.spriteEnable, {
      write: (index, value) => {
        this.writeDefault(index, value);
        this.forEachSpriteBit(value, (sprite, set) => {
          sprite.enabled = set;
        });
        this.spritesEnabled = value !== 0;
      },
    });
    this.mapRegister(VIC_REGISTER.screenControl2, {
      read: (index) => this.readDefault(index) | VIC_REGISTER_READ_MASK.screenControl2UnusedBits,
      write: (index, value) => this.writeScreenControl2(index, value),
    });
    this.mapRegister(VIC_REGISTER.spriteExpandVertical, {
      write: (index, value) => {
        const previousValue = this.registers[index] ?? 0;
        if (value !== previousValue) {
          // CPU 写周期先推进到当前 VIC-II 周期再提交寄存器值；时序器因此可以在第 15
          // 周期执行 MC crunch，并在第 56 周期之后正确恢复扩展触发器。
          this.cycleSequencer.writeSpriteVerticalExpansionRegister(value);
        }
        this.writeDefault(index, value);
        this.forEachSpriteBit(value, (sprite, set) => {
          sprite.expandVertical = set;
        });
      },
    });
    this.mapRegister(VIC_REGISTER.memoryPointers, {
      read: (index) => this.readDefault(index) | VIC_REGISTER_READ_MASK.memoryPointersUnusedBit,
      write: (index, value) => {
        this.writeDefault(index, value);
        this.characterMemoryAddress = (value & VIC_MASK.characterMemoryPointer) << 10;
        this.bitmapMemoryAddress = (value & VIC_MASK.bitmapMemoryPointer) << 10;
        this.screenMemoryAddress = (value & VIC_MASK.screenMemoryPointer) << 6;
      },
    });
    this.mapRegister(VIC_REGISTER.interruptStatus, {
      read: () => this.readInterruptStatus(),
      write: (_index, value) => {
        this.interruptLatches &= ~(value & VIC_MASK.interruptSources);
      },
    });
    this.mapRegister(VIC_REGISTER.interruptMask, {
      read: () => this.interruptMask | VIC_MASK.interruptMaskReadHigh,
      write: (index, value) => {
        this.interruptMask = value & VIC_MASK.interruptSources;
        this.registers[index] = this.interruptMask | VIC_MASK.interruptMaskReadHigh;
      },
    });
    this.mapSpriteFlagRegister(VIC_REGISTER.spritePriority, (sprite, set) => {
      sprite.foreground = !set;
    });
    this.mapSpriteFlagRegister(VIC_REGISTER.spriteMulticolorEnable, (sprite, set) => {
      sprite.multicolor = set;
    });
    this.mapSpriteFlagRegister(VIC_REGISTER.spriteExpandHorizontal, (sprite, set) => {
      sprite.expandHorizontal = set;
    });
    this.mapRegister(VIC_REGISTER.spriteSpriteCollision, {
      read: () => this.readSpriteCollision('sprite'),
      write: () => undefined,
    });
    this.mapRegister(VIC_REGISTER.spriteForegroundCollision, {
      read: () => this.readSpriteCollision('foreground'),
      write: () => undefined,
    });

    this.mapColorRegister(VIC_REGISTER.borderColor, (color) => {
      this.borderColor = color;
    });
    for (let colorIndex = 0; colorIndex < VIC_BACKGROUND_COLOR_COUNT; colorIndex += 1) {
      this.mapColorRegister(VIC_REGISTER.backgroundColor0 + colorIndex, (color) => {
        this.backgroundColors[colorIndex] = color;
      });
    }
    this.mapColorRegister(VIC_REGISTER.spriteMulticolor0, (color) => {
      this.spriteMulticolor0 = color;
    });
    this.mapColorRegister(VIC_REGISTER.spriteMulticolor1, (color) => {
      this.spriteMulticolor1 = color;
    });
    for (let spriteIndex = 0; spriteIndex < VIC_SPRITE_COUNT; spriteIndex += 1) {
      this.mapColorRegister(VIC_REGISTER.spriteColor0 + spriteIndex, (color) => {
        const sprite = this.sprites[spriteIndex];
        if (sprite) sprite.color = color;
      });
    }
    for (let register = VIC_REGISTER.firstUnused; register < VIC_REGISTER_COUNT; register += 1) {
      this.mapRegister(register, {
        read: () => VIC_REGISTER_READ_MASK.unusedRegister,
      });
    }
  }

  private writeScreenControl1(index: number, value: number): void {
    this.verticalScroll = value & VIC_MASK.scroll;
    this.screenHeight =
      (value & VIC_SCREEN_CONTROL_1_BIT.rowSelect) !== 0
        ? VIC_DISPLAY.standardRowCount
        : VIC_DISPLAY.reducedRowCount;
    this.screenVisible = (value & VIC_SCREEN_CONTROL_1_BIT.displayEnable) !== 0;
    this.bitmapMode = (value & VIC_SCREEN_CONTROL_1_BIT.bitmapMode) !== 0;
    this.extendedBackgroundMode = (value & VIC_SCREEN_CONTROL_1_BIT.extendedBackgroundMode) !== 0;
    this.displayMode =
      (this.displayMode & 0x01) |
      ((value &
        (VIC_SCREEN_CONTROL_1_BIT.bitmapMode | VIC_SCREEN_CONTROL_1_BIT.extendedBackgroundMode)) >>
        4);
    this.displayModeValid = this.displayMode <= VIC_DISPLAY.highestValidMode;
    this.rasterTrigger =
      (this.rasterTrigger & 0xff) | ((value & VIC_SCREEN_CONTROL_1_BIT.rasterCounterHigh) << 1);
    this.registers[index] = value & ~VIC_SCREEN_CONTROL_1_BIT.rasterCounterHigh;
    this.updateRasterInterruptComparison();
  }

  private writeScreenControl2(index: number, value: number): void {
    this.horizontalScroll = value & VIC_MASK.scroll;
    this.screenWidth =
      (value & VIC_SCREEN_CONTROL_2_BIT.columnSelect) !== 0
        ? VIC_DISPLAY.standardColumnCount
        : VIC_DISPLAY.reducedColumnCount;
    this.multicolorMode = (value & VIC_SCREEN_CONTROL_2_BIT.multicolorMode) !== 0;
    this.displayMode = (this.displayMode & 0x06) | (this.multicolorMode ? 1 : 0);
    this.displayModeValid = this.displayMode <= VIC_DISPLAY.highestValidMode;
    this.registers[index] = byte(value);
  }

  private readInterruptStatus(): number {
    const active = this.interruptLatches & VIC_MASK.interruptSources;
    return (
      VIC_MASK.interruptStatusReadHigh |
      active |
      ((active & this.interruptMask) !== 0 ? VIC_INTERRUPT_BIT.any : 0)
    );
  }

  private readSpriteCollision(type: 'foreground' | 'sprite'): number {
    const value = type === 'sprite' ? this.spriteSpriteCollision : this.spriteForegroundCollision;
    if (type === 'sprite') {
      this.spriteSpriteCollision = 0;
      for (const sprite of this.sprites) sprite.collisionWithSprite = false;
    } else {
      this.spriteForegroundCollision = 0;
      for (const sprite of this.sprites) sprite.collisionWithForeground = false;
    }
    return value;
  }

  private mapSpriteFlagRegister(
    register: number,
    apply: (sprite: VicSprite, set: boolean) => void,
  ): void {
    this.mapRegister(register, {
      write: (index, value) => {
        this.writeDefault(index, value);
        this.forEachSpriteBit(value, apply);
      },
    });
  }

  private mapColorRegister(register: number, apply: (color: number) => void): void {
    this.mapRegister(register, {
      read: (index) => this.readDefault(index) | VIC_REGISTER_READ_MASK.colorHighBits,
      write: (index, value) => {
        this.writeDefault(index, value & VIC_MASK.color);
        apply(this.paletteColor(value));
      },
    });
  }

  private forEachSpriteBit(value: number, apply: (sprite: VicSprite, set: boolean) => void): void {
    for (let index = 0; index < this.sprites.length; index += 1) {
      const sprite = this.sprites[index];
      if (sprite) apply(sprite, (value & (1 << index)) !== 0);
    }
  }

  private paletteColor(value: number): number {
    return C64_PALETTE[value & VIC_MASK.color] ?? C64_COLOR.black;
  }

  private updateRasterInterruptComparison(): void {
    const matched = this.rasterPosition === this.rasterTrigger;
    if (matched && !this.rasterInterruptMatched) {
      this.interruptLatches |= VIC_INTERRUPT_BIT.raster;
    }
    this.rasterInterruptMatched = matched;
  }

  private notifyRasterLineObservers(rasterLine: number): void {
    if (this.rasterLineObservers.size === 0) return;
    const event = {
      frameCompleted: rasterLine === PAL_VIC_TIMING.rasterLineCount - 1,
      rasterLine,
    } as const;
    for (const observer of this.rasterLineObservers) observer(event);
  }
}
