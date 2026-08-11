// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - Worker Transferable 固定缓冲池
//
//   文件:       TransferableBufferPool.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

/** 固定容量所有权池；缓冲区跨线程返回后才允许下一次 acquire。 */
export class TransferableBufferPool {
  private readonly available: ArrayBuffer[];
  private inFlight = 0;

  constructor(
    readonly byteLength: number,
    readonly capacity: number,
  ) {
    if (!Number.isSafeInteger(byteLength) || byteLength <= 0) {
      throw new RangeError(
        `Transferable buffer size must be a positive integer; got ${byteLength}.`,
      );
    }
    if (!Number.isSafeInteger(capacity) || capacity <= 0) {
      throw new RangeError(`Transferable buffer capacity must be positive; got ${capacity}.`);
    }
    this.available = Array.from({ length: capacity }, () => new ArrayBuffer(byteLength));
  }

  get inFlightCount(): number {
    return this.inFlight;
  }

  acquire(): ArrayBuffer | undefined {
    const buffer = this.available.pop();
    if (!buffer) return undefined;
    this.inFlight += 1;
    return buffer;
  }

  release(buffer: ArrayBuffer): void {
    if (buffer.byteLength !== this.byteLength) {
      throw new RangeError(
        `Returned Transferable buffer must contain exactly ${this.byteLength} bytes; got ${buffer.byteLength}.`,
      );
    }
    if (this.inFlight === 0) {
      throw new Error('Cannot return a Transferable buffer when none are in flight.');
    }
    this.inFlight -= 1;
    this.available.push(buffer);
  }
}
