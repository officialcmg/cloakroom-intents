// The mainnet build switch (docs/MAINNET.md). A build without VITE_ZECRET_ALLOW_MAINNET=1
// (the live testnet site) refuses mainnet from a link, from the build default and from a
// file. A build with it accepts the shipped mainnet file, whose escrow and code hash come
// from the bundle ("@build"), never from the file.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { chooseConfig, FROM_BUILD, knownConfigs, MAINNET_ALLOWED, MAINNET_ANCHORS, MAINNET_BUILD, NetConfigError, parseNetworkConfig, type MainnetAnchors } from '../../src/lib/netconfig';

const read = (p: string) => readFileSync(new URL(`../../${p}`, import.meta.url), 'utf8');
const shipped = JSON.parse(read('public/config/mainnet.json')) as { near: Record<string, unknown>; honesty: string[] };
const anchors: MainnetAnchors = { ...MAINNET_ANCHORS, escrow: 'escrow.zecret.near', escrowCodeHash: 'DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1' };
const OFF = false;
const ON = true;

describe('mainnet switch: off (the default build)', () => {
  it('this test build has the switch off, like the testnet site', () => {
    expect(MAINNET_ALLOWED).toBe(false);
    expect(MAINNET_BUILD).toBe(false);
    expect(knownConfigs()).toEqual(['local', 'testnet']);
  });

  it('refuses ?config=mainnet with a plain message, also when overrides are allowed', () => {
    for (const override of [false, true]) {
      expect(() => chooseConfig('?config=mainnet', undefined, override, OFF)).toThrow(/does not run on mainnet/);
      expect(() => chooseConfig('?config=mainnet', 'testnet', override, OFF)).toThrow(NetConfigError);
    }
  });

  it('refuses VITE_ZECRET_CONFIG=mainnet', () => {
    expect(() => chooseConfig('', 'mainnet', false, OFF)).toThrow(/does not run on mainnet/);
  });

  it('refuses any file that says mainnet, even with the right contracts', () => {
    expect(() => parseNetworkConfig(shipped, anchors, OFF)).toThrow(/does not run on mainnet/);
    expect(() => parseNetworkConfig({ ...shipped, near: { ...shipped.near, escrow: anchors.escrow, escrowCodeHash: anchors.escrowCodeHash } }, anchors, OFF)).toThrow(/does not run on mainnet/);
  });

  it('still loads testnet', () => {
    expect(chooseConfig('?config=testnet', undefined, false, OFF)?.url).toBe('/config/testnet.json');
    expect(parseNetworkConfig(JSON.parse(read('public/config/testnet.json')), MAINNET_ANCHORS, OFF).network).toBe('testnet');
  });
});

describe('mainnet switch: on (a build with VITE_ZECRET_ALLOW_MAINNET=1)', () => {
  it('takes ?config=mainnet and the build default mainnet', () => {
    expect(knownConfigs(ON)).toEqual(['local', 'testnet', 'mainnet']);
    expect(chooseConfig('?config=mainnet', undefined, false, ON)).toEqual({ url: '/config/mainnet.json', source: 'link', name: 'mainnet' });
    expect(chooseConfig('', 'mainnet', false, ON)).toEqual({ url: '/config/mainnet.json', source: 'build', name: 'mainnet' });
  });

  it('fills "@build" from the bundle and checks every contract', () => {
    expect(shipped.near.escrow).toBe(FROM_BUILD);
    expect(shipped.near.escrowCodeHash).toBe(FROM_BUILD);
    const cfg = parseNetworkConfig(shipped, anchors, ON);
    expect(cfg.network).toBe('mainnet');
    expect(cfg.near.escrow).toBe('escrow.zecret.near');
    expect(cfg.near.escrowCodeHash).toBe(anchors.escrowCodeHash);
    expect(cfg.near.token).toBe('17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1');
    expect(cfg.near.verifier).toBe('intents.near');
    expect(cfg.near.tokenDecimals).toBe(6);
    expect(cfg.exit.mode).toBe('oneclick');
    // The caller's object is not changed.
    expect(shipped.near.escrow).toBe(FROM_BUILD);
  });

  it('a mainnet build without the escrow values runs no mainnet swap', () => {
    expect(() => parseNetworkConfig(shipped, MAINNET_ANCHORS, ON)).toThrow(/no mainnet escrow/);
    expect(() => parseNetworkConfig(shipped, { ...anchors, escrow: FROM_BUILD }, ON)).toThrow(NetConfigError);
  });

  it('a file cannot name another escrow than the bundle', () => {
    expect(() => parseNetworkConfig({ ...shipped, near: { ...shipped.near, escrow: 'escrow.attacker.near' } }, anchors, ON)).toThrow(/must be escrow.zecret.near/);
  });

  it('the shipped honesty lines say real money, no audit, the untested 1Click leg and the single RPC', () => {
    const text = shipped.honesty.join(' ');
    for (const w of ['No security audit', '1Click', 'real funds', 'one NEAR RPC endpoint', 'afford to lose']) expect(text).toContain(w);
  });
});
