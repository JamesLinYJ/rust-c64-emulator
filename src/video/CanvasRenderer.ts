// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Canvas 视频输出
//
//   文件:       CanvasRenderer.ts
//
//   日期:       2026年07月16日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { CanvasSurface } from './CanvasSurface';
import { PAL_VIDEO_STANDARD } from './palVideoStandard';

/**
 * 浏览器输出适配器。
 *
 * VIC-II 已在芯片时钟域完成像素、边框、精灵优先级和碰撞；这里仅复制锁存光栅线并提交
 * ImageData。这样无 Canvas 的测试和服务器环境仍得到完全相同的芯片寄存器行为。
 */
export class CanvasRenderer {
  readonly surface: CanvasSurface;

  constructor(
    canvas: HTMLCanvasElement,
    initialBackgroundColor: number,
    size: { readonly height: number; readonly width: number } = C64_CANVAS_SIZE,
  ) {
    this.surface = new CanvasSurface(canvas, size.width, size.height);
    this.surface.clear(initialBackgroundColor);
    this.surface.present();
  }

  beginFrame(backgroundColor: number): void {
    this.surface.clear(backgroundColor);
  }

  writeRasterLine(visibleRow: number, pixels: Uint32Array): void {
    this.surface.frameBuffer.writeRow(visibleRow, pixels);
  }

  presentPixels(pixels: Uint32Array): void {
    if (pixels.length !== this.surface.frameBuffer.pixels.length) {
      throw new RangeError(
        `Frame contains ${pixels.length} pixels; expected ${this.surface.frameBuffer.pixels.length}.`,
      );
    }
    this.surface.frameBuffer.pixels.set(pixels);
    this.surface.present();
  }

  presentFrame(): void {
    this.surface.present();
  }
}

export const C64_CANVAS_SIZE = {
  width: PAL_VIDEO_STANDARD.output.width,
  height: PAL_VIDEO_STANDARD.output.height,
} as const;
