// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 固定哈希参考资产加载器
//
//   文件:       loadPinnedReferenceAsset.ts
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { createHash, randomUUID } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { setTimeout as wait } from 'node:timers/promises';

export interface PinnedReferenceAsset {
  readonly cachePath: string;
  readonly name: string;
  readonly sha256: string;
  readonly url: string;
}

export interface PinnedReferenceAssetLoadOptions {
  readonly fetchImplementation?: typeof fetch;
  readonly retryDelaysMs?: readonly number[];
  readonly timeoutMs?: number;
}

const DEFAULT_RETRY_DELAYS_MS = [0, 250, 1_000, 3_000] as const;
const DEFAULT_TIMEOUT_MS = 30_000;

class ReferenceAssetHttpError extends Error {
  public constructor(
    message: string,
    public readonly retryable: boolean,
  ) {
    super(message);
    this.name = 'ReferenceAssetHttpError';
  }
}

class ReferenceAssetIntegrityError extends Error {
  public constructor(message: string) {
    super(message);
    this.name = 'ReferenceAssetIntegrityError';
  }
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

function isMissingFileError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error && error.code === 'ENOENT';
}

function isRetryableStatus(status: number): boolean {
  return status === 408 || status === 425 || status === 429 || status >= 500;
}

function validateHash(asset: PinnedReferenceAsset, bytes: Uint8Array, source: string): void {
  const actualHash = sha256(bytes);
  if (actualHash !== asset.sha256) {
    throw new ReferenceAssetIntegrityError(
      `${source} ${asset.name} SHA-256 mismatch: expected ${asset.sha256}, received ${actualHash}.`,
    );
  }
}

async function readCachedAsset(asset: PinnedReferenceAsset): Promise<Uint8Array | undefined> {
  try {
    const cached = new Uint8Array(await readFile(resolve(asset.cachePath)));
    validateHash(asset, cached, 'Cached reference asset');
    return cached;
  } catch (error: unknown) {
    if (isMissingFileError(error)) return undefined;
    throw error;
  }
}

async function fetchWithTimeout(
  fetchImplementation: typeof fetch,
  url: string,
  timeoutMs: number,
): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetchImplementation(url, { signal: controller.signal });
  } finally {
    clearTimeout(timeout);
  }
}

async function writeCacheAtomically(
  asset: PinnedReferenceAsset,
  downloaded: Uint8Array,
): Promise<void> {
  const cachePath = resolve(asset.cachePath);
  const temporaryPath = `${cachePath}.${process.pid}.${randomUUID()}.tmp`;
  await mkdir(dirname(cachePath), { recursive: true });

  try {
    await writeFile(temporaryPath, downloaded, { flag: 'wx' });
    await rename(temporaryPath, cachePath);
  } catch (error: unknown) {
    // 并行门禁可能同时填充同一缓存。只在另一写入者已经留下正确资产时接受竞争结果。
    const concurrentCache = await readCachedAsset(asset);
    if (concurrentCache === undefined) throw error;
  } finally {
    await rm(temporaryPath, { force: true });
  }
}

/**
 * 加载固定 SHA-256 的参考资产。缓存损坏立即失败；网络瞬断和服务端限流执行有界重试；
 * 下载内容只有在哈希验证完成后才通过原子 rename 对其他测试可见。
 */
export async function loadPinnedReferenceAsset(
  asset: PinnedReferenceAsset,
  options: PinnedReferenceAssetLoadOptions = {},
): Promise<Uint8Array> {
  const cached = await readCachedAsset(asset);
  if (cached !== undefined) return cached;

  const fetchImplementation = options.fetchImplementation ?? globalThis.fetch;
  const retryDelaysMs = options.retryDelaysMs ?? DEFAULT_RETRY_DELAYS_MS;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (retryDelaysMs.length === 0) {
    throw new Error('Reference asset retry schedule must contain at least one attempt.');
  }

  let lastError: unknown;
  for (const [attemptIndex, retryDelayMs] of retryDelaysMs.entries()) {
    if (retryDelayMs < 0 || !Number.isFinite(retryDelayMs)) {
      throw new Error(`Invalid reference asset retry delay: ${retryDelayMs}.`);
    }
    if (attemptIndex > 0 && retryDelayMs > 0) await wait(retryDelayMs);

    try {
      const response = await fetchWithTimeout(fetchImplementation, asset.url, timeoutMs);
      if (!response.ok) {
        throw new ReferenceAssetHttpError(
          `Unable to download reference asset ${asset.name}: HTTP ${response.status}.`,
          isRetryableStatus(response.status),
        );
      }

      const downloaded = new Uint8Array(await response.arrayBuffer());
      validateHash(asset, downloaded, 'Downloaded reference asset');
      await writeCacheAtomically(asset, downloaded);
      return downloaded;
    } catch (error: unknown) {
      if (error instanceof ReferenceAssetIntegrityError) throw error;
      if (error instanceof ReferenceAssetHttpError && !error.retryable) throw error;
      lastError = error;
    }
  }

  throw new Error(
    `Unable to download reference asset ${asset.name} after ${retryDelaysMs.length} attempts.`,
    { cause: lastError },
  );
}
