// Security review 3, NV3-01: a link chooses every trust anchor of the real engine.
//
// Run: copy to web/tests/unit/, then `cd web && npx vitest run tests/unit/config_override.test.ts`.
// (The import paths below assume that location.) No network is used.
//
// What it shows: `?config=<URL>` is taken as is, and a configuration with an attacker's
// escrow, code hash, NEAR RPC, light-wallet server, Verifier and relay passes
// `parseNetworkConfig`. `?directory=<URL>` replaces the configuration's directory too.
// In the browser the only thing between such a link and the engine is the deployed
// Content-Security-Policy `connect-src` (web/vercel.json), which blocks the fetch of a
// cross-origin file. Where the site is served without that header (vite preview, another
// host, a mirror), the link works. These tests pass on the reviewed code; they document
// the behaviour, they do not fail.
import { describe, expect, it } from 'vitest';
import { configUrl, parseNetworkConfig } from '../../src/lib/netconfig';
import { directoryUrl } from '../../src/lib/directory';

const hostile = {
  format: 'zecret-config',
  version: 1,
  network: 'testnet',
  label: 'Zcash testnet and NEAR testnet',
  directoryUrl: 'https://attacker.example/dir.json',
  zcash: { lightwalletd: 'https://attacker.example/lwd' },
  near: {
    rpc: 'https://attacker.example/near',
    escrow: 'escrow.attacker.testnet',
    escrowCodeHash: 'AttackerCodeHash1111111111111111111111111111',
    token: 'usdc.attacker.testnet',
    tokenSymbol: 'test USDC',
    tokenDecimals: 6,
    verifier: 'intents.attacker.testnet',
  },
  timing: { inclusion_secs: 30, finality_secs: 10, reorg_budget: 2, fund_secs: 900, k_z: 3 },
  minStipend: '0',
  exit: { mode: 'direct', relayUrl: 'https://attacker.example/relay' },
  honesty: ['Everything here is the official deployment.'],
};

describe('NV3-01: the configuration and the directory come from the URL', () => {
  it('takes any URL from ?config=, ahead of the build-time configuration', () => {
    expect(configUrl('?config=https://attacker.example/c.json', 'testnet')).toBe('https://attacker.example/c.json');
    expect(configUrl('?config=//attacker.example/c.json', 'testnet')).toBe('//attacker.example/c.json');
  });

  it('accepts a configuration whose every endpoint and contract is the attacker’s', () => {
    const cfg = parseNetworkConfig(hostile);
    expect(cfg.near.escrow).toBe('escrow.attacker.testnet');
    expect(cfg.near.escrowCodeHash).toBe('AttackerCodeHash1111111111111111111111111111');
    expect(cfg.near.rpc).toBe('https://attacker.example/near');
    expect(cfg.zcash.lightwalletd).toBe('https://attacker.example/lwd');
    expect(cfg.minStipend).toBe('0');
  });

  it('a mainnet configuration with http endpoints and a foreign escrow also passes', () => {
    const m = { ...hostile, network: 'mainnet', exit: { mode: 'oneclick', oneClickUrl: 'http://attacker.example/1click' }, zcash: { lightwalletd: 'http://attacker.example/lwd' } };
    expect(parseNetworkConfig(m).exit).toEqual({ mode: 'oneclick', oneClickUrl: 'http://attacker.example/1click' });
  });

  it('takes any URL from ?directory=', () => {
    expect(directoryUrl('?config=testnet&directory=https://attacker.example/dir.json')).toBe('https://attacker.example/dir.json');
  });
});
