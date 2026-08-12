// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Sites production artifact packager
//
//   File:       packageSitesBuild.ts
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import type { Dirent } from 'node:fs';
import { copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { extname, join, relative, resolve } from 'node:path';

import { minify, transformWithOxc } from 'vite';

const ROOT_DIRECTORY = resolve('.');
const DIST_DIRECTORY = resolve('dist');
const CLIENT_DIRECTORY = resolve('dist/client');
const SERVER_DIRECTORY = resolve('dist/server');
const HOSTING_DIRECTORY = resolve('dist/.openai');
const GENERATED_WASM_DIRECTORY = resolve('generated/wasm/web');
const WASM_OUTPUT_DIRECTORY = resolve(CLIENT_DIRECTORY, 'wasm');
const SOURCE_MAP_DIRECTIVE = /(?:\/\/[#@]|\/\*#)\s*sourceMappingURL\s*=/u;
const TEXT_EXTENSIONS = new Set(['.css', '.html', '.js', '.json', '.mjs']);
const WASM_MAGIC = Uint8Array.of(0x00, 0x61, 0x73, 0x6d);

async function minifyModule(
  sourcePath: string,
  outputPath: string,
  loader: 'js' | 'ts',
): Promise<void> {
  const source = await readFile(sourcePath, 'utf8');
  const transformed = await transformWithOxc(source, sourcePath, {
    lang: loader,
    sourcemap: false,
    target: 'es2022',
  });
  if (transformed.warnings.length !== 0) {
    throw new Error(
      `${relative(ROOT_DIRECTORY, sourcePath)} produced transform warnings: ${transformed.warnings
        .map((warning) => warning.message)
        .join('; ')}`,
    );
  }
  const minified = await minify(outputPath, transformed.code, {
    compress: true,
    mangle: true,
    module: true,
    sourcemap: false,
  });
  if (minified.errors.length !== 0) {
    throw new Error(
      `${relative(ROOT_DIRECTORY, sourcePath)} failed to minify: ${minified.errors
        .map((error) => error.message)
        .join('; ')}`,
    );
  }
  if (SOURCE_MAP_DIRECTIVE.test(minified.code)) {
    throw new Error(`${relative(ROOT_DIRECTORY, outputPath)} contains a source map directive.`);
  }
  await writeFile(outputPath, minified.code, 'utf8');
}

async function listFiles(directory: string): Promise<string[]> {
  const entries: Dirent[] = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry): Promise<string[]> => {
      const path = join(directory, entry.name);
      return entry.isDirectory() ? listFiles(path) : [path];
    }),
  );
  return nested.flat();
}

async function assertNoSourceMaps(): Promise<number> {
  const files = await listFiles(resolve('dist'));
  const mapFiles = files.filter((file) => file.toLowerCase().endsWith('.map'));
  if (mapFiles.length !== 0) {
    throw new Error(
      `Production build contains source maps: ${mapFiles
        .map((file) => relative(ROOT_DIRECTORY, file))
        .join(', ')}`,
    );
  }

  for (const file of files) {
    if (!TEXT_EXTENSIONS.has(extname(file).toLowerCase())) continue;
    const contents = await readFile(file, 'utf8');
    if (SOURCE_MAP_DIRECTIVE.test(contents)) {
      throw new Error(`${relative(ROOT_DIRECTORY, file)} contains a source map directive.`);
    }
  }
  return files.length;
}

async function assertWasmArtifact(path: string): Promise<number> {
  const bytes = await readFile(path);
  if (!WASM_MAGIC.every((value, index) => bytes[index] === value)) {
    throw new Error(`${relative(ROOT_DIRECTORY, path)} is not a WebAssembly module.`);
  }
  return bytes.length;
}

async function clearNonClientBuildOutputs(): Promise<void> {
  const entries = await readdir(DIST_DIRECTORY, { withFileTypes: true });
  await Promise.all(
    entries
      .filter((entry) => entry.name !== 'client')
      .map((entry) => rm(join(DIST_DIRECTORY, entry.name), { force: true, recursive: true })),
  );
}

async function packageSitesBuild(): Promise<void> {
  await clearNonClientBuildOutputs();
  const indexPath = resolve(CLIENT_DIRECTORY, 'index.html');
  if (!(await stat(indexPath)).isFile()) {
    throw new Error('Vite did not produce dist/client/index.html.');
  }

  await Promise.all([
    mkdir(SERVER_DIRECTORY, { recursive: true }),
    mkdir(HOSTING_DIRECTORY, { recursive: true }),
    mkdir(WASM_OUTPUT_DIRECTORY, { recursive: true }),
  ]);
  await Promise.all([
    minifyModule(resolve('site/worker.ts'), resolve(SERVER_DIRECTORY, 'index.js'), 'ts'),
    minifyModule(
      resolve(GENERATED_WASM_DIRECTORY, 'c64_vm.js'),
      resolve(WASM_OUTPUT_DIRECTORY, 'c64_vm.js'),
      'js',
    ),
    copyFile(
      resolve(GENERATED_WASM_DIRECTORY, 'c64_vm_bg.wasm'),
      resolve(WASM_OUTPUT_DIRECTORY, 'c64_vm_bg.wasm'),
    ),
    copyFile(resolve('.openai/hosting.json'), resolve(HOSTING_DIRECTORY, 'hosting.json')),
  ]);

  const wasmPath = resolve(WASM_OUTPUT_DIRECTORY, 'c64_vm_bg.wasm');
  const [fileCount, wasmByteLength] = await Promise.all([
    assertNoSourceMaps(),
    assertWasmArtifact(wasmPath),
  ]);
  console.log(
    `Sites production build verified: ${fileCount} files, ${wasmByteLength.toLocaleString(
      'en-US',
    )}-byte Wasm module, no source maps.`,
  );
}

await packageSitesBuild();
