// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - asynchronous Rust JSON trace process runner
//
//   File:       runRustJsonTrace.ts
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { spawn, spawnSync } from 'node:child_process';
import { closeSync, mkdtempSync, openSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

interface RustJsonTraceOptions {
  readonly example: string;
  readonly input: unknown;
  readonly label: string;
  readonly maximumOutputBytes: number;
}

interface BoundedCommandOptions {
  readonly arguments: readonly string[];
  readonly command: string;
  readonly input?: string;
  readonly label: string;
  readonly maximumOutputBytes: number;
  readonly timeoutMilliseconds?: number;
}

export async function runBoundedCommand({
  arguments: commandArguments,
  command,
  input,
  label,
  maximumOutputBytes,
  timeoutMilliseconds,
}: BoundedCommandOptions): Promise<string> {
  const child = spawn(command, commandArguments, {
    cwd: process.cwd(),
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  let stdoutBytes = 0;
  let stderrBytes = 0;
  let outputLimitExceeded = false;
  let timedOut = false;

  child.stdout.on('data', (chunk: Buffer) => {
    stdoutBytes += chunk.byteLength;
    if (stdoutBytes > maximumOutputBytes) {
      outputLimitExceeded = true;
      child.kill();
      return;
    }
    stdout.push(chunk);
  });
  child.stderr.on('data', (chunk: Buffer) => {
    stderrBytes += chunk.byteLength;
    if (stderrBytes > maximumOutputBytes) {
      outputLimitExceeded = true;
      child.kill();
      return;
    }
    stderr.push(chunk);
  });

  const completion = new Promise<number | null>((resolve, reject) => {
    child.once('error', reject);
    child.once('close', resolve);
  });
  const timeout =
    timeoutMilliseconds === undefined
      ? undefined
      : setTimeout(() => {
          timedOut = true;
          child.kill();
        }, timeoutMilliseconds);
  child.stdin.end(input);
  const exitCode = await completion.finally(() => {
    if (timeout !== undefined) clearTimeout(timeout);
  });
  if (timedOut) {
    throw new Error(`${label} exceeded its ${String(timeoutMilliseconds)} ms timeout.`);
  }
  if (outputLimitExceeded) {
    throw new Error(`${label} exceeded ${maximumOutputBytes} output bytes.`);
  }
  if (exitCode !== 0) {
    throw new Error(
      `${label} failed (${String(exitCode)}):\n${Buffer.concat(stderr).toString('utf8')}`,
    );
  }
  return Buffer.concat(stdout).toString('utf8');
}

export async function runRustJsonTrace<Output>({
  example,
  input,
  label,
  maximumOutputBytes,
}: RustJsonTraceOptions): Promise<Output> {
  const output = await runBoundedCommand({
    arguments: ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', example],
    command: 'cargo',
    input: `${JSON.stringify(input)}\n`,
    label,
    maximumOutputBytes,
  });
  return JSON.parse(output) as Output;
}

export function runRustJsonTraceSync<Output>({
  example,
  input,
  label,
  maximumOutputBytes,
}: RustJsonTraceOptions): Output {
  const traceDirectory = mkdtempSync(join(tmpdir(), 'rust-c64-trace-'));
  const inputPath = join(traceDirectory, 'request.json');
  let inputDescriptor: number | undefined;
  try {
    writeFileSync(inputPath, JSON.stringify(input));
    inputDescriptor = openSync(inputPath, 'r');
    const result = spawnSync(
      'cargo',
      ['run', '--quiet', '--locked', '-p', 'c64-core', '--example', example],
      {
        cwd: process.cwd(),
        encoding: 'utf8',
        maxBuffer: maximumOutputBytes,
        stdio: [inputDescriptor, 'pipe', 'pipe'],
      },
    );
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(`${label} failed (${String(result.status)}):\n${result.stderr}`);
    }
    return JSON.parse(result.stdout) as Output;
  } finally {
    if (inputDescriptor !== undefined) closeSync(inputDescriptor);
    rmSync(traceDirectory, { force: true, recursive: true });
  }
}
