// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 跨浏览器生产 Worker/Wasm 验证器
//
//   文件:       verifyBrowserCompatibility.ts
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import {
  chromium,
  firefox,
  webkit,
  type BrowserType,
  type LaunchOptions,
  type Page,
  type Response,
} from 'playwright';
import { preview, type PreviewServer } from 'vite';

interface BrowserTarget {
  readonly browserType: BrowserType;
  readonly label: string;
  readonly launchOptions: LaunchOptions;
  readonly realtimePalRequired: boolean;
}

interface RuntimeMetrics {
  readonly framesPerSecond: number;
  readonly sampledFrames: number;
}

const PREVIEW_URL = 'http://127.0.0.1:4173';
const BASIC_BOOT_TIMEOUT_MS = 45_000;
const PROGRAM_LOAD_TIMEOUT_MS = 45_000;
const PROGRAM_START_TIMEOUT_MS = 10_000;
const PROGRAM_COUNTER_PATTERN = /PC \$([0-9A-F]{4})/u;
const RUNTIME_METRICS_PATTERN = /呈现\s+(\d+)\s+FPS.*?超预算\s+\d+\/(\d+)/u;
const MINIMUM_PAL_FRAMES_PER_SECOND = 45;
const MAXIMUM_PAL_FRAMES_PER_SECOND = 55;
const MINIMUM_SAMPLED_FRAMES = 20;

const BROWSER_TARGETS = {
  chrome: {
    browserType: chromium,
    label: 'Google Chrome',
    launchOptions: { channel: 'chrome', headless: true },
    realtimePalRequired: true,
  },
  edge: {
    browserType: chromium,
    label: 'Microsoft Edge',
    launchOptions: { channel: 'msedge', headless: true },
    realtimePalRequired: true,
  },
  firefox: {
    browserType: firefox,
    label: 'Mozilla Firefox',
    launchOptions: { headless: true },
    realtimePalRequired: false,
  },
  webkit: {
    browserType: webkit,
    label: 'WebKit',
    launchOptions: { headless: true },
    realtimePalRequired: false,
  },
} as const satisfies Record<string, BrowserTarget>;

type BrowserTargetName = keyof typeof BROWSER_TARGETS;

async function main(): Promise<void> {
  const targets = requestedTargets(process.argv[2]);
  const previewServer = await preview({
    preview: { host: '127.0.0.1', port: 4173, strictPort: true },
  });

  try {
    for (const targetName of targets) {
      await verifyBrowserTarget(BROWSER_TARGETS[targetName]);
    }
  } finally {
    await closePreviewServer(previewServer);
  }
}

function requestedTargets(argument: string | undefined): readonly BrowserTargetName[] {
  if (argument === undefined) return Object.keys(BROWSER_TARGETS) as BrowserTargetName[];
  if (isBrowserTargetName(argument)) return [argument];
  throw new Error(
    `Unknown browser target "${argument}"; expected ${Object.keys(BROWSER_TARGETS).join(', ')}.`,
  );
}

function isBrowserTargetName(value: string): value is BrowserTargetName {
  return Object.hasOwn(BROWSER_TARGETS, value);
}

async function verifyBrowserTarget(target: BrowserTarget): Promise<void> {
  const browser = await target.browserType.launch(target.launchOptions);
  const page = await browser.newPage({ viewport: { height: 900, width: 1_440 } });
  const problems: string[] = [];
  collectBrowserProblems(page, problems);

  try {
    await page.goto(PREVIEW_URL, { waitUntil: 'networkidle' });
    await waitForBoot(page);
    await verifyProductionWorker(page);
    const metrics = await waitForRuntimeMetrics(page, target.realtimePalRequired);

    await page.getByRole('button', { name: '暂停' }).click();
    await page.getByText('已暂停', { exact: true }).first().waitFor();
    await page.getByRole('button', { name: '单帧' }).click();
    await page.getByText('已执行一帧。', { exact: true }).waitFor();
    await page.getByRole('button', { name: '运行' }).click();
    await page.getByText('运行中', { exact: true }).first().waitFor();

    await page.getByRole('button', { name: '载入程序' }).click();
    await page.getByText(/已载入 .* 字节至 \$0801/u).waitFor({ timeout: PROGRAM_LOAD_TIMEOUT_MS });
    const programCounter = await waitForProgramExecution(page);

    if (problems.length > 0) {
      throw new Error(`${target.label} reported browser problems:\n${problems.join('\n')}`);
    }
    console.log(
      `PASS ${target.label}: production module Worker/Wasm, ${metrics.framesPerSecond} host FPS, ` +
        `${metrics.sampledFrames} sampled frames, PRG PC $${programCounter
          .toString(16)
          .toUpperCase()
          .padStart(4, '0')}.`,
    );
  } finally {
    await page.close();
    await browser.close();
  }
}

