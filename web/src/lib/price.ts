// The one price the user sees. It has two parts:
//   (a) ZEC -> USDC at the maker's rate, after the Zcash network fees and the maker's premium;
//   (b) USDC -> destination token, from a NEAR Intents 1Click quote.
// All math is integer math in base units and rounds down.
import type { MakerTerms } from './directory';
import { mulDiv, ZEC_DECIMALS } from './units';

/** Shielded sends that the swap's temporary wallet makes on the normal path: request, acceptance, funding. */
export const SENDS_PER_SWAP = 3n;
/** How far the maker's firm quote may fall below the indicative rate before the swap stops. */
export const DEFAULT_MAKER_TOLERANCE_BPS = 100n;

const ONE_ZEC = 10n ** BigInt(ZEC_DECIMALS);

export type ZecSideError =
  | { kind: 'below_min'; minZats: bigint }
  | { kind: 'above_max'; maxZats: bigint }
  | { kind: 'fees_exceed_amount'; costZats: bigint }
  | { kind: 'maker_paused' };

export interface ZecSide {
  /** What the user pays into the temporary wallet. */
  payZats: bigint;
  /** Zcash network fees for the three sends. */
  networkFeeZats: bigint;
  /** Paid to the maker with the acceptance memo. */
  premiumZats: bigint;
  /** ZEC that goes to the shared address and is sold to the maker. */
  swapZats: bigint;
  /** USDC at the maker's indicative rate (micro-USDC). */
  usdcExpected: bigint;
  /** The lowest firm quote the engine accepts (micro-USDC). */
  usdcMin: bigint;
  toleranceBps: bigint;
}

export function premiumFor(payZats: bigint, terms: MakerTerms): bigint {
  const byBps = mulDiv(payZats, terms.premiumBps, 10000n);
  return byBps > terms.premiumMinZats ? byBps : terms.premiumMinZats;
}

export function computeZecSide(
  payZats: bigint,
  terms: MakerTerms,
  toleranceBps: bigint = DEFAULT_MAKER_TOLERANCE_BPS,
): { ok: true; value: ZecSide } | { ok: false; error: ZecSideError } {
  if (terms.status !== 'online') return { ok: false, error: { kind: 'maker_paused' } };
  if (payZats < terms.minZats) return { ok: false, error: { kind: 'below_min', minZats: terms.minZats } };
  if (payZats > terms.maxZats) return { ok: false, error: { kind: 'above_max', maxZats: terms.maxZats } };
  const networkFeeZats = terms.feePerSendZats * SENDS_PER_SWAP;
  const premiumZats = premiumFor(payZats, terms);
  const swapZats = payZats - networkFeeZats - premiumZats;
  if (swapZats <= 0n) return { ok: false, error: { kind: 'fees_exceed_amount', costZats: networkFeeZats + premiumZats } };
  const usdcExpected = mulDiv(swapZats, terms.rateMicroUsdcPerZec, ONE_ZEC);
  const usdcMin = mulDiv(usdcExpected, 10000n - toleranceBps, 10000n);
  return { ok: true, value: { payZats, networkFeeZats, premiumZats, swapZats, usdcExpected, usdcMin, toleranceBps } };
}

/** The part of a 1Click quote that the price needs (all base units of the destination token, except amountIn). */
export interface ExitQuoteAmounts {
  /** micro-USDC sent into the quote */
  amountIn: bigint;
  amountOut: bigint;
  minAmountOut: bigint;
  amountInUsd?: number;
  amountOutUsd?: number;
}

export interface CombinedPrice extends ZecSide {
  /** Expected destination amount at the indicative rate and the current quote. */
  destExpected: bigint;
  /**
   * The user's minimum: the 1Click minimum for the quoted USDC, scaled down to the lowest
   * USDC the engine accepts. The engine signs no exit whose firm minAmountOut is below it
   * without the user's consent, and 1Click delivers at least the signed minAmountOut or
   * refunds. It is not a promise of delivery (security review 3, H9).
   */
  destMin: bigint;
  /** Cost of the exit through NEAR Intents in basis points of the USDC, if 1Click gave USD values. */
  exitCostBps?: number;
}

export function combine(zec: ZecSide, quote: ExitQuoteAmounts): CombinedPrice {
  if (quote.amountIn <= 0n) throw new Error('quote amountIn must be positive');
  const destExpected = mulDiv(quote.amountOut, zec.usdcExpected, quote.amountIn);
  const destMin = mulDiv(quote.minAmountOut, zec.usdcMin, quote.amountIn);
  let exitCostBps: number | undefined;
  if (quote.amountInUsd && quote.amountOutUsd && quote.amountInUsd > 0) {
    exitCostBps = Math.max(0, Math.round((1 - quote.amountOutUsd / quote.amountInUsd) * 10000));
  }
  return { ...zec, destExpected, destMin, exitCostBps };
}

/**
 * The maker's rate against the market price of ZEC (from the 1Click token list).
 * Positive means the maker pays less than the market price.
 */
export function makerMarginBps(rateMicroUsdcPerZec: bigint, marketUsdPerZec: number | undefined): number | undefined {
  if (!marketUsdPerZec || marketUsdPerZec <= 0) return undefined;
  const rate = Number(rateMicroUsdcPerZec) / 1e6;
  return Math.round((1 - rate / marketUsdPerZec) * 10000);
}

export function formatBps(bps: number): string {
  const pct = bps / 100;
  const digits = Math.abs(pct) < 1 ? 2 : 1;
  return `${pct.toFixed(digits).replace(/\.?0+$/, '')}%`;
}

/** Firm quote check, used by the engine before it accepts the maker's quote. */
export function quoteMeetsMinimum(usdcQuoted: bigint, usdcMin: bigint): boolean {
  return usdcQuoted >= usdcMin;
}
