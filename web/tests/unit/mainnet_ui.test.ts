// The site's cap per swap, the payment range from the maker's signed limits, the network
// switch's URLs and the maker's liquidity (docs/MAINNET.md, "Swap limits" and "Network switch").
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { CreateSwapParams } from '../../src/engine/api';
import { memoryRecords } from '../../src/engine/real/idb';
import { RealEngine } from '../../src/engine/real/realEngine';
import type { WasmModule } from '../../src/engine/real/wasm';
import { effectiveTerms, parseDirectory } from '../../src/lib/directory';
import { exceedsLiquidity, readMakerLiquidity, spendableMicroUsdc } from '../../src/lib/liquidity';
import { MAINNET_ANCHORS, parseNetworkConfig, parseSiteUrl, siteCapZats } from '../../src/lib/netconfig';
import { computeZecSide } from '../../src/lib/price';

const read = (p: string) => JSON.parse(readFileSync(new URL(`../../${p}`, import.meta.url), 'utf8'));
const anchors = { ...MAINNET_ANCHORS, escrow: 'escrow.zecret.near', escrowCodeHash: 'DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1' };
const mainnetFile = read('public/config/mainnet.json');
const testnetFile = read('public/config/testnet.json');

/** A mainnet directory with the template's limits (crates/zecret-node/config/mainnet-maker.json). */
const template = read('../crates/zecret-node/config/mainnet-maker.json');
const mainnetDirectory = {
  ...read('e2e/fixtures/mainnet-directory.json'),
  maker: { ...read('e2e/fixtures/mainnet-directory.json').maker, signedLimits: { min_zats: template.min_zats, max_zats: template.max_zats, premium_zats: template.premium_zats } },
  limits: { minZec: '0.002', maxZec: '0.005' },
  premium: { bps: 0, minZec: '0.0004' },
};

describe('site cap (maxSwapZats)', () => {
  it('the shipped mainnet file caps a payment at 0.005 ZEC, and mainnet requires the cap', () => {
    const cfg = parseNetworkConfig(mainnetFile, anchors, true);
    expect(siteCapZats(cfg)).toBe(500_000n);
    const noCap = { ...mainnetFile };
    delete noCap.maxSwapZats;
    expect(() => parseNetworkConfig(noCap, anchors, true)).toThrow(/maxSwapZats is required/);
  });

  it('is optional elsewhere and must be a positive whole number', () => {
    expect(siteCapZats(parseNetworkConfig(testnetFile))).toBeUndefined();
    for (const bad of [0, '0', '-1', '1.5', 500000, 'abc', '']) expect(() => parseNetworkConfig({ ...testnetFile, maxSwapZats: bad })).toThrow(/maxSwapZats/);
    expect(siteCapZats(parseNetworkConfig({ ...testnetFile, maxSwapZats: '123' }))).toBe(123n);
  });

  it('wins over a directory that claims more, and the form refuses above it', () => {
    const { directory, terms } = parseDirectory({ ...mainnetDirectory, maker: { ...mainnetDirectory.maker, signedLimits: { min_zats: 200000, max_zats: 100_000_000, premium_zats: 40000 } }, limits: { minZec: '0.002', maxZec: '1' } });
    const t = effectiveTerms(terms, directory, 500_000n);
    expect(t.maxZats).toBe(500_000n);
    expect(t.maxBy).toBe('site');
    expect(computeZecSide(500_000n, t).ok).toBe(true);
    const r = computeZecSide(500_001n, t);
    expect(r.ok ? undefined : r.error).toEqual({ kind: 'above_max', maxZats: 500_000n });
  });
});

describe('payment range from the signed limits', () => {
  it('the mainnet template: 0.0028 to 0.005 ZEC with the site cap; at the minimum the ZEC sold equals the signed minimum', () => {
    const { directory, terms } = parseDirectory(mainnetDirectory);
    const t = effectiveTerms(terms, directory, 500_000n);
    // 200,000 sold + 4 Zcash fees of 10,000 + premium 40,000.
    expect(t.minZats).toBe(280_000n);
    expect(t.maxZats).toBe(500_000n);
    const atMin = computeZecSide(t.minZats, t);
    expect(atMin.ok && atMin.value.swapZats).toBe(200_000n);
    const atMax = computeZecSide(t.maxZats, t);
    // The largest swap sells 0.0042 ZEC, inside the maker's signed 0.005.
    expect(atMax.ok && atMax.value.swapZats).toBe(420_000n);
    // The template's premium check: premium x yoctoNEAR per ZEC / 10^8 >= stipend + costs.
    const premiumYocto = (BigInt(template.premium_zats) * BigInt(template.premium_check.yocto_near_per_zec)) / 10n ** 8n;
    expect(premiumYocto >= BigInt(template.stipend) + BigInt(template.premium_check.costs_per_request_yocto)).toBe(true);
  });

  it('without signed limits or a cap the directory range stays as it is (the demo)', () => {
    const { directory, terms } = parseDirectory(read('public/maker-directory.json'));
    const t = effectiveTerms(terms, directory);
    expect([t.minZats, t.maxZats, t.maxBy]).toEqual([terms.minZats, terms.maxZats, 'maker']);
    expect(t.demoLiquidityMicroUsdc).toBe(5_000_000_000n);
  });
});

