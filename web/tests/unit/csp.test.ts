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

  it('lists exactly the hosts of the testnet configuration and 1Click in connect-src', () => {
    const cfg = parseNetworkConfig(JSON.parse(read('public/config/testnet.json')));
    const hosts = new Set(["'self'", 'https://1click.chaindefuser.com', new URL(cfg.zcash.lightwalletd!).origin, new URL(cfg.near.rpc).origin]);
    expect(new Set(directive('connect-src'))).toEqual(hosts);
  });
});
