import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import sample from '../../public/maker-directory.json';
import { formatUnits, formatZec, parseUnits, toDecimalString, mulDiv } from '../../src/lib/units';
import { parseDirectory, DirectoryError } from '../../src/lib/directory';
import { combine, computeZecSide, formatBps, makerMarginBps, premiumFor, quoteMeetsMinimum } from '../../src/lib/price';
import { parsePaymentUri, paymentUri } from '../../src/lib/zip321';
import { classifyError, parseQuote, selectDestinations } from '../../src/lib/oneclick';

const sampleTerms = parseDirectory(sample).terms;
// The price math below uses a percentage premium (0.2%, at least 0.0002 ZEC), so that both
// rules are exercised; the bundled sample has a flat premium, like the maker it describes.
const terms = { ...sampleTerms, premiumBps: 20n, premiumMinZats: 20_000n };

describe('units', () => {
  it('parses decimal input into base units', () => {
    expect(parseUnits('1.5', 8)).toEqual({ ok: true, value: 150_000_000n });
    expect(parseUnits('.25', 8)).toEqual({ ok: true, value: 25_000_000n });
    expect(parseUnits('0,1', 8)).toEqual({ ok: true, value: 10_000_000n });
    expect(parseUnits(' 2 ', 8)).toEqual({ ok: true, value: 200_000_000n });
    expect(parseUnits('', 8)).toEqual({ ok: false, reason: 'empty' });
    expect(parseUnits('1.123456789', 8)).toEqual({ ok: false, reason: 'too_precise' });
    expect(parseUnits('1e3', 8)).toEqual({ ok: false, reason: 'invalid' });
    expect(parseUnits('-1', 8)).toEqual({ ok: false, reason: 'invalid' });
    expect(parseUnits('1.2.3', 8)).toEqual({ ok: false, reason: 'invalid' });
  });
  it('formats with rounding down, grouping and small-value precision', () => {
    expect(formatUnits(123_456_789n, 6)).toBe('123.4567');
    expect(formatUnits(1_234_567_891n, 6, { maxFrac: 2 })).toBe('1,234.56');
    expect(formatUnits(199n, 8)).toBe('0.00000199');
    expect(formatUnits(61_920_531n, 9)).toBe('0.06192');
    expect(formatUnits(10n ** 24n * 5n, 18)).toBe('5,000,000');
    expect(formatZec(150_000_000n)).toBe('1.50');
    expect(formatZec(123_456_789n)).toBe('1.23456789');
    expect(toDecimalString(100_000_000n, 8)).toBe('1');
    expect(toDecimalString(1n, 8)).toBe('0.00000001');
  });
  it('mulDiv rounds down', () => {
    expect(mulDiv(10n, 1n, 3n)).toBe(3n);
  });
});

describe('maker directory', () => {
  it('reads the bundled sample', () => {
    expect(terms.network).toBe('testnet');
    expect(terms.rateMicroUsdcPerZec).toBe(940_000_000n);
    expect(terms.minZats).toBe(1_000_000n);
    expect(terms.maxZats).toBe(200_000_000n);
    expect(terms.feePerSendZats).toBe(10_000n);
  });
  it('the sample matches the testnet maker of deploy/maker/README.md and deploy/deployment.testnet.json', () => {
    const deployment = JSON.parse(readFileSync(new URL('../../../deploy/deployment.testnet.json', import.meta.url), 'utf8'));
    expect(sample.maker.nearAccount).toBe(deployment.maker.account_id);
    expect(sample.maker.escrowContract).toBe(deployment.escrow.account_id);
    // premium_zats 450000 at the example price: the smallest premium the maker's premium check passes.
    expect(sampleTerms.premiumBps).toBe(0n);
    expect(sampleTerms.premiumMinZats).toBe(450_000n);
    expect(premiumFor(sampleTerms.minZats, sampleTerms)).toBe(450_000n);
    expect(premiumFor(sampleTerms.maxZats, sampleTerms)).toBe(450_000n);
    expect(sampleTerms.rateMicroUsdcPerZec).toBe(940_000_000n); // price_per_zec 940000000
  });
  it('rejects broken files with a reason', () => {
    expect(() => parseDirectory({ ...sample, format: 'x' })).toThrow(DirectoryError);
    expect(() => parseDirectory({ ...sample, rate: { usdcPerZec: '-1', validForSeconds: 1 } })).toThrow(/rate.usdcPerZec/);
    expect(() => parseDirectory({ ...sample, limits: { minZec: '3', maxZec: '2' } })).toThrow(/maxZec/);
    expect(() => parseDirectory({ ...sample, maker: { ...sample.maker, requestKey: 'zz' } })).toThrow(/requestKey/);
    expect(() => parseDirectory(null)).toThrow(DirectoryError);
  });
});

