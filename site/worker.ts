// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Sites static application worker
//
//   File:       worker.ts
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

interface AssetBinding {
  fetch(request: Request): Promise<Response>;
}

interface SitesEnvironment {
  readonly ASSETS: AssetBinding;
}

const INDEX_PATH = '/index.html';
const HTML_MEDIA_TYPE = 'text/html';

function secure(response: Response): Response {
  const headers = new Headers(response.headers);
  headers.set('Referrer-Policy', 'no-referrer');
  headers.set('X-Content-Type-Options', 'nosniff');
  headers.set('X-Frame-Options', 'DENY');
  return new Response(response.body, {
    headers,
    status: response.status,
    statusText: response.statusText,
  });
}

function acceptsHtml(request: Request): boolean {
  return request.headers.get('Accept')?.includes(HTML_MEDIA_TYPE) ?? false;
}

const worker = {
  async fetch(request: Request, environment: SitesEnvironment): Promise<Response> {
    const response = await environment.ASSETS.fetch(request);
    if (response.status !== 404 || !acceptsHtml(request)) return secure(response);

    const indexUrl = new URL(INDEX_PATH, request.url);
    const indexRequest = new Request(indexUrl, request);
    return secure(await environment.ASSETS.fetch(indexRequest));
  },
};

export default worker;
