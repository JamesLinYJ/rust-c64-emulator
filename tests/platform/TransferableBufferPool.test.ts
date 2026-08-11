// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - Worker Transferable 双缓冲测试
//
//   文件:       TransferableBufferPool.test.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it } from 'vitest';

import { TransferableBufferPool } from '../../src/platform/TransferableBufferPool';

describe('TransferableBufferPool', () => {
  it('bounds production ownership to two recyclable buffers', () => {
    const pool = new TransferableBufferPool(16, 2);
    const first = pool.acquire();
    const second = pool.acquire();

    expect(first).toBeInstanceOf(ArrayBuffer);
    expect(second).toBeInstanceOf(ArrayBuffer);
    expect(first).not.toBe(second);
    expect(pool.acquire()).toBeUndefined();
    expect(pool.inFlightCount).toBe(2);

    pool.release(new ArrayBuffer(16));
    expect(pool.inFlightCount).toBe(1);
    expect(pool.acquire()).toBeInstanceOf(ArrayBuffer);
    expect(pool.inFlightCount).toBe(2);
  });

  it('rejects malformed or unsolicited returned ownership', () => {
    const pool = new TransferableBufferPool(8, 2);
    expect(() => pool.release(new ArrayBuffer(8))).toThrow(/in flight/u);

    pool.acquire();
    expect(() => pool.release(new ArrayBuffer(7))).toThrow(/8 bytes/u);
    expect(pool.inFlightCount).toBe(1);
  });
});
