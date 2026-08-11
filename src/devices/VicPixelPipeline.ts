// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 逐周期像素管线
//
//   文件:       VicPixelPipeline.ts
//
//   日期:       2026年07月16日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { packRgbaPixel } from '../shared/RgbaPixel';
import type { VicCycleResult } from './VicCycleSequencer';
import type { VicSprite } from './VicSprite';
import { PAL_VIC_TIMING, type VicTiming } from './VicTiming';

export interface VicPixelRegisters {
  readonly backgroundColors: readonly number[];
  readonly bitmapMode: boolean;
  readonly borderColor: number;
  readonly displayModeValid: boolean;
  readonly extendedBackgroundMode: boolean;
  readonly horizontalScroll: number;
  readonly multicolorMode: boolean;
  readonly palette: readonly number[];
  readonly screenVisible: boolean;
  readonly spriteMulticolor0: number;
  readonly spriteMulticolor1: number;
  readonly sprites: readonly VicSprite[];
}

export interface VicPixelCollisionSink {
  recordSpriteForegroundCollision(spriteMask: number): void;
  recordSpriteSpriteCollision(spriteMask: number): void;
}

export interface VicPixelDataSource {
  readonly spriteDisplayMask: number;
  screenMatrixByte(column: number): number;
  colorMatrixNibble(column: number): number;
  graphicsByte(column: number): number;
  spriteDataWord(spriteIndex: number): number;
}

export interface VicPixelPipelineOptions {
  readonly firstVisibleCycle?: number;
  readonly outputWidth?: number;
}

interface GraphicsPixelResult {
  color: number;
  foreground: boolean;
}

interface SpritePixelResult {
  behindForeground: boolean;
  color: number;
  mask: number;
}

const PIXELS_PER_VIC_CYCLE = 8;
const TEXT_COLUMN_COUNT = 40;
const TEXT_COLUMN_WIDTH = 8;
const TEXT_DISPLAY_WIDTH = TEXT_COLUMN_COUNT * TEXT_COLUMN_WIDTH;
const SPRITE_SOURCE_WIDTH = 24;
const DEFAULT_FIRST_VISIBLE_CYCLE = 12;
const DEFAULT_OUTPUT_WIDTH = 403;
const DEFAULT_BLACK_PIXEL = packRgbaPixel(0, 0, 0);
const TRANSPARENT_PIXEL = 0;
const VIC_X_COUNTER_MODULUS = 0x0200;
const VIC_X_COUNTER_MASK = VIC_X_COUNTER_MODULUS - 1;
const TEXT_DISPLAY_VIC_X = 0x18;
// PAL 光栅物理像素 112 对应 VIC 的 X=$000；可见裁剪从物理像素 88 开始。
const VIC_X_ZERO_PHYSICAL_PIXEL = 112;

/**
 * 在 VIC-II 时钟域内产生最终像素并锁存碰撞。
 *
 * PAL 6569 的 G-access 在周期 16..55 进行，字符移位器从周期 18 起输出，因此这里按
 * “输出周期 - 18”选择 40 个列锁存。寄存器在每个周期读取当前引脚状态，光栅中途写颜色、
 * 模式、X 坐标或优先级只影响后续像素；Canvas 不再重新读取可变寄存器。
 */
export class VicPixelPipeline {
  private readonly firstVisibleCycle: number;
  private readonly outputWidth: number;
  private readonly visibleCropPhysicalPixel: number;
  private readonly linePixels: Uint32Array;
  private borderColorOutputDelay: number | undefined;
  // 这两个结果槽随管线实例复用，避免为每个物理像素创建短命对象。
  private readonly graphicsPixelResult: GraphicsPixelResult = { color: 0, foreground: false };
  private readonly spritePixelResult: SpritePixelResult = {
    behindForeground: false,
    color: TRANSPARENT_PIXEL,
    mask: 0,
  };

