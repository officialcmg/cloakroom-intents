// The 1Click proxy (web/server/oneclickProxy.ts, docs/ONECLICK_PROXY.md), run directly with
// a mocked upstream: the allowlist, the key header, the no-key mode, the body cap, the
// same-site rule, the headers that pass, and that the upstream answer passes unchanged.
import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { handleOneClick, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, ROUTES, UPSTREAM, type Endpoint, type ProxyDeps } from '../../server/oneclickProxy';

const SITE = 'https://site.example';
const KEY = 'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.c2lnbmF0dXJl';

interface Seen {
  url: string;
  init: RequestInit;
}

function upstream(answer: () => Response = () => Response.json({ ok: true })): { deps: (env?: Record<string, string | undefined>) => ProxyDeps; seen: Seen[] } {
  const seen: Seen[] = [];
  const f = (async (url: string | URL | Request, init?: RequestInit) => {
    seen.push({ url: String(url), init: init ?? {} });
    return answer();
  }) as typeof fetch;
  return { seen, deps: (env = {}) => ({ env, fetch: f }) };
}

const get = (path: string, headers: Record<string, string> = {}) => new Request(`${SITE}/api/oneclick/v0/${path}`, { headers: { host: 'site.example', ...headers } });
const post = (path: string, body: string, headers: Record<string, string> = {}) =>
  new Request(`${SITE}/api/oneclick/v0/${path}`, { method: 'POST', body, headers: { host: 'site.example', 'content-type': 'application/json', origin: SITE, 'sec-fetch-site': 'same-origin', ...headers } });

const sentHeaders = (s: Seen) => Object.fromEntries(new Headers(s.init.headers).entries());

