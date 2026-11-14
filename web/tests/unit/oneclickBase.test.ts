// The 1Click base URL comes from one configuration value, oneClickUrl, for both the price
// preview and the engine's exit (docs/ONECLICK_PROXY.md). It may be 1Click itself or a path
// on this site (the proxy that adds the partner key).
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { MAINNET_ANCHORS, NetConfigError, ONECLICK_DIRECT_URL, oneClickBase, parseNetworkConfig } from '../../src/lib/netconfig';

const read = (p: string) => JSON.parse(readFileSync(new URL(`../../${p}`, import.meta.url), 'utf8'));
const testnet = read('public/config/testnet.json');
const anchors = { ...MAINNET_ANCHORS, escrow: 'escrow.zecret.near', escrowCodeHash: 'DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1' };
const mainnet = read('public/config/mainnet.json');
const parseMainnet = (j: unknown) => parseNetworkConfig(j, anchors, true);

describe('oneClickUrl', () => {
  it('defaults to 1Click itself, in a file without it and in the demo', () => {
    const { oneClickUrl, ...without } = testnet;
    expect(oneClickUrl === undefined || typeof oneClickUrl === 'string').toBe(true);
    expect(parseNetworkConfig(without).oneClickUrl).toBe(ONECLICK_DIRECT_URL);
    expect(oneClickBase(undefined, 'https://site.example')).toBe(ONECLICK_DIRECT_URL);
  });

  it('the shipped files name the same 1Click base for the preview and the exit', () => {
    for (const cfg of [parseNetworkConfig(testnet), parseMainnet(mainnet)]) {
      const base = oneClickBase(cfg, 'https://site.example');
      expect(base === ONECLICK_DIRECT_URL || base === 'https://site.example/api/oneclick').toBe(true);
    }
  });

  it('takes a path on this site, on the page origin', () => {
    const c = parseNetworkConfig({ ...testnet, oneClickUrl: '/api/oneclick/' });
    expect(c.oneClickUrl).toBe('/api/oneclick');
    expect(oneClickBase(c, 'https://site.example')).toBe('https://site.example/api/oneclick');
    expect(parseMainnet({ ...mainnet, oneClickUrl: '/api/oneclick', exit: { mode: 'oneclick' } }).oneClickUrl).toBe('/api/oneclick');
  });

  it('still reads the older exit.oneClickUrl, and moves it out of exit', () => {
    const { oneClickUrl: _top, ...older } = mainnet;
    expect(_top).toBe(ONECLICK_DIRECT_URL);
    const c = parseMainnet({ ...older, exit: { mode: 'oneclick', oneClickUrl: 'https://1click.chaindefuser.com/' } });
    expect(c.oneClickUrl).toBe(ONECLICK_DIRECT_URL);
    expect(c.exit).toEqual({ mode: 'oneclick' });
    expect(() => parseMainnet({ ...mainnet, oneClickUrl: '/api/oneclick', exit: { mode: 'oneclick', oneClickUrl: ONECLICK_DIRECT_URL } })).toThrow(/different/);
  });

  it('refuses another site by a protocol-relative path, a non-http scheme, a bare slash, a query, and http on mainnet', () => {
    for (const bad of ['//evil.example/1click', '/\\evil.example', 'javascript:alert(1)', 'ftp://x', '/', '/api/oneclick?x=1', '']) {
      expect(() => parseNetworkConfig({ ...testnet, oneClickUrl: bad }), bad).toThrow(NetConfigError);
    }
    expect(() => parseMainnet({ ...mainnet, oneClickUrl: 'http://attacker.example/1click', exit: { mode: 'oneclick' } })).toThrow(/oneClickUrl/);
    expect(parseNetworkConfig({ ...testnet, oneClickUrl: 'http://127.0.0.1:8787' }).oneClickUrl).toBe('http://127.0.0.1:8787');
  });

  it('refuses site paths that a URL parser could read as another host or another directory, and URLs with credentials or whitespace', () => {
    const bad = [
      '/\t/evil.example', '/\n/evil.example', '/ /evil.example', '/%2f/evil.example', '/%2e%2e/x', '/api/../x', '/api/./x', '/..', '/api/oneclick\u0000',
      'https://a@1click.chaindefuser.com', 'https://a:b@1click.chaindefuser.com', 'https://@1click.chaindefuser.com',
      'https://1click.chaindefuser.com\t', ' https://1click.chaindefuser.com', 'https://1click.chain\ndefuser.com',
    ];
    for (const b of bad) expect(() => parseNetworkConfig({ ...testnet, oneClickUrl: b }), JSON.stringify(b)).toThrow(NetConfigError);
    for (const b of ['/api/oneclick', '/api/one-click_v0/x~y']) expect(parseNetworkConfig({ ...testnet, oneClickUrl: b }).oneClickUrl).toBe(b);
  });
});
