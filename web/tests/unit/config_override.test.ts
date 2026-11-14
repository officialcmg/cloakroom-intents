// Security review 3, NV3-01: a link chose every trust anchor of the real engine.
// Adapted from docs/security/poc3/config_override.test.ts. The review's version documented
// that ?config=<URL>, a hostile configuration and ?directory=<URL> were all accepted; these
// tests check the fixed behaviour:
//   - a production build takes only the shipped names from ?config= (a URL, a path or an
//     unknown name is refused); a development build (or VITE_ZECRET_ALLOW_CONFIG_URL=1)
//     takes any file and marks it as an override, which the page shows in a warning;
//   - a mainnet configuration must use https and the contracts built into the bundle;
//   - ?directory= is not read when a configuration is loaded (src/app/useZecret.ts).
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { chooseConfig, configUrl, explorerUrl, KNOWN_CONFIGS, MAINNET_ANCHORS, NetConfigError, parseNetworkConfig, safeLink, type MainnetAnchors } from '../../src/lib/netconfig';
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

const PROD = false; // allowOverride in a production build
const DEV = true;

describe('NV3-01: ?config= in a production build', () => {
  it('takes only the shipped names', () => {
    expect(KNOWN_CONFIGS).toEqual(['local', 'testnet']);
    expect(configUrl('?config=testnet', undefined, PROD)).toBe('/config/testnet.json');
    expect(configUrl('?config=local', undefined, PROD)).toBe('/config/local.json');
    expect(chooseConfig('?config=testnet', undefined, PROD)?.source).toBe('link');
  });

  it('refuses a URL, a protocol-relative URL, a path and an unknown name', () => {
    for (const q of ['?config=https://attacker.example/c.json', '?config=//attacker.example/c.json', '?config=/run/a/config.json', '?config=data:application/json,{}', '?config=mainnet2', '?config=TESTNET'])
      expect(() => configUrl(q, undefined, PROD), q).toThrow(NetConfigError);
  });

  it('a link cannot replace the build-time configuration, and ?demo=1 still wins', () => {
    expect(() => configUrl('?config=https://attacker.example/c.json', 'testnet', PROD)).toThrow(NetConfigError);
    expect(configUrl('', 'testnet', PROD)).toBe('/config/testnet.json');
    expect(chooseConfig('', 'https://deploy.example/c.json', PROD)).toEqual({ url: 'https://deploy.example/c.json', source: 'build', name: 'https://deploy.example/c.json' });
    expect(configUrl('?demo=1&config=https://attacker.example/c.json', 'testnet', PROD)).toBeUndefined();
  });
});

describe('NV3-01: ?config= in a development build', () => {
  it('takes any file and marks it as an override, for the warning banner', () => {
    expect(chooseConfig('?config=/run/a/config.json', undefined, DEV)).toEqual({ url: '/run/a/config.json', source: 'override', name: '/run/a/config.json' });
    expect(chooseConfig('?config=http://127.0.0.1:9/c.json', undefined, DEV)?.source).toBe('override');
    expect(chooseConfig('?config=my-test', undefined, DEV)).toEqual({ url: '/config/my-test.json', source: 'override', name: 'my-test' });
    // A shipped name is not an override.
    expect(chooseConfig('?config=local', undefined, DEV)?.source).toBe('link');
  });
});

describe('NV3-01: what a configuration file may say', () => {
  it('a testnet file still names its own contracts (only shipped files are loaded from links)', () => {
    expect(parseNetworkConfig(hostile).near.escrow).toBe('escrow.attacker.testnet');
  });

  const anchors: MainnetAnchors = { ...MAINNET_ANCHORS, escrow: 'escrow.zecret.near', escrowCodeHash: 'DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1' };
  const mainnet = {
    ...hostile,
    network: 'mainnet',
    label: 'Mainnet',
    directoryUrl: '/maker-directory.json',
    zcash: { lightwalletd: 'https://zec.example' },
    near: { ...hostile.near, rpc: 'https://rpc.mainnet.example', escrow: anchors.escrow, escrowCodeHash: anchors.escrowCodeHash, token: anchors.token, verifier: anchors.verifier, tokenSymbol: 'USDC' },
    exit: { mode: 'oneclick', oneClickUrl: 'https://1click.chaindefuser.com' },
  };

  it('a mainnet file with the built-in contracts and https endpoints passes', () => {
    expect(parseNetworkConfig(mainnet, anchors, true).near.escrow).toBe('escrow.zecret.near');
  });

  it('a mainnet file with http endpoints, or another escrow, code hash, token or Verifier, is refused', () => {
    const bad: unknown[] = [
      { ...mainnet, exit: { mode: 'oneclick', oneClickUrl: 'http://attacker.example/1click' } },
      { ...mainnet, zcash: { lightwalletd: 'http://attacker.example/lwd' } },
      { ...mainnet, near: { ...mainnet.near, rpc: 'http://attacker.example/near' } },
      { ...mainnet, directoryUrl: 'http://attacker.example/dir.json' },
      { ...mainnet, directoryUrl: '//attacker.example/dir.json' },
      { ...mainnet, near: { ...mainnet.near, escrow: 'escrow.attacker.near' } },
      { ...mainnet, near: { ...mainnet.near, escrowCodeHash: 'AttackerCodeHash1111111111111111111111111111' } },
      { ...mainnet, near: { ...mainnet.near, token: 'usdc.attacker.near' } },
      { ...mainnet, near: { ...mainnet.near, verifier: 'intents.attacker.near' } },
    ];
    for (const b of bad) expect(() => parseNetworkConfig(b, anchors, true), JSON.stringify(b)).toThrow(NetConfigError);
  });

  it('this bundle has no mainnet escrow built in, so it refuses every mainnet file', () => {
    expect(MAINNET_ANCHORS.escrow).toBe('');
    expect(() => parseNetworkConfig(mainnet, MAINNET_ANCHORS, true)).toThrow(/no mainnet escrow/);
    // And without the build switch it refuses mainnet before it looks at the contracts.
    expect(() => parseNetworkConfig(mainnet)).toThrow(/does not run on mainnet/);
  });

  it('explorer links are https only', () => {
    expect(() => parseNetworkConfig({ ...hostile, near: { ...hostile.near, explorer: 'javascript:alert(1)//{hash}' } })).toThrow(/https/);
    expect(explorerUrl('javascript:alert(1)//{hash}', 'ab')).toBeUndefined();
    expect(explorerUrl('http://x/{hash}', 'ab')).toBeUndefined();
    expect(explorerUrl('https://x/{hash}', 'ab')).toBe('https://x/ab');
    expect(safeLink('https://basescan.org/tx/0x1')).toBe('https://basescan.org/tx/0x1');
    for (const u of ['javascript:alert(1)', 'data:text/html,x', 'http://x', '/relative', undefined, 5]) expect(safeLink(u)).toBeUndefined();
  });
});

describe('NV3-01: ?directory=', () => {
  it('is read only for the demo; with a configuration the page uses the configuration’s directory', () => {
    // directoryUrl() is called only when no configuration is loaded (useZecret.ts).
    expect(directoryUrl('?directory=https://attacker.example/dir.json')).toBe('https://attacker.example/dir.json');
    const src = readFileSync(new URL('../../src/app/useZecret.ts', import.meta.url), 'utf8');
    expect(src).toContain('fetchDirectory(network ? network.directoryUrl : directoryUrl())');
  });
});