  constructor(
    private readonly timing: VicTiming = PAL_VIC_TIMING,
    options: VicPixelPipelineOptions = {},
  ) {
    this.firstVisibleCycle = options.firstVisibleCycle ?? DEFAULT_FIRST_VISIBLE_CYCLE;
    this.outputWidth = options.outputWidth ?? DEFAULT_OUTPUT_WIDTH;
    if (!Number.isInteger(this.firstVisibleCycle) || this.firstVisibleCycle < 1) {
      throw new RangeError('VIC-II first visible cycle must be a positive integer.');
    }
    if (!Number.isInteger(this.outputWidth) || this.outputWidth <= 0) {
      throw new RangeError('VIC-II output width must be a positive integer.');
    }
    this.visibleCropPhysicalPixel = (this.firstVisibleCycle - 1) * PIXELS_PER_VIC_CYCLE;
    this.linePixels = new Uint32Array(this.outputWidth);
  }

  reset(borderColor: number): void {
    const normalizedColor = borderColor >>> 0;
    this.linePixels.fill(normalizedColor);
    this.borderColorOutputDelay = normalizedColor;
  }

  clockCycle(
    cycle: VicCycleResult,
    borderPixelMask: number,
    registers: VicPixelRegisters,
    fetch: VicPixelDataSource,
    collisions: VicPixelCollisionSink,
  ): void {
    if (cycle.cycle < 1 || cycle.cycle > this.timing.cyclesPerRasterLine) {
      throw new RangeError(`VIC-II pixel cycle ${cycle.cycle} is outside the selected timing.`);
    }
    if (cycle.lineStarted) this.linePixels.fill(registers.borderColor >>> 0);

    const physicalCycleX = (cycle.cycle - 1) * PIXELS_PER_VIC_CYCLE;
    const spriteDisplayMask = fetch.spriteDisplayMask;
    // PAL 6569 的颜色寄存器先完成 CPU 可见锁存，最终边框多路器再经过一个 dot
    // 的输出级；这个标量跨八像素批次和光栅线保留，不延迟背景或碰撞状态。
    const borderColor = registers.borderColor >>> 0;
    const latchedBorderColor = this.borderColorOutputDelay;
    const delayedBorderColor = latchedBorderColor ?? borderColor;
    if (latchedBorderColor !== borderColor) this.borderColorOutputDelay = borderColor;

    // 边框最终覆盖颜色，且没有活动精灵时也不可能产生碰撞。整周期边框因此可以按
    // 连续像素段直接写入，避免执行八次不具状态的图形解码。
    if (borderPixelMask === 0xff && spriteDisplayMask === 0) {
      const outputStart = physicalCycleX - this.visibleCropPhysicalPixel;
      const firstOutput = Math.max(0, outputStart);
      const lastOutput = Math.min(this.outputWidth, outputStart + PIXELS_PER_VIC_CYCLE);
      for (let outputX = firstOutput; outputX < lastOutput; outputX += 1) {
        this.linePixels[outputX] = borderColor;
      }
      if (outputStart >= 0 && outputStart < this.outputWidth) {
        this.linePixels[outputStart] = delayedBorderColor;
      }
      return;
    }

    let spriteSpriteCollisionMask = 0;
    let spriteForegroundCollisionMask = 0;
    for (let pixelInCycle = 0; pixelInCycle < PIXELS_PER_VIC_CYCLE; pixelInCycle += 1) {
      const physicalX = physicalCycleX + pixelInCycle;
      const vicX = (physicalX - VIC_X_ZERO_PHYSICAL_PIXEL) & VIC_X_COUNTER_MASK;
      const outputX = physicalX - this.visibleCropPhysicalPixel;

      this.resolveGraphicsPixel(vicX, registers, fetch);
      const graphics = this.graphicsPixelResult;
      let outputColor = graphics.color;

      // 一般光栅线没有活动精灵；在周期级锁存为零时完全跳过八次精灵扫描。
      if (spriteDisplayMask !== 0) {
        this.resolveSpritePixel(vicX, spriteDisplayMask, registers, fetch);
        const sprites = this.spritePixelResult;
        if ((sprites.mask & (sprites.mask - 1)) !== 0) {
          spriteSpriteCollisionMask |= sprites.mask;
        }
        if (graphics.foreground && sprites.mask !== 0) {
          spriteForegroundCollisionMask |= sprites.mask;
        }
        if (
          sprites.color !== TRANSPARENT_PIXEL &&
          !(graphics.foreground && sprites.behindForeground)
        ) {
          outputColor = sprites.color;
        }
      }

      // 边框属于最终模拟多路器：它遮住可见颜色，但不抑制此前已经发生的精灵碰撞。
      if ((borderPixelMask & (0x80 >> pixelInCycle)) !== 0) {
        outputColor = pixelInCycle === 0 ? delayedBorderColor : borderColor;
      }
      if (outputX >= 0 && outputX < this.outputWidth) {
        this.linePixels[outputX] = outputColor >>> 0;
      }
    }

    if (spriteSpriteCollisionMask !== 0) {
      collisions.recordSpriteSpriteCollision(spriteSpriteCollisionMask);
    }
    if (spriteForegroundCollisionMask !== 0) {
      collisions.recordSpriteForegroundCollision(spriteForegroundCollisionMask);
    }
  }

