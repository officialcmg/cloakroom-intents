// A same-origin proxy for the NEAR Intents 1Click API that adds the partner key
// (docs/ONECLICK_PROXY.md). The website's Vercel functions in web/api/oneclick/v0/ call
// handleOneClick with their fixed endpoint name.
//
// Rules:
// - Only the five calls the site and the engine make: GET /v0/tokens, POST /v0/quote,
//   POST /v0/generate-intent, POST /v0/submit-intent, GET /v0/status. The upstream host
//   is fixed; no part of the request picks it.
// - The key comes from the ONECLICK_JWT environment variable and goes upstream as
//   X-API-Key (the header the 1Click OpenAPI file recommends). Without the variable the
//   call goes upstream with no key, as the browser would send it.
// - No request header of the browser goes upstream (no cookie, no Authorization, no
//   X-API-Key). Only content-type and accept, set here.
// - Only content-type (when it is JSON) and retry-after come back from upstream; set-cookie and every
//   other upstream header are dropped. Every answer says which mode ran in
//   x-oneclick-auth: key, none, or key-rejected (1Click answered 401 to the key, and the
//   proxy asked once more without it, so a bad or expired key never stops the site).
// - Browsers on other sites are refused (Sec-Fetch-Site, Origin), and the answers carry
//   no access-control-allow-origin, so no other site can spend the key's quota.
// - Request bodies above 64 KiB, upstream answers above 4 MiB and upstream calls longer
//   than 15 s end with an error. Nothing is logged except the endpoint and the error name.
//
// The proxy is untrusted transport for the engine: the engine checks 1Click's signature
// on every quote and checks the payload before it signs (crates/zecret-intents/src/check.rs).

export const UPSTREAM = 'https://1click.chaindefuser.com';
export const KEY_ENV = 'ONECLICK_JWT';
export const MAX_REQUEST_BYTES = 64 * 1024;
export const MAX_RESPONSE_BYTES = 4 * 1024 * 1024;
export const UPSTREAM_TIMEOUT_MS = 15_000;
/** The status call's query values are deposit addresses and memos: short. */
const MAX_QUERY_VALUE = 256;

export type Endpoint = 'tokens' | 'quote' | 'generate-intent' | 'submit-intent' | 'status';

interface Route {
  method: 'GET' | 'POST';
  path: string;
  /** Query parameters forwarded upstream (others are dropped). */
  query: readonly string[];
  cache: string;
}

const NO_STORE = 'no-store';

export const ROUTES: Readonly<Record<Endpoint, Route>> = {
  // The token list changes slowly and is the same for everyone: a short shared cache.
  tokens: { method: 'GET', path: '/v0/tokens', query: [], cache: 'public, max-age=60, s-maxage=60' },
  quote: { method: 'POST', path: '/v0/quote', query: [], cache: NO_STORE },
  'generate-intent': { method: 'POST', path: '/v0/generate-intent', query: [], cache: NO_STORE },
  'submit-intent': { method: 'POST', path: '/v0/submit-intent', query: [], cache: NO_STORE },
  status: { method: 'GET', path: '/v0/status', query: ['depositAddress', 'depositMemo'], cache: NO_STORE },
};

export interface ProxyDeps {
  /** The environment (process.env on Vercel). */
  env: Record<string, string | undefined>;
  fetch: typeof fetch;
  timeoutMs?: number;
}

const defaultDeps = (): ProxyDeps => ({ env: process.env, fetch: globalThis.fetch.bind(globalThis) });

/** The partner key, or undefined when the variable is unset, empty or not a header value. */
export function partnerKey(env: Record<string, string | undefined>): string | undefined {
  const k = env[KEY_ENV]?.trim();
  if (!k || !/^[\x21-\x7e]+$/.test(k)) return undefined;
  return k;
}

export type AuthMode = 'key' | 'none' | 'key-rejected';

function reply(status: number, body: string | Uint8Array<ArrayBuffer>, mode: AuthMode, extra: Record<string, string> = {}): Response {
  const headers = new Headers({
    'content-type': 'application/json',
    'cache-control': NO_STORE,
    'x-oneclick-auth': mode,
    'x-content-type-options': 'nosniff',
    ...extra,
  });
  return new Response(body, { status, headers });
}

const message = (status: number, text: string, mode: AuthMode, extra?: Record<string, string>) => reply(status, JSON.stringify({ message: text }), mode, extra);

/**
 * Whether a browser sent this request from a page on this site. Requests without the
 * browser headers (curl, servers) pass: CORS cannot stop them; the platform's rate limit
 * can (docs/ONECLICK_PROXY.md).
 */
export function sameSite(request: Request): boolean {
  const site = request.headers.get('sec-fetch-site');
  if (site !== null && site !== 'same-origin') return false;
  const origin = request.headers.get('origin');
  if (origin === null) return true;
  let host: string;
  try {
    host = new URL(origin).host;
  } catch {
    return false;
  }
  const own = new Set<string>();
  for (const h of [request.headers.get('x-forwarded-host'), request.headers.get('host')]) if (h) own.add(h.split(',')[0].trim().toLowerCase());
  try {
    own.add(new URL(request.url).host.toLowerCase());
  } catch {
    // no usable request URL: the headers decide
  }
  return own.has(host.toLowerCase());
}