describe('1Click proxy', () => {
  it('has one function file per allowlisted call and nothing else', () => {
    const files = readdirSync(new URL('../../api/oneclick/v0/', import.meta.url)).sort();
    expect(files).toEqual(Object.keys(ROUTES).map((e) => `${e}.ts`).sort());
    for (const f of files) {
      const src = readFileSync(new URL(`../../api/oneclick/v0/${f}`, import.meta.url), 'utf8');
      expect(src).toContain(`handleOneClick(request, '${f.replace(/\.ts$/, '')}')`);
    }
  });

  it('forwards each allowlisted call to the fixed upstream with its method and path', async () => {
    for (const [e, r] of Object.entries(ROUTES) as [Endpoint, (typeof ROUTES)[Endpoint]][]) {
      const u = upstream();
      const req = r.method === 'GET' ? get(e) : post(e, '{}');
      const res = await handleOneClick(req, e, u.deps());
      expect(res.status, e).toBe(200);
      expect(u.seen).toHaveLength(1);
      expect(u.seen[0].url).toBe(`${UPSTREAM}${r.path}`);
      expect(u.seen[0].init.method).toBe(r.method);
      expect(u.seen[0].init.redirect).toBe('manual');
    }
  });

  it('refuses other methods and unknown endpoints without calling upstream', async () => {
    const u = upstream();
    expect((await handleOneClick(post('tokens', '{}'), 'tokens', u.deps())).status).toBe(405);
    expect((await handleOneClick(get('quote'), 'quote', u.deps())).status).toBe(405);
    const del = new Request(`${SITE}/api/oneclick/v0/status`, { method: 'DELETE' });
    expect((await handleOneClick(del, 'status', u.deps())).status).toBe(405);
    const opt = new Request(`${SITE}/api/oneclick/v0/quote`, { method: 'OPTIONS', headers: { origin: 'https://evil.example' } });
    const r = await handleOneClick(opt, 'quote', u.deps());
    expect(r.status).toBe(405);
    expect(r.headers.get('access-control-allow-origin')).toBeNull();
    expect((await handleOneClick(get('x'), 'orders' as Endpoint, u.deps())).status).toBe(404);
    expect((await handleOneClick(get('x'), 'constructor' as Endpoint, u.deps())).status).toBe(404);
    expect(u.seen).toHaveLength(0);
  });

  it('sends the key as X-API-Key when ONECLICK_JWT is set, and says so', async () => {
    const u = upstream();
    const res = await handleOneClick(post('quote', '{"dry":true}'), 'quote', u.deps({ ONECLICK_JWT: `  ${KEY}\n` }));
    expect(res.headers.get('x-oneclick-auth')).toBe('key');
    expect(sentHeaders(u.seen[0])['x-api-key']).toBe(KEY);
    expect(sentHeaders(u.seen[0]).authorization).toBeUndefined();
    // The key is never in the answer.
    expect(JSON.stringify([...res.headers.entries()])).not.toContain(KEY);
    expect(await res.text()).not.toContain(KEY);
  });

  it('forwards without a key when ONECLICK_JWT is unset or empty, and says so', async () => {
    for (const env of [{}, { ONECLICK_JWT: '' }, { ONECLICK_JWT: '   ' }, { ONECLICK_JWT: 'a b' }]) {
      const u = upstream();
      const res = await handleOneClick(get('tokens'), 'tokens', u.deps(env));
      expect(res.status).toBe(200);
      expect(res.headers.get('x-oneclick-auth')).toBe('none');
      expect(sentHeaders(u.seen[0])['x-api-key']).toBeUndefined();
    }
  });

  it('asks once more without the key when 1Click refuses it, and says so', async () => {
    const seen: Seen[] = [];
    const f = (async (url: string, init?: RequestInit) => {
      seen.push({ url, init: init ?? {} });
      return new Headers(init?.headers).has('x-api-key') ? Response.json({ message: 'Invalid token' }, { status: 401 }) : Response.json({ quote: {} }, { status: 201 });
    }) as typeof fetch;
    const res = await handleOneClick(post('quote', '{"dry":true}'), 'quote', { env: { ONECLICK_JWT: KEY }, fetch: f });
    expect(res.status).toBe(201);
    expect(res.headers.get('x-oneclick-auth')).toBe('key-rejected');
    expect(seen).toHaveLength(2);
    expect(sentHeaders(seen[1])['x-api-key']).toBeUndefined();
    expect(new TextDecoder().decode(seen[1].init.body as Uint8Array)).toBe('{"dry":true}');
    // Without a key a 401 passes as it is: nothing to retry.
    const plain = upstream(() => Response.json({ message: 'no' }, { status: 401 }));
    const r = await handleOneClick(get('tokens'), 'tokens', plain.deps());
    expect(r.status).toBe(401);
    expect(plain.seen).toHaveLength(1);
  });

  it('passes no browser header upstream: no cookie, no Authorization, no X-API-Key of the client', async () => {
    const u = upstream();
    await handleOneClick(
      post('submit-intent', '{"type":"swap_transfer"}', { cookie: 'session=1', authorization: 'Bearer client', 'x-api-key': 'client-key', 'x-forwarded-for': '1.2.3.4', 'user-agent': 'ua', referer: `${SITE}/swap` }),
      'submit-intent',
      u.deps({ ONECLICK_JWT: KEY }),
    );
    expect(sentHeaders(u.seen[0])).toEqual({ accept: 'application/json', 'content-type': 'application/json', 'x-api-key': KEY });
    expect(new TextDecoder().decode(u.seen[0].init.body as Uint8Array)).toBe('{"type":"swap_transfer"}');
  });

  it('takes only the status query parameters the API defines, and only as query values', async () => {
    const u = upstream();
    const req = get('status?depositAddress=abc%2F..%2F%40evil.example&depositMemo=7&url=https://evil.example&x=1');
    await handleOneClick(req, 'status', u.deps());
    const sent = new URL(u.seen[0].url);
    expect(sent.origin).toBe(UPSTREAM);
    expect(sent.pathname).toBe('/v0/status');
    expect([...sent.searchParams.keys()]).toEqual(['depositAddress', 'depositMemo']);
    expect(sent.searchParams.get('depositAddress')).toBe('abc/../@evil.example');
    const tok = upstream();
    await handleOneClick(get('tokens?host=evil.example'), 'tokens', tok.deps());
    expect(tok.seen[0].url).toBe(`${UPSTREAM}/v0/tokens`);
    const long = upstream();
    expect((await handleOneClick(get(`status?depositAddress=${'a'.repeat(300)}`), 'status', long.deps())).status).toBe(400);
    expect(long.seen).toHaveLength(0);
  });

  it('refuses bodies above the cap, declared or streamed, and non-JSON bodies', async () => {
    const u = upstream();
    const big = 'x'.repeat(MAX_REQUEST_BYTES + 1);
    expect((await handleOneClick(post('quote', big), 'quote', u.deps())).status).toBe(413);
    const stream = new ReadableStream<Uint8Array>({
      start(c) {
        for (let i = 0; i < 70; i++) c.enqueue(new Uint8Array(1024));
        c.close();
      },
    });
    const chunked = new Request(`${SITE}/api/oneclick/v0/quote`, { method: 'POST', body: stream, headers: { 'content-type': 'application/json' }, duplex: 'half' } as RequestInit);
    expect((await handleOneClick(chunked, 'quote', u.deps())).status).toBe(413);
    expect((await handleOneClick(post('quote', '{}', { 'content-type': 'text/plain' }), 'quote', u.deps())).status).toBe(415);
    expect((await handleOneClick(post('quote', '{}', { 'content-length': 'abc' }), 'quote', u.deps())).status).toBe(413);
    expect(u.seen).toHaveLength(0);
    expect((await handleOneClick(post('quote', 'x'.repeat(MAX_REQUEST_BYTES - 10)), 'quote', u.deps())).status).toBe(200);
  });

  it('answers pages of this site only, and never sends access-control-allow-origin', async () => {
    const u = upstream(() => new Response('{}', { headers: { 'content-type': 'application/json', 'access-control-allow-origin': '*' } }));
    const env = { ONECLICK_JWT: KEY };
    // Another site, by Sec-Fetch-Site or by Origin.
    expect((await handleOneClick(post('quote', '{}', { 'sec-fetch-site': 'cross-site' }), 'quote', u.deps(env))).status).toBe(403);
    expect((await handleOneClick(post('quote', '{}', { 'sec-fetch-site': 'same-site' }), 'quote', u.deps(env))).status).toBe(403);
    expect((await handleOneClick(post('quote', '{}', { origin: 'https://evil.example', 'sec-fetch-site': '' }), 'quote', u.deps(env))).status).toBe(403);
    expect((await handleOneClick(post('quote', '{}', { origin: 'null' }), 'quote', u.deps(env))).status).toBe(403);
    expect((await handleOneClick(get('tokens', { 'sec-fetch-site': 'cross-site' }), 'tokens', u.deps(env))).status).toBe(403);
    expect(u.seen).toHaveLength(0);
    // This site: a page (Origin and Sec-Fetch-Site), a same-origin GET (no Origin), a script without browser headers.
    const ok = await handleOneClick(post('quote', '{}'), 'quote', u.deps(env));
    expect(ok.status).toBe(200);
    expect(ok.headers.get('access-control-allow-origin')).toBeNull();
    expect((await handleOneClick(get('tokens', { 'sec-fetch-site': 'same-origin' }), 'tokens', u.deps(env))).status).toBe(200);
    expect((await handleOneClick(new Request(`${SITE}/api/oneclick/v0/tokens`), 'tokens', u.deps(env))).status).toBe(200);
    // The public host from x-forwarded-host counts as this site.
    const fwd = new Request('https://internal.vercel/api/oneclick/v0/quote', { method: 'POST', body: '{}', headers: { 'content-type': 'application/json', origin: SITE, 'x-forwarded-host': 'site.example' } });
    expect((await handleOneClick(fwd, 'quote', u.deps(env))).status).toBe(200);
  });

  it('returns the upstream status and body byte for byte, with only content-type and retry-after of its headers', async () => {
    // A signed quote is passed as bytes: the proxy cannot re-serialize or edit it.
    const body = '{"quote":{"amountOut":"4950000"},"signature":"ed25519:abc", "timestamp" : "2026-11-14T00:00:00.000Z"}';
    const u = upstream(
      () =>
        new Response(body, {
          status: 429,
          headers: { 'content-type': 'application/json; charset=utf-8', 'set-cookie': 'track=1', 'retry-after': '5', server: 'x', 'access-control-allow-origin': '*', location: 'https://evil.example' },
        }),
    );
    const res = await handleOneClick(post('quote', '{}'), 'quote', u.deps());
    expect(res.status).toBe(429);
    expect(await res.text()).toBe(body);
    expect(res.headers.get('set-cookie')).toBeNull();
    expect(res.headers.get('server')).toBeNull();
    expect(res.headers.get('location')).toBeNull();
    expect(res.headers.get('access-control-allow-origin')).toBeNull();
    expect(res.headers.get('retry-after')).toBe('5');
    expect(res.headers.get('content-type')).toBe('application/json; charset=utf-8');
    expect(res.headers.get('cache-control')).toBe('no-store');
  });

  it('serves every upstream answer as JSON, never as a type a browser renders', async () => {
    for (const t of ['text/html', 'image/svg+xml', 'application/json; x=<script>', '']) {
      const u = upstream(() => new Response('<script>alert(1)</script>', { headers: t ? { 'content-type': t } : {} }));
      const res = await handleOneClick(get('tokens'), 'tokens', u.deps());
      expect(res.headers.get('content-type'), t).toBe('application/json');
      expect(res.headers.get('x-content-type-options')).toBe('nosniff');
    }
  });

  it('lets only the token list be cached, and only when it succeeded', async () => {
    const ok = upstream();
    expect((await handleOneClick(get('tokens'), 'tokens', ok.deps())).headers.get('cache-control')).toBe('public, max-age=60, s-maxage=60');
    for (const e of ['quote', 'generate-intent', 'submit-intent'] as const) expect((await handleOneClick(post(e, '{}'), e, ok.deps())).headers.get('cache-control')).toBe('no-store');
    expect((await handleOneClick(get('status?depositAddress=a'), 'status', ok.deps())).headers.get('cache-control')).toBe('no-store');
    const down = upstream(() => Response.json({ message: 'down' }, { status: 503 }));
    expect((await handleOneClick(get('tokens'), 'tokens', down.deps())).headers.get('cache-control')).toBe('no-store');
  });

  it('never follows a redirect, and reports a failed, slow or oversized upstream as an error', async () => {
    const redirect = upstream(() => new Response(null, { status: 302, headers: { location: 'https://evil.example/' } }));
    const r = await handleOneClick(get('tokens'), 'tokens', redirect.deps());
    expect(r.status).toBe(502);
    expect(r.headers.get('location')).toBeNull();
    const fail: ProxyDeps = { env: {}, fetch: (async () => { throw new TypeError('fetch failed'); }) as typeof fetch };
    expect((await handleOneClick(get('tokens'), 'tokens', fail)).status).toBe(502);
    const slow: ProxyDeps = {
      env: {},
      timeoutMs: 20,
      fetch: ((_u: string, init?: RequestInit) =>
        new Promise((_, reject) => init?.signal?.addEventListener('abort', () => reject(init.signal!.reason)))) as typeof fetch,
    };
    expect((await handleOneClick(get('tokens'), 'tokens', slow)).status).toBe(504);
    const huge = upstream(() => new Response(new Uint8Array(MAX_RESPONSE_BYTES + 1)));
    expect((await handleOneClick(get('tokens'), 'tokens', huge.deps())).status).toBe(502);
  });

  it('answers an upstream status a Response cannot carry with an error, not a crash', async () => {
    for (const s of [204, 205]) {
      const u = upstream(() => new Response(null, { status: s }));
      const r = await handleOneClick(get('tokens'), 'tokens', u.deps({ ONECLICK_JWT: KEY }));
      expect(r.status, String(s)).toBe(502);
      expect(r.headers.get('cache-control')).toBe('no-store');
    }
  });

  it('never puts the key in an answer, on any path', async () => {
    const answers: (() => Response)[] = [
      () => Response.json({ ok: true }),
      () => Response.json({ message: 'Invalid token' }, { status: 401 }),
      () => Response.json({ message: 'busy' }, { status: 429, headers: { 'retry-after': '5' } }),
      () => new Response(null, { status: 302, headers: { location: 'https://evil.example/' } }),
      () => new Response(null, { status: 204 }),
    ];
    for (const a of answers) {
      const u = upstream(a);
      for (const r of [
        await handleOneClick(post('quote', '{}'), 'quote', u.deps({ ONECLICK_JWT: KEY })),
        await handleOneClick(get('status?depositAddress=a'), 'status', u.deps({ ONECLICK_JWT: KEY })),
        await handleOneClick(post('quote', '{}', { 'sec-fetch-site': 'cross-site' }), 'quote', u.deps({ ONECLICK_JWT: KEY })),
      ]) {
        const text = `${JSON.stringify([...r.headers.entries()])}\n${await r.text()}`;
        expect(text).not.toContain(KEY);
        expect(text).not.toContain(KEY.split('.')[2]);
      }
    }
  });

  it('never puts the key in a log line', async () => {
    const lines: string[] = [];
    const orig = { warn: console.warn, log: console.log, error: console.error, info: console.info };
    for (const k of Object.keys(orig) as (keyof typeof orig)[]) console[k] = (...a: unknown[]) => void lines.push(a.map(String).join(' '));
    try {
      const fail: ProxyDeps = { env: { ONECLICK_JWT: KEY }, fetch: (async () => { throw new Error(`boom ${KEY}`); }) as typeof fetch };
      await handleOneClick(post('submit-intent', '{"signature":"secret-body"}'), 'submit-intent', fail);
    } finally {
      Object.assign(console, orig);
    }
    expect(lines.join('\n')).not.toContain(KEY);
    expect(lines.join('\n')).not.toContain('secret-body');
  });
});