  snapshot(): Uint32Array {
    return this.linePixels.slice();
  }

  copyPixelsTo(target: Uint32Array, targetOffset: number): void {
    if (!Number.isInteger(targetOffset) || targetOffset < 0) {
      throw new RangeError(
        `VIC-II raster target offset ${targetOffset} must be a non-negative integer.`,
      );
    }
    if (targetOffset + this.linePixels.length > target.length) {
      throw new RangeError(
        `VIC-II raster of ${this.linePixels.length} pixels does not fit target length ${target.length} at offset ${targetOffset}.`,
      );
    }
    target.set(this.linePixels, targetOffset);
  }

  private resolveGraphicsPixel(
    vicX: number,
    registers: VicPixelRegisters,
    fetch: VicPixelDataSource,
  ): void {
    const result = this.graphicsPixelResult;
    const background0 =
      registers.backgroundColors[0] ?? registers.palette[0] ?? DEFAULT_BLACK_PIXEL;
    if (!registers.screenVisible) {
      result.color = background0;
      result.foreground = false;
      return;
    }
    if (!registers.displayModeValid) {
      result.color = registers.palette[0] ?? DEFAULT_BLACK_PIXEL;
      result.foreground = true;
      return;
    }

    const displayX = vicX - TEXT_DISPLAY_VIC_X - registers.horizontalScroll;
    if (displayX < 0 || displayX >= TEXT_DISPLAY_WIDTH) {
      result.color = background0;
      result.foreground = false;
      return;
    }
    const column = Math.trunc(displayX / TEXT_COLUMN_WIDTH);
    const pixel = displayX % TEXT_COLUMN_WIDTH;
    const screenCode = fetch.screenMatrixByte(column);
    const colorRam = fetch.colorMatrixNibble(column);
    const graphics = fetch.graphicsByte(column);

    if (registers.bitmapMode) {
      if (registers.multicolorMode) {
        this.resolveMulticolorBitmapPixel(pixel, graphics, screenCode, colorRam, registers);
      } else {
        this.resolveHighResolutionBitmapPixel(pixel, graphics, screenCode, registers);
      }
      return;
    }
    if (registers.multicolorMode && colorRam >= 8) {
      this.resolveMulticolorTextPixel(pixel, graphics, colorRam, registers);
    } else {
      this.resolveHighResolutionTextPixel(pixel, graphics, screenCode, colorRam, registers);
    }
  }

  private resolveHighResolutionTextPixel(
    pixel: number,
    graphics: number,
    screenCode: number,
    colorRam: number,
    registers: VicPixelRegisters,
  ): void {
    const result = this.graphicsPixelResult;
    const foreground = (graphics & (0x80 >> pixel)) !== 0;
    if (foreground) {
      result.color = registers.palette[colorRam] ?? registers.palette[0] ?? 0;
      result.foreground = true;
      return;
    }
    const backgroundIndex = registers.extendedBackgroundMode ? screenCode >> 6 : 0;
    result.color =
      registers.backgroundColors[backgroundIndex] ?? registers.backgroundColors[0] ?? 0;
    result.foreground = false;
  }