function collectBrowserProblems(page: Page, problems: string[]): void {
  const previewOrigin = new URL(PREVIEW_URL).origin;
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') {
      problems.push(`console.${message.type()}: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => problems.push(`pageerror: ${error.message}`));
  page.on('requestfailed', (request) => {
    if (new URL(request.url()).origin !== previewOrigin) return;
    problems.push(
      `requestfailed: ${request.method()} ${request.url()} ` +
        `(${request.failure()?.errorText ?? 'unknown error'})`,
    );
  });
  page.on('response', (response) => collectFailedResponse(response, previewOrigin, problems));
}

function collectFailedResponse(
  response: Response,
  previewOrigin: string,
  problems: string[],
): void {
  if (response.status() < 400 || new URL(response.url()).origin !== previewOrigin) return;
  problems.push(`response ${response.status()}: ${response.request().method()} ${response.url()}`);
}

async function waitForBoot(page: Page): Promise<void> {
  await page
    .locator('.boot-overlay.is-complete')
    .waitFor({ state: 'attached', timeout: BASIC_BOOT_TIMEOUT_MS });
  await page.waitForFunction(() => {
    const overlay = document.querySelector('.boot-overlay');
    if (!overlay?.classList.contains('is-complete')) return false;
    const style = getComputedStyle(overlay);
    return style.opacity === '0' && style.visibility === 'hidden';
  });
}

async function verifyProductionWorker(page: Page): Promise<void> {
  await page.waitForFunction(() => performance.getEntriesByType('resource').length > 0);
  const workers = page.workers();
  if (workers.length !== 1) {
    throw new Error(`Production page created ${workers.length} dedicated workers; expected one.`);
  }
  const workerUrl = workers[0]?.url() ?? '';
  if (!/C64WasmWorker[^/]*\.js(?:\?|$)/u.test(workerUrl)) {
    throw new Error(`Production page started an unexpected worker: "${workerUrl}".`);
  }
}

async function waitForRuntimeMetrics(
  page: Page,
  realtimePalRequired: boolean,
): Promise<RuntimeMetrics> {
  const telemetry = page.locator('[aria-label="实时执行数据"]');
  const deadline = Date.now() + 30_000;
  let latestText = '';
  while (Date.now() < deadline) {
    latestText = (await telemetry.textContent()) ?? '';
    const match = RUNTIME_METRICS_PATTERN.exec(latestText);
    if (match) {
      const framesPerSecond = Number.parseInt(match[1] ?? '', 10);
      const sampledFrames = Number.parseInt(match[2] ?? '', 10);
      if (
        framesPerSecond > 0 &&
        (!realtimePalRequired ||
          (framesPerSecond >= MINIMUM_PAL_FRAMES_PER_SECOND &&
            framesPerSecond <= MAXIMUM_PAL_FRAMES_PER_SECOND)) &&
        sampledFrames >= MINIMUM_SAMPLED_FRAMES
      ) {
        return { framesPerSecond, sampledFrames };
      }
    }
    await page.waitForTimeout(100);
  }
  const expectation = realtimePalRequired
    ? `${MINIMUM_PAL_FRAMES_PER_SECOND}-${MAXIMUM_PAL_FRAMES_PER_SECOND} PAL FPS`
    : 'positive host frame progress';
  throw new Error(`Runtime metrics did not reach ${expectation}: "${latestText.trim()}".`);
}

async function waitForProgramExecution(page: Page): Promise<number> {
  const executionData = page.locator('[aria-label="实时执行数据"]');
  const deadline = Date.now() + PROGRAM_START_TIMEOUT_MS;
  let latestText = '';

  while (Date.now() < deadline) {
    latestText = (await executionData.textContent()) ?? '';
    const match = PROGRAM_COUNTER_PATTERN.exec(latestText);
    const programCounter = match?.[1] === undefined ? Number.NaN : Number.parseInt(match[1], 16);
    if (programCounter >= 0x0801 && programCounter < 0xa000) return programCounter;
    await page.waitForTimeout(100);
  }

  throw new Error(`Bundled PRG did not execute from RAM; latest status was "${latestText}".`);
}

async function closePreviewServer(server: PreviewServer): Promise<void> {
  await new Promise<void>((resolveClose, rejectClose) => {
    server.httpServer.close((error) => {
      if (error) rejectClose(error);
      else resolveClose();
    });
  });
}

await main();
