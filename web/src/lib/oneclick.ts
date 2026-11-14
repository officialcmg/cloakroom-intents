// NEAR Intents 1Click API: the token list and quotes for the exit (USDC on NEAR -> any token).
// The browser sends no API key. The base URL comes from the network configuration
// (netconfig.ts oneClickBase): 1Click itself, or the site's proxy that adds the partner key
// (docs/ONECLICK_PROXY.md). Docs: https://docs.near-intents.org/llms.txt
import { chainInfo } from './address/chains';

export const USDC_NEAR_ASSET = 'nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1';
export const ZEC_NEAR_ASSET = 'nep141:zec.omft.near';
/** 1Click's own slippage for the exit, in basis points. */
export const EXIT_SLIPPAGE_BPS = 100;

export interface Token {
  assetId: string;
  symbol: string;
  blockchain: string;
  decimals: number;
  price?: number;
  contractAddress?: string;
}

export interface TokenList {
  /** Tokens the website offers as a destination. */
  destinations: Token[];
  /** Market price of ZEC in USD, for comparison with the maker's rate. */
  zecUsd?: number;
}

const POPULAR = ['USDC', 'USDT', 'ETH', 'BTC', 'SOL', 'NEAR', 'USDT0', 'WBTC', 'cbBTC', 'XRP', 'TRX', 'TON', 'SUI', 'APT', 'ADA', 'DOGE', 'LTC', 'BNB', 'AVAX', 'POL', 'DAI'];
const CHAIN_ORDER = ['eth', 'base', 'sol', 'arb', 'near', 'btc', 'op', 'pol', 'bsc', 'tron', 'ton', 'avax'];

function rank(t: Token): [number, number, string] {
  const p = POPULAR.indexOf(t.symbol);
  const c = CHAIN_ORDER.indexOf(t.blockchain);
  return [p === -1 ? 99 : p, c === -1 ? 99 : c, t.symbol.toLowerCase()];
}

export function compareTokens(a: Token, b: Token): number {
  const ra = rank(a);
  const rb = rank(b);
  return ra[0] - rb[0] || ra[1] - rb[1] || ra[2].localeCompare(rb[2]);
}

/** Keeps tokens on chains the page can check addresses for; drops deprecated entries, Zcash itself and USDC on NEAR. */
export function selectDestinations(raw: unknown): TokenList {
  if (!Array.isArray(raw)) throw new OneClickError('The token list has an unexpected shape.', 0, 'other');
  let zecUsd: number | undefined;
  const destinations: Token[] = [];
  for (const r of raw as Record<string, unknown>[]) {
    if (typeof r?.assetId !== 'string' || typeof r.symbol !== 'string' || typeof r.blockchain !== 'string' || typeof r.decimals !== 'number') continue;
    const t: Token = {
      assetId: r.assetId,
      symbol: r.symbol,
      blockchain: r.blockchain,
      decimals: r.decimals,
      price: typeof r.price === 'number' ? r.price : undefined,
      contractAddress: typeof r.contractAddress === 'string' ? r.contractAddress : undefined,
    };
    if (t.assetId === ZEC_NEAR_ASSET) zecUsd = t.price;
    if (t.blockchain === 'zec' || t.assetId === USDC_NEAR_ASSET) continue;
    if (/deprecated/i.test(t.symbol)) continue;
    if (!chainInfo(t.blockchain)) continue;
    destinations.push(t);
  }
  destinations.sort(compareTokens);
  return { destinations, zecUsd };
}

export type OneClickErrorKind = 'recipient' | 'too_low' | 'no_liquidity' | 'busy' | 'network' | 'other';

export class OneClickError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly kind: OneClickErrorKind,
    /** For `too_low`: the smallest amountIn (micro-USDC) that 1Click named. */
    readonly minAmountIn?: bigint,
  ) {
    super(message);
  }
}

export function classifyError(status: number, message: string): OneClickError {
  const low = /too low.*?(\d+)/i.exec(message);
  if (low) return new OneClickError(message, status, 'too_low', BigInt(low[1]));
  if (/recipient/i.test(message)) return new OneClickError(message, status, 'recipient');
  // Seen for about 20 s at a time on 2026-11-13 (docs/INTENTS.md §5); it clears by itself.
  if (/quoting for this pair is not available/i.test(message)) return new OneClickError(message, status, 'busy');
  if (/liquidity/i.test(message)) return new OneClickError(message, status, 'no_liquidity');
  return new OneClickError(message || `1Click answered ${status}`, status, 'other');
}

