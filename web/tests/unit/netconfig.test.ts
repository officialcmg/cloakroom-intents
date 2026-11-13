import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { configUrl, coreNetwork, explorerUrl, NetConfigError, parseNetworkConfig } from '../../src/lib/netconfig';

const read = (p: string) => JSON.parse(readFileSync(new URL(`../../${p}`, import.meta.url), 'utf8'));

describe('network configuration', () => {
  it('parses the shipped files', () => {
    const t = parseNetworkConfig(read('public/config/testnet.json'));
    expect(t.network).toBe('testnet');
    expect(t.exit.mode).toBe('direct');
    expect(t.near.tokenDecimals).toBe(6);
    const l = parseNetworkConfig(read('public/config/local.json'));
    expect(l.zcash.zebraRpc).toMatch(/^http:\/\/127\.0\.0\.1/);
    expect(l.near.tokenDecimals).toBe(24);
  });

  it('refuses incomplete or unsafe files', () => {
    const good = read('public/config/testnet.json');
    expect(() => parseNetworkConfig({ ...good, format: 'x' })).toThrow(NetConfigError);
    expect(() => parseNetworkConfig({ ...good, near: { ...good.near, tokenDecimals: 2 } })).toThrow(/tokenDecimals/);
    expect(() => parseNetworkConfig({ ...good, zcash: {} })).toThrow(/zebraRpc or lightwalletd/);
    expect(() => parseNetworkConfig({ ...good, network: 'mainnet' })).toThrow(/1Click/);
    expect(() => parseNetworkConfig({ ...good, exit: { mode: 'direct' } })).toThrow(/relayUrl/);
  });

  it('picks the configuration from the URL, the build, or the demo', () => {
    expect(configUrl('?config=testnet', undefined)).toBe('/config/testnet.json');
    // Paths and URLs only in a development build (see config_override.test.ts).
    expect(configUrl('?config=/run/a/config.json', undefined, true)).toBe('/run/a/config.json');
    expect(() => configUrl('?config=/run/a/config.json', undefined, false)).toThrow(NetConfigError);
    expect(configUrl('', 'local')).toBe('/config/local.json');
    expect(configUrl('?demo=1', 'local')).toBeUndefined();
    expect(configUrl('', undefined)).toBeUndefined();
  });

  it('names networks as the engine does and fills explorer links', () => {
    expect([coreNetwork('regtest'), coreNetwork('testnet'), coreNetwork('mainnet')]).toEqual(['regtest', 'test', 'main']);
    expect(explorerUrl('https://x/tx/{txid}', 'ab')).toBe('https://x/tx/ab');
    expect(explorerUrl(undefined, 'ab')).toBeUndefined();
  });
});