/** Reads at most `cap` bytes of a stream; undefined when it holds more. */
async function readStream(stream: ReadableStream<Uint8Array> | null, cap: number): Promise<Uint8Array<ArrayBuffer> | undefined> {
  if (!stream) return new Uint8Array();
  const reader = stream.getReader();
  const parts: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > cap) {
      await reader.cancel().catch(() => undefined);
      return undefined;
    }
    parts.push(value);
  }
  const out = new Uint8Array(size);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.byteLength;
  }
  return out;
}

/** Reads at most `cap` bytes of a request or response body; undefined when it is larger. */
async function readCapped(m: Request | Response, cap: number): Promise<Uint8Array<ArrayBuffer> | undefined> {
  const declared = m.headers.get('content-length');
  if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > cap)) {
    await m.body?.cancel().catch(() => undefined);
    return undefined;
  }
  return readStream(m.body, cap);
}

/** Forwards one allowlisted 1Click call. */
export async function handleOneClick(request: Request, endpoint: Endpoint, deps: ProxyDeps = defaultDeps()): Promise<Response> {
  const key = partnerKey(deps.env);
  let mode: AuthMode = key ? 'key' : 'none';
  const route = Object.prototype.hasOwnProperty.call(ROUTES, endpoint) ? ROUTES[endpoint] : undefined;
  if (!route) return message(404, 'Not a 1Click call this site forwards.', mode);
  if (request.method !== route.method) return message(405, `Use ${route.method}.`, mode, { allow: route.method });
  if (!sameSite(request)) return message(403, 'This proxy answers pages of this site only.', mode);

  const url = new URL(route.path, UPSTREAM);
  if (route.query.length > 0) {
    let incoming: URLSearchParams;
    try {
      incoming = new URL(request.url).searchParams;
    } catch {
      return message(400, 'The request URL cannot be read.', mode);
    }
    for (const name of route.query) {
      const v = incoming.get(name);
      if (v === null) continue;
      if (v.length > MAX_QUERY_VALUE) return message(400, `${name} is too long.`, mode);
      url.searchParams.set(name, v);
    }
  }
  if (url.origin !== UPSTREAM) return message(500, 'Upstream mismatch.', mode);

  const headers = new Headers({ accept: 'application/json' });
  if (key) headers.set('x-api-key', key);
  let body: Uint8Array<ArrayBuffer> | undefined;
  if (route.method === 'POST') {
    const type = (request.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase();
    if (type !== 'application/json') return message(415, 'Send application/json.', mode);
    body = await readCapped(request, MAX_REQUEST_BYTES);
    if (body === undefined) return message(413, `The request body is larger than ${MAX_REQUEST_BYTES} bytes.`, mode);
    headers.set('content-type', 'application/json');
  }

  const call = () =>
    deps.fetch(url.toString(), {
      method: route.method,
      headers,
      body,
      redirect: 'manual',
      signal: AbortSignal.timeout(deps.timeoutMs ?? UPSTREAM_TIMEOUT_MS),
    });
  let upstream: Response;
  try {
    upstream = await call();
    if (key && upstream.status === 401) {
      // 1Click refused the key (bad, expired or revoked). Ask once more without it, as the
      // browser would, and say so in x-oneclick-auth.
      console.warn(`oneclick proxy: ${endpoint}: 1Click refused the key (401); asking without it`);
      await upstream.body?.cancel().catch(() => undefined);
      headers.delete('x-api-key');
      mode = 'key-rejected';
      upstream = await call();
    }
  } catch (e) {
    const name = (e as Error)?.name ?? 'Error';
    console.warn(`oneclick proxy: ${endpoint}: upstream call failed (${name})`);
    return name === 'TimeoutError' ? message(504, 'NEAR Intents did not answer in time.', mode) : message(502, 'NEAR Intents could not be reached.', mode);
  }
  // A redirect is never followed: the upstream host stays fixed.
  if (upstream.status >= 300 && upstream.status < 400) {
    await upstream.body?.cancel().catch(() => undefined);
    return message(502, 'NEAR Intents answered with a redirect.', mode);
  }
  const bytes = await readCapped(upstream, MAX_RESPONSE_BYTES).catch(() => undefined);
  if (bytes === undefined) return message(502, 'The answer of NEAR Intents is too large or broke off.', mode);
  const extra: Record<string, string> = { 'cache-control': upstream.ok ? route.cache : NO_STORE };
  // Served on the site's origin: always as JSON, never as a type a browser would render.
  const type = upstream.headers.get('content-type') ?? '';
  extra['content-type'] = /^application\/json(;\s*charset=[\w-]+)?$/i.test(type) ? type : 'application/json';
  const retry = upstream.headers.get('retry-after');
  if (retry && /^[\w ,:+-]{1,64}$/.test(retry)) extra['retry-after'] = retry;
  return reply(upstream.status, bytes, mode, extra);
}
