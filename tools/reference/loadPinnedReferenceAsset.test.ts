// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 固定哈希参考资产加载器测试
//
//   文件:       loadPinnedReferenceAsset.test.ts
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { createHash } from 'node:crypto';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { loadPinnedReferenceAsset } from './loadPinnedReferenceAsset';

const temporaryDirectories: string[] = [];

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

async function createTemporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), 'rust-c64-reference-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((directory) => rm(directory, { force: true, recursive: true })),
  );
});

describe('loadPinnedReferenceAsset', () => {
  it('uses a hash-verified cache without touching the network', async () => {
    const directory = await createTemporaryDirectory();
    const cachePath = join(directory, 'asset.bin');
    const expected = new TextEncoder().encode('verified fixture');
    await writeFile(cachePath, expected);
    let fetchCount = 0;

    const actual = await loadPinnedReferenceAsset(
      { cachePath, name: 'fixture', sha256: sha256(expected), url: 'https://example.invalid/a' },
      {
        fetchImplementation: () => {
          fetchCount += 1;
          return Promise.resolve(new Response('unexpected'));
        },
      },
    );

    expect(actual).toEqual(expected);
    expect(fetchCount).toBe(0);
  });

  it('retries transient transport failures and publishes one complete cache file', async () => {
    const directory = await createTemporaryDirectory();
    const cachePath = join(directory, 'asset.bin');
    const expectedText = 'downloaded fixture';
    const expected = new TextEncoder().encode(expectedText);
    let fetchCount = 0;

    const actual = await loadPinnedReferenceAsset(
      { cachePath, name: 'fixture', sha256: sha256(expected), url: 'https://example.invalid/a' },
      {
        fetchImplementation: () => {
          fetchCount += 1;
          if (fetchCount < 3) return Promise.reject(new TypeError('transient TLS reset'));
          return Promise.resolve(new Response(expectedText));
        },
        retryDelaysMs: [0, 0, 0],
      },
    );

    expect(actual).toEqual(expected);
    expect(fetchCount).toBe(3);
    expect(new Uint8Array(await readFile(cachePath))).toEqual(expected);
    expect(await readdir(directory)).toEqual(['asset.bin']);
  });

  it('rejects corrupted cached and downloaded bytes instead of hiding an integrity failure', async () => {
    const directory = await createTemporaryDirectory();
    const cachePath = join(directory, 'asset.bin');
    const expected = new TextEncoder().encode('expected');
    await writeFile(cachePath, 'corrupt');
    let fetchCount = 0;

    await expect(
      loadPinnedReferenceAsset(
        { cachePath, name: 'fixture', sha256: sha256(expected), url: 'https://example.invalid/a' },
        {
          fetchImplementation: () => {
            fetchCount += 1;
            return Promise.resolve(new Response('also corrupt'));
          },
          retryDelaysMs: [0, 0],
        },
      ),
    ).rejects.toThrow('SHA-256 mismatch');
    expect(fetchCount).toBe(0);
  });
});
