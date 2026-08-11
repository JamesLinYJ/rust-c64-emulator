// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - Canvas 像素转换测试
//
//   文件:       CanvasSurface.test.ts
//
//   日期:       2026年08月08日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it } from 'vitest';

import { packRgbaPixel } from '../../src/shared/RgbaPixel';
import { PixelFrameBuffer } from '../../src/video/PixelFrameBuffer';

const PAL_OUTPUT_WIDTH = 403;
const PAL_OUTPUT_HEIGHT = 284;

describe('Canvas framebuffer representation', () => {
  it('stores every fixed PAL output pixel as exact RGBA bytes in the shared ImageData view', () => {
    const pixelCount = PAL_OUTPUT_WIDTH * PAL_OUTPUT_HEIGHT;
    const imageBytes = new Uint8ClampedArray(pixelCount * 4);
    const expectedBytes = new Uint8ClampedArray(imageBytes.length);
    const imageWords = new Uint32Array(imageBytes.buffer);
    const frameBuffer = new PixelFrameBuffer(PAL_OUTPUT_WIDTH, PAL_OUTPUT_HEIGHT, imageWords);

    for (let index = 0; index < frameBuffer.pixels.length; index += 1) {
      const byteOffset = index * 4;
      const alpha = (index * 29 + 3) & 0xff;
      const red = (index * 31 + 5) & 0xff;
      const green = (index * 37 + 7) & 0xff;
      const blue = (index * 41 + 11) & 0xff;
      frameBuffer.pixels[index] = packRgbaPixel(red, green, blue, alpha);
      expectedBytes[byteOffset] = red;
      expectedBytes[byteOffset + 1] = green;
      expectedBytes[byteOffset + 2] = blue;
      expectedBytes[byteOffset + 3] = alpha;
    }

    let firstMismatch = -1;
    for (let index = 0; index < imageBytes.length; index += 1) {
      if (imageBytes[index] === expectedBytes[index]) continue;
      firstMismatch = index;
      break;
    }
    expect(firstMismatch).toBe(-1);
  });
});