describe('price math', () => {
  it('splits the payment into fees, premium and the ZEC that is sold', () => {
    const r = computeZecSide(100_000_000n, terms); // 1 ZEC
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const z = r.value;
    expect(z.networkFeeZats).toBe(30_000n); // 3 sends x 0.0001
    expect(z.premiumZats).toBe(200_000n); // 0.2% of 1 ZEC
    expect(z.swapZats).toBe(100_000_000n - 30_000n - 200_000n);
    // 0.9977 ZEC x 940 USDC = 937.838 USDC
    expect(z.usdcExpected).toBe(937_838_000n);
    expect(z.usdcMin).toBe(928_459_620n); // 1% below
    expect(z.networkFeeZats + z.premiumZats + z.swapZats).toBe(z.payZats);
  });
  it('uses the minimum premium for small amounts', () => {
    expect(premiumFor(1_000_000n, terms)).toBe(20_000n);
    expect(premiumFor(100_000_000n, terms)).toBe(200_000n);
  });
  it('reports limits and a paused maker', () => {
    expect(computeZecSide(999_999n, terms)).toEqual({ ok: false, error: { kind: 'below_min', minZats: 1_000_000n } });
    expect(computeZecSide(200_000_001n, terms)).toEqual({ ok: false, error: { kind: 'above_max', maxZats: 200_000_000n } });
    expect(computeZecSide(100_000_000n, { ...terms, status: 'paused' })).toEqual({ ok: false, error: { kind: 'maker_paused' } });
    const expensive = { ...terms, minZats: 1n, premiumMinZats: 50_000n };
    expect(computeZecSide(60_000n, expensive)).toEqual({ ok: false, error: { kind: 'fees_exceed_amount', costZats: 80_000n } });
  });
  it('combines with a 1Click quote; the minimum is scaled to the lowest acceptable USDC', () => {
    const r = computeZecSide(100_000_000n, terms);
    if (!r.ok) throw new Error('no');
    // Quote for 937.838 USDC -> SOL: 11.6 SOL, min 11.484 SOL (9 decimals)
    const p = combine(r.value, { amountIn: 937_838_000n, amountOut: 11_600_000_000n, minAmountOut: 11_484_000_000n, amountInUsd: 937.36, amountOutUsd: 932.6 });
    expect(p.destExpected).toBe(11_600_000_000n);
    // 11.484 x 0.99 = 11.36916
    expect(p.destMin).toBe(11_369_160_000n);
    expect(p.destMin < p.destExpected).toBe(true);
    expect(p.exitCostBps).toBe(51);
  });
  it('scales a quote fetched for a different USDC amount', () => {
    const r = computeZecSide(100_000_000n, terms);
    if (!r.ok) throw new Error('no');
    const p = combine(r.value, { amountIn: 468_919_000n, amountOut: 5_800_000_000n, minAmountOut: 5_742_000_000n });
    expect(p.destExpected).toBe(11_600_000_000n);
  });
  it('compares the maker rate with the market and formats percentages', () => {
    expect(makerMarginBps(940_000_000n, 948.32)).toBe(88);
    expect(makerMarginBps(940_000_000n, undefined)).toBeUndefined();
    expect(formatBps(88)).toBe('0.88%');
    expect(formatBps(150)).toBe('1.5%');
    expect(formatBps(100)).toBe('1%');
    expect(quoteMeetsMinimum(10n, 10n)).toBe(true);
    expect(quoteMeetsMinimum(9n, 10n)).toBe(false);
  });
});

describe('ZIP 321', () => {
  it('builds and reads a payment URI', () => {
    const uri = paymentUri('utest1abc', 123_450_000n);
    expect(uri).toBe('zcash:utest1abc?amount=1.2345');
    expect(parsePaymentUri(uri)).toEqual({ address: 'utest1abc', amount: '1.2345' });
    expect(paymentUri('u1x', 1n)).toBe('zcash:u1x?amount=0.00000001');
    expect(() => paymentUri('u1x', 0n)).toThrow();
  });
});

describe('1Click responses', () => {
  it('parses a quote as returned on 2026-11-13', () => {
    const body = {
      quote: { amountIn: '5000000', amountInUsd: '4.997460000000', amountOut: '61920531', amountOutUsd: '4.970361023370', minAmountOut: '61301325', timeEstimate: 9 },
    };
    const q = parseQuote(body, '2026-11-13T03:00:00Z');
    expect(q.amountOut).toBe(61_920_531n);
    expect(q.minAmountOut).toBe(61_301_325n);
    expect(q.timeEstimateS).toBe(9);
    expect(q.depositAddress).toBeUndefined();
    expect(q.feeBps).toBeUndefined();
    const withFees = parseQuote({ ...body, quoteRequest: { appFees: [{ recipient: '5880ad2b', fee: 20 }] } }, 'x');
    expect(withFees.feeBps).toBe(20);
    expect(() => parseQuote({}, 'x')).toThrow();
  });
  it('classifies error messages', () => {
    expect(classifyError(400, 'recipient is not valid').kind).toBe('recipient');
    const low = classifyError(400, 'Amount is too low for bridge, try at least 82594');
    expect(low.kind).toBe('too_low');
    expect(low.minAmountIn).toBe(82_594n);
    expect(classifyError(400, 'No liquidity available').kind).toBe('no_liquidity');
    expect(classifyError(400, 'Quoting for this pair is not available').kind).toBe('busy');
  });
  it('keeps supported chains and drops deprecated, Zcash and USDC on NEAR', () => {
    const list = selectDestinations([
      { assetId: 'nep141:zec.omft.near', symbol: 'ZEC', blockchain: 'zec', decimals: 8, price: 948.32 },
      { assetId: 'nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1', symbol: 'USDC', blockchain: 'near', decimals: 6 },
      { assetId: 'nep141:sol.omft.near', symbol: 'SOL', blockchain: 'sol', decimals: 9 },
      { assetId: 'a', symbol: 'BNB(DEPRECATED)', blockchain: 'bsc', decimals: 18 },
      { assetId: 'b', symbol: 'QTC', blockchain: 'qtc', decimals: 12 },
      { assetId: 'c', symbol: 'USDC', blockchain: 'base', decimals: 6 },
      { bad: true },
    ]);
    expect(list.zecUsd).toBe(948.32);
    expect(list.destinations.map((t) => `${t.symbol}/${t.blockchain}`)).toEqual(['USDC/base', 'SOL/sol']);
  });
});
