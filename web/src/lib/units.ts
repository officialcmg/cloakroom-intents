// Amounts are integers in base units (bigint). Strings are only for input and display.
// Display always rounds down, so a shown minimum is never higher than the real one.

export const ZEC_DECIMALS = 8;
export const USDC_DECIMALS = 6;

export type ParseResult = { ok: true; value: bigint } | { ok: false; reason: 'empty' | 'invalid' | 'too_precise' };

/** Parses a decimal string such as "1.25" or "1,25" into base units. */
export function parseUnits(input: string, decimals: number): ParseResult {
  let s = input.trim().replace(/[\s_']/g, '');
  if (s === '') return { ok: false, reason: 'empty' };
  if (!s.includes('.') && /^\d*,\d+$/.test(s)) s = s.replace(',', '.');
  if (!/^(\d+\.?\d*|\.\d+)$/.test(s)) return { ok: false, reason: 'invalid' };
  const [whole, frac = ''] = s.split('.');
  if (frac.length > decimals) return { ok: false, reason: 'too_precise' };
  const value = BigInt((whole || '0') + frac.padEnd(decimals, '0'));
  return { ok: true, value };
}

/** Exact decimal representation, with trailing zeros removed. */
export function toDecimalString(value: bigint, decimals: number): string {
  const neg = value < 0n;
  const v = neg ? -value : value;
  const base = 10n ** BigInt(decimals);
  const whole = v / base;
  let frac = decimals > 0 ? (v % base).toString().padStart(decimals, '0') : '';
  frac = frac.replace(/0+$/, '');
  return `${neg ? '-' : ''}${whole}${frac ? '.' + frac : ''}`;
}

function group(whole: string): string {
  return whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

export interface FormatOptions {
  /** Most fraction digits to show. */
  maxFrac?: number;
  /** Least fraction digits to show (pads with zeros). */
  minFrac?: number;
  /** For values below 1: how many significant digits to keep. */
  significant?: number;
}

/**
 * Human display, rounded down. Large values get fewer decimals; small values keep
 * `significant` significant digits so that 0.00000123 does not show as 0.
 */
export function formatUnits(value: bigint, decimals: number, opts: FormatOptions = {}): string {
  const neg = value < 0n;
  const v = neg ? -value : value;
  const base = 10n ** BigInt(decimals);
  const whole = v / base;
  const fracDigits = decimals > 0 ? (v % base).toString().padStart(decimals, '0') : '';
  let maxFrac = opts.maxFrac;
  if (maxFrac === undefined) {
    if (whole >= 100000n) maxFrac = 0;
    else if (whole >= 1000n) maxFrac = 2;
    else if (whole >= 1n) maxFrac = 4;
    else {
      const firstNonZero = fracDigits.search(/[1-9]/);
      maxFrac = firstNonZero === -1 ? 2 : firstNonZero + (opts.significant ?? 4);
    }
  }
  maxFrac = Math.min(maxFrac, decimals);
  let frac = fracDigits.slice(0, maxFrac).replace(/0+$/, '');
  const minFrac = Math.min(opts.minFrac ?? 0, maxFrac);
  if (frac.length < minFrac) frac = frac.padEnd(minFrac, '0');
  return `${neg ? '-' : ''}${group(whole.toString())}${frac ? '.' + frac : ''}`;
}

export const formatZec = (zats: bigint, maxFrac = 8) => formatUnits(zats, ZEC_DECIMALS, { maxFrac, minFrac: 2 });
export const formatUsdc = (micro: bigint) => formatUnits(micro, USDC_DECIMALS, { maxFrac: 2, minFrac: 2 });

/** a * num / den, rounded down. */
export function mulDiv(a: bigint, num: bigint, den: bigint): bigint {
  if (den === 0n) throw new Error('division by zero');
  return (a * num) / den;
}

/** Floating-point view of an amount, only for percentages and comparisons in display. */
export function toNumber(value: bigint, decimals: number): number {
  return Number(toDecimalString(value, decimals));
}
