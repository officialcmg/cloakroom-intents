// The Content-Security-Policy in vercel.json must allow what the page and the real engine
// need, and nothing more: the inline theme script by its hash, WebAssembly compilation,
// module workers from the site, and the network hosts of the shipped configurations.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseNetworkConfig } from '../../src/lib/netconfig';

const read = (p: string) => readFileSync(new URL(`../../${p}`, import.meta.url), 'utf8');
const vercel = JSON.parse(read('vercel.json')) as { headers: { source: string; headers: { key: string; value: string }[] }[] };
const csp = vercel.headers.find((h) => h.source === '/(.*)')!.headers.find((h) => h.key === 'Content-Security-Policy')!.value;
const directive = (name: string) => (csp.split(';').map((d) => d.trim()).find((d) => d.startsWith(`${name} `)) ?? '').split(/\s+/).slice(1);

describe('Content-Security-Policy', () => {
  it('allows the inline theme script by its hash, and no other inline script', () => {
    const html = read('index.html');
    const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
    expect(scripts).toHaveLength(1);
    const hash = createHash('sha256').update(scripts[0]).digest('base64');
    expect(directive('script-src')).toContain(`'sha256-${hash}'`);
    expect(directive('script-src')).not.toContain("'unsafe-inline'");
    expect(directive('script-src')).not.toContain("'unsafe-eval'");
  });

  it('lets the engine compile WebAssembly and start workers from the site only', () => {
    expect(directive('script-src')).toContain("'wasm-unsafe-eval'");
    expect(directive('worker-src')).toEqual(["'self'"]);
    expect(directive('default-src')).toEqual(["'self'"]);
    expect(directive('frame-ancestors')).toEqual(["'none'"]);
  });

  it('lists exactly the https hosts of the testnet configuration and 1Click in connect-src', () => {
    // Driven by the configuration (review 3, H10): when public/config/testnet.json names a
    // public relay (exit.relayUrl, https), its origin must be added to connect-src. A relay
    // on this computer (http://127.0.0.1) is never allowed by the deployed site's CSP; it
    // works only from a local build served without these headers.
    const cfg = parseNetworkConfig(JSON.parse(read('public/config/testnet.json')));
    const endpoints = [cfg.zcash.lightwalletd!, cfg.near.rpc, cfg.exit.mode === 'direct' ? cfg.exit.relayUrl : cfg.exit.oneClickUrl];
    const https = endpoints.filter((u) => new URL(u).protocol === 'https:').map((u) => new URL(u).origin);
    const hosts = new Set(["'self'", 'https://1click.chaindefuser.com', ...https]);
    expect(new Set(directive('connect-src'))).toEqual(hosts);
    expect(directive('connect-src').filter((h) => h.startsWith('http:'))).toEqual([]);
  });
});