  private resolveMulticolorTextPixel(
    pixel: number,
    graphics: number,
    colorRam: number,
    registers: VicPixelRegisters,
  ): void {
    const pair = Math.trunc(pixel / 2);
    const colorIndex = (graphics >> (6 - pair * 2)) & 0x03;
    const result = this.graphicsPixelResult;
    switch (colorIndex) {
      case 0:
        result.color = registers.backgroundColors[0] ?? 0;
        break;
      case 1:
        result.color = registers.backgroundColors[1] ?? 0;
        break;
      case 2:
        result.color = registers.backgroundColors[2] ?? 0;
        break;
      default:
        result.color = registers.palette[colorRam & 0x07] ?? 0;
        break;
    }
    result.foreground = colorIndex >= 2;
  }

  private resolveHighResolutionBitmapPixel(
    pixel: number,
    graphics: number,
    screenCode: number,
    registers: VicPixelRegisters,
  ): void {
    const foreground = (graphics & (0x80 >> pixel)) !== 0;
    const paletteIndex = foreground ? screenCode >> 4 : screenCode & 0x0f;
    this.graphicsPixelResult.color = registers.palette[paletteIndex] ?? 0;
    this.graphicsPixelResult.foreground = foreground;
  }

  private resolveMulticolorBitmapPixel(
    pixel: number,
    graphics: number,
    screenCode: number,
    colorRam: number,
    registers: VicPixelRegisters,
  ): void {
    const pair = Math.trunc(pixel / 2);
    const colorIndex = (graphics >> (6 - pair * 2)) & 0x03;
    const result = this.graphicsPixelResult;
    switch (colorIndex) {
      case 0:
        result.color = registers.backgroundColors[0] ?? 0;
        break;
      case 1:
        result.color = registers.palette[screenCode >> 4] ?? 0;
        break;
      case 2:
        result.color = registers.palette[screenCode & 0x0f] ?? 0;
        break;
      default:
        result.color = registers.palette[colorRam] ?? 0;
        break;
    }
    result.foreground = colorIndex >= 2;
  }

  private resolveSpritePixel(
    vicX: number,
    displayMask: number,
    registers: VicPixelRegisters,
    fetch: VicPixelDataSource,
  ): void {
    let mask = 0;
    let selectedColor = TRANSPARENT_PIXEL;
    let selectedBehindForeground = false;

    for (let index = 0; index < registers.sprites.length; index += 1) {
      const sprite = registers.sprites[index];
      if (!sprite || (displayMask & (1 << index)) === 0) continue;
      const sourcePixel = spriteSourcePixel(vicX, sprite);
      if (sourcePixel === undefined) continue;

      const color = spritePixelColor(fetch.spriteDataWord(index), sourcePixel, sprite, registers);
      if (color === TRANSPARENT_PIXEL) continue;
      mask |= 1 << index;
      // VIC-II 的低编号精灵优先；循环顺序保证第一个非透明精灵保持可见。
      if (selectedColor === TRANSPARENT_PIXEL) {
        selectedColor = color;
        selectedBehindForeground = !sprite.foreground;
      }
    }

    const result = this.spritePixelResult;
    result.behindForeground = selectedBehindForeground;
    result.color = selectedColor;
    result.mask = mask;
  }
}

function spriteSourcePixel(vicX: number, sprite: VicSprite): number | undefined {
  const scale = sprite.expandHorizontal ? 2 : 1;
  const relativePixel = (vicX - sprite.x + VIC_X_COUNTER_MODULUS) & VIC_X_COUNTER_MASK;
  if (relativePixel >= SPRITE_SOURCE_WIDTH * scale) return undefined;
  return Math.trunc(relativePixel / scale);
}

function spritePixelColor(
  data: number,
  sourcePixel: number,
  sprite: VicSprite,
  registers: VicPixelRegisters,
): number {
  if (!sprite.multicolor) {
    return (data & (1 << (SPRITE_SOURCE_WIDTH - 1 - sourcePixel))) !== 0
      ? sprite.color
      : TRANSPARENT_PIXEL;
  }
  const pair = Math.trunc(sourcePixel / 2);
  const colorIndex = (data >> (SPRITE_SOURCE_WIDTH - 2 - pair * 2)) & 0x03;
  switch (colorIndex) {
    case 1:
      return registers.spriteMulticolor0;
    case 2:
      return sprite.color;
    case 3:
      return registers.spriteMulticolor1;
    default:
      return TRANSPARENT_PIXEL;
  }
}
