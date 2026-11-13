// ZIP 321 payment request URIs: zcash:<address>?amount=<ZEC>
import { toDecimalString, ZEC_DECIMALS } from './units';

export function paymentUri(address: string, amountZats: bigint, message?: string): string {
  if (amountZats <= 0n) throw new Error('amount must be positive');
  const params = [`amount=${toDecimalString(amountZats, ZEC_DECIMALS)}`];
  if (message) params.push(`message=${encodeURIComponent(message)}`);
  return `zcash:${address}?${params.join('&')}`;
}

/** Reads the address and amount back from a single-payment ZIP 321 URI. */
export function parsePaymentUri(uri: string): { address: string; amount: string } | null {
  const m = /^zcash:([^?]+)\?(.*)$/.exec(uri);
  if (!m) return null;
  const amount = new URLSearchParams(m[2]).get('amount');
  if (!amount || !/^\d+(\.\d{1,8})?$/.test(amount)) return null;
  return { address: m[1], amount };
}