async function call(base: string, path: string, init: RequestInit | undefined, signal?: AbortSignal): Promise<unknown> {
  let res: Response;
  try {
    res = await fetch(`${base}${path}`, { ...init, signal });
  } catch (e) {
    if ((e as Error).name === 'AbortError') throw e;
    throw new OneClickError('NEAR Intents could not be reached. Check your connection.', 0, 'network');
  }
  const body: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const msg = typeof (body as { message?: unknown })?.message === 'string' ? (body as { message: string }).message : '';
    throw classifyError(res.status, msg);
  }
  return body;
}

/** `base`: the absolute 1Click base URL (netconfig.ts oneClickBase). */
export async function fetchTokens(base: string, signal?: AbortSignal): Promise<TokenList> {
  return selectDestinations(await call(base, '/v0/tokens', undefined, signal));
}

export interface ExitQuoteRequest {
  /** micro-USDC */
  amountIn: bigint;
  destinationAsset: string;
  recipient: string;
  /** The Intents account that gets the USDC back if the exit fails (the taker's key, 64 hex). */
  refundTo: string;
  dry: boolean;
  deadline?: Date;
  slippageBps?: number;
}

export interface ExitQuote {
  amountIn: bigint;
  amountOut: bigint;
  minAmountOut: bigint;
  amountInUsd?: number;
  amountOutUsd?: number;
  timeEstimateS?: number;
  /** Fees 1Click added to this quote (quoteRequest.appFees), in basis points. Already inside amountOut. */
  feeBps?: number;
  /** Present on a real (not dry) quote: where the signed transfer goes. */
  depositAddress?: string;
  deadline: string;
  fetchedAt: number;
}

/** A neutral Intents account for dry quotes made before the swap's own key exists. */
export const DRY_REFUND_ACCOUNT = '0'.repeat(64);

function big(v: unknown, field: string): bigint {
  if (typeof v !== 'string' || !/^\d+$/.test(v)) throw new OneClickError(`The quote has no valid ${field}.`, 0, 'other');
  return BigInt(v);
}

export function parseQuote(body: unknown, deadline: string): ExitQuote {
  const q = (body as { quote?: Record<string, unknown> })?.quote;
  if (!q) throw new OneClickError('The quote has an unexpected shape.', 0, 'other');
  const fees = (body as { quoteRequest?: { appFees?: { fee?: unknown }[] } }).quoteRequest?.appFees;
  const feeBps = Array.isArray(fees) ? fees.reduce((sum, f) => sum + (typeof f?.fee === 'number' ? f.fee : 0), 0) : undefined;
  const usd = (v: unknown) => (typeof v === 'string' || typeof v === 'number' ? Number(v) : undefined);
  return {
    amountIn: big(q.amountIn, 'amountIn'),
    amountOut: big(q.amountOut, 'amountOut'),
    minAmountOut: big(q.minAmountOut, 'minAmountOut'),
    amountInUsd: usd(q.amountInUsd),
    amountOutUsd: usd(q.amountOutUsd),
    timeEstimateS: typeof q.timeEstimate === 'number' ? q.timeEstimate : undefined,
    feeBps,
    depositAddress: typeof q.depositAddress === 'string' ? q.depositAddress : undefined,
    deadline,
    fetchedAt: Date.now(),
  };
}

export async function fetchExitQuote(base: string, req: ExitQuoteRequest, signal?: AbortSignal): Promise<ExitQuote> {
  const deadline = (req.deadline ?? new Date(Date.now() + 30 * 60 * 1000)).toISOString();
  const body = {
    dry: req.dry,
    swapType: 'EXACT_INPUT',
    slippageTolerance: req.slippageBps ?? EXIT_SLIPPAGE_BPS,
    originAsset: USDC_NEAR_ASSET,
    depositType: 'INTENTS',
    destinationAsset: req.destinationAsset,
    amount: req.amountIn.toString(),
    refundTo: req.refundTo,
    refundType: 'INTENTS',
    recipient: req.recipient,
    recipientType: 'DESTINATION_CHAIN',
    deadline,
  };
  const res = await call(base, '/v0/quote', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }, signal);
  return parseQuote(res, deadline);
}

/** Link to the NEAR Intents explorer for a 1Click deposit address. */
export function intentsExplorerUrl(depositAddress: string): string {
  return `https://explorer.near-intents.org/transactions/${depositAddress}`;
}