describe('the engine refuses above the site cap', () => {
  const module = {
    default: async () => undefined,
    WasmTaker: class {
      constructor() {
        throw new Error('the engine must refuse before it opens a swap');
      }
    },
    prepareProvingKey: () => 0,
    requiredDeposit: (_z: number, premium: number) => 40_000 + premium,
    verifyMaker: () => undefined,
    checkAddress: () => true,
  } as unknown as WasmModule;

  it('with a directory that claims 1 ZEC and a cap of 0.005 ZEC', async () => {
    const cfg = parseNetworkConfig({ ...testnetFile, maxSwapZats: '500000' });
    const directory = {
      ...read('public/config/testnet-directory.json'),
      maker: { ...read('public/config/testnet-directory.json').maker, signedLimits: { min_zats: 1000, max_zats: 100_000_000, premium_zats: 1000 } },
      limits: { minZec: '0.0001', maxZec: '1' },
    };
    const engine = new RealEngine(memoryRecords(), '/', async () => module);
    await engine.init({ network: 'testnet', directory, real: cfg });
    const params: CreateSwapParams = {
      amountZats: '600000',
      destination: { assetId: 'nep141:usdc', symbol: 'USDC', chain: 'near', decimals: 6, address: '' },
      directExit: { nearAccount: 'alice.testnet' },
      minUsdc: '1',
      minDestinationAmount: '1',
      shown: { usdcExpected: '1', destinationExpected: '1', premiumZats: '1000', networkFeeZats: '40000' },
    };
    await expect(engine.createSwap(params)).rejects.toThrow(/at most 0.005 ZEC per swap/);
  });
});

describe('network switch URLs', () => {
  it('keeps only an https origin (http only for localhost), never a path or query', () => {
    expect(parseSiteUrl('https://mainnet.cloakroom.chrismg.dev/swap?x=1#y')).toBe('https://mainnet.cloakroom.chrismg.dev');
    expect(parseSiteUrl('http://localhost:4174')).toBe('http://localhost:4174');
    expect(parseSiteUrl('http://example.com')).toBeUndefined();
    expect(parseSiteUrl('javascript:alert(1)')).toBeUndefined();
    expect(parseSiteUrl('https://user:pw@example.com')).toBeUndefined();
    expect(parseSiteUrl('not a url')).toBeUndefined();
    expect(parseSiteUrl('')).toBeUndefined();
    expect(parseSiteUrl(undefined)).toBeUndefined();
  });
});

describe('maker liquidity', () => {
  it('adds the Verifier balance and the plain balance, scaled to micro-USDC', () => {
    expect(spendableMicroUsdc({ inVerifier: 58_000_000n, plain: 200_000n }, 6)).toBe(58_200_000n);
    expect(spendableMicroUsdc({ inVerifier: 10n ** 18n, plain: 5n * 10n ** 17n }, 18)).toBe(1_500_000n);
  });

  it('a swap that needs more than the maker holds is too large; an unknown liquidity blocks nothing', () => {
    expect(exceedsLiquidity(4_500_000n, 3_100_000n)).toBe(true);
    expect(exceedsLiquidity(3_100_000n, 3_100_000n)).toBe(false);
    expect(exceedsLiquidity(4_500_000n, undefined)).toBe(false);
  });

  const answer = (result: unknown) => new Response(JSON.stringify({ jsonrpc: '2.0', id: 'zecret', result }), { status: 200 });
  const bytes = (v: unknown) => [...new TextEncoder().encode(JSON.stringify(v))];

  it('reads both views at one final block, and moves to the next endpoint when one fails', async () => {
    const calls: { url: string; body: { method: string; params: Record<string, string> } }[] = [];
    const f = (async (url: string, init: RequestInit) => {
      const body = JSON.parse(init.body as string);
      calls.push({ url, body });
      if (url.includes('first')) return new Response('rate limited', { status: 429 });
      if (body.method === 'block') return answer({ header: { hash: 'H' } });
      if (body.params.method_name === 'mt_balance_of') {
        expect(JSON.parse(atob(body.params.args_base64))).toEqual({ account_id: 'maker.zecret.near', token_id: 'nep141:usdc.near' });
        expect(body.params.account_id).toBe('intents.near');
        return answer({ result: bytes('3000000') });
      }
      expect(body.params.account_id).toBe('usdc.near');
      expect(JSON.parse(atob(body.params.args_base64))).toEqual({ account_id: 'maker.zecret.near' });
      return answer({ result: bytes('100000') });
    }) as unknown as typeof fetch;
    const v = await readMakerLiquidity({ rpcs: ['https://first', 'https://second'], verifier: 'intents.near', token: 'usdc.near', maker: 'maker.zecret.near', tokenDecimals: 6 }, f);
    expect(v).toBe(3_100_000n);
    expect(calls.filter((c) => c.body.method === 'query').every((c) => c.body.params.block_id === 'H')).toBe(true);
  });

  it('fails when no endpoint answers or an answer is not an amount', async () => {
    const down = (async () => new Response('down', { status: 503 })) as unknown as typeof fetch;
    await expect(readMakerLiquidity({ rpcs: ['https://a', 'https://b'], verifier: 'v', token: 't', maker: 'm', tokenDecimals: 6 }, down)).rejects.toThrow(/503/);
    const odd = (async (_u: string, init: RequestInit) =>
      JSON.parse(init.body as string).method === 'block' ? answer({ header: { hash: 'H' } }) : answer({ result: bytes({ not: 'an amount' }) })) as unknown as typeof fetch;
    await expect(readMakerLiquidity({ rpcs: ['https://a'], verifier: 'v', token: 't', maker: 'm', tokenDecimals: 6 }, odd)).rejects.toThrow(/not an amount/);
  });
});
