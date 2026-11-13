// A stand-in for the NEAR Intents 1Click API, built from a real token list
// (fetched 2026-11-13). Quotes are priced from the token list with a 0.5% cost and
// 1% slippage, like real quotes of that day.
import type { Page, Route } from '@playwright/test';
import tokens from './fixtures/tokens.json' with { type: 'json' };

type Tok = { assetId: string; decimals: number; price?: number; symbol: string; blockchain: string };

export interface MockOptions {
  /** Recipients that 1Click rejects. */
  rejectRecipients?: string[];
  /** Multiplier for quotes after the first N quote calls (to simulate a falling price). */
  priceFactor?: number;
  /** Fail /v0/tokens. */
  tokensDown?: boolean;
  /** Error messages answered (HTTP 400) to the first quote calls, one per call. */
  quoteErrors?: string[];
  /** Delay for each response, in ms. */
  delayMs?: number;
}

export interface MockHandle {
  quotes: { amount: string; destinationAsset: string; recipient: string; dry: boolean }[];
}

const TOKENS = tokens as Tok[];

export async function mockOneClick(page: Page, opts: MockOptions = {}): Promise<MockHandle> {
  const handle: MockHandle = { quotes: [] };
  await page.route('https://1click.chaindefuser.com/**', async (route: Route) => {
    const req = route.request();
    const url = new URL(req.url());
    if (opts.delayMs) await new Promise((r) => setTimeout(r, opts.delayMs));
    const json = (status: number, body: unknown) =>
      route.fulfill({ status, contentType: 'application/json', headers: { 'access-control-allow-origin': '*' }, body: JSON.stringify(body) });
    if (req.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': 'content-type' } });
    if (url.pathname === '/v0/tokens') {
      if (opts.tokensDown) return json(503, { message: 'Service unavailable' });
      return json(200, TOKENS);
    }
    if (url.pathname === '/v0/quote') {
      const body = req.postDataJSON();
      handle.quotes.push({ amount: body.amount, destinationAsset: body.destinationAsset, recipient: body.recipient, dry: body.dry });
      if (opts.rejectRecipients?.includes(body.recipient)) return json(400, { message: 'recipient is not valid' });
      const scripted = opts.quoteErrors?.shift();
      if (scripted) return json(400, { message: scripted });
      const t = TOKENS.find((x) => x.assetId === body.destinationAsset);
      if (!t || !t.price) return json(400, { message: 'No liquidity available' });
      const usd = Number(body.amount) / 1e6;
      const factor = opts.priceFactor ?? 1;
      const outUnits = ((usd * 0.995 * factor) / t.price) * 10 ** t.decimals;
      const out = BigInt(Math.floor(outUnits));
      const min = (out * 99n) / 100n;
      return json(201, {
        quote: {
          amountIn: body.amount,
          amountInUsd: usd.toFixed(6),
          amountOut: out.toString(),
          amountOutUsd: (usd * 0.995 * factor).toFixed(6),
          minAmountOut: min.toString(),
          timeEstimate: 20,
          ...(body.dry ? {} : { depositAddress: 'f'.repeat(64) }),
        },
        // Without an API key, 1Click adds an app fee of 1 bp (stablecoins) to 20 bp.
        quoteRequest: { ...body, appFees: [{ recipient: '5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd', fee: /USD/.test(t.symbol) ? 1 : 20 }] },
        timestamp: new Date().toISOString(),
      });
    }
    return json(404, { message: 'not mocked' });
  });
  return handle;
}
