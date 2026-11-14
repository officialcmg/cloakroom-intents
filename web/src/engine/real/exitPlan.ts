// Decisions of the 1Click exit that depend on the taker key's balance inside NEAR Intents
// (security review 6). The engine reads that balance (`mt_balance_of` at a final block)
// before each firm quote, after each final status, and before it believes a SUCCESS.
//
// Why: the status from 1Click is not signed (only the quote inside it is), a refund can
// arrive after 1Click reports it, and a refund or a fee can leave less than the payout. The
// Verifier balance is the one fact that only the taker key can change.

/** How long a `done` swap restored after its exit keeps watching for a refund: the longest a signed exit stays valid (request deadline + 72 h) and a margin. */
export const REFUND_WATCH_MS = 96 * 3600_000;

export type AmountPlan =
  /** Quote and sign this amount, for at least this minimum. */
  | { kind: 'quote'; amountIn: bigint; minAmountOut: bigint }
  /** Nothing under the key: an earlier signed exit moved it, or a refund is still on its way. */
  | { kind: 'empty' };

const ceilDiv = (a: bigint, b: bigint) => (a + b - 1n) / b;

/**
 * The amount of the next exit attempt: the payout, or less if less is under the key (a
 * refund that came back short). Never more than the payout, so an exit never moves tokens
 * that were not part of this swap. The approved minimum is for `minimumFor` (the payout,
 * or the amount of an offer the user accepted); for another amount it is scaled at the same
 * rate, rounded up.
 */
export function planExitAmount(payout: bigint, approvedMinimum: bigint, minimumFor: bigint, balance: bigint): AmountPlan {
  if (balance <= 0n || payout <= 0n) return { kind: 'empty' };
  const amountIn = balance < payout ? balance : payout;
  const base = minimumFor > 0n ? minimumFor : payout;
  const minAmountOut = amountIn === base ? approvedMinimum : ceilDiv(approvedMinimum * amountIn, base);
  return { kind: 'quote', amountIn, minAmountOut };
}

/**
 * Whether a SUCCESS from 1Click for a deposit address that this key signed `signed` for can
 * be true: the tokens must have left the key. A balance still at or above the signed amount
 * means the signed transfer did not run (the answer is wrong, or the view is behind).
 */
export function believeSuccess(balance: bigint, signed: bigint): boolean {
  return balance < signed;
}

/**
 * After 1Click reports REFUNDED or FAILED: `back` when tokens are under the key again (the
 * user may try again), `wait` when they are not there yet (a refund still on its way, or a
 * deposit that 1Click holds). The engine keeps watching in `wait`.
 */
export function afterFinalStatus(balance: bigint): 'back' | 'wait' {
  return balance > 0n ? 'back' : 'wait';
}

/**
 * A swap restored from a recovery file whose payout already left the taker key, on a
 * network with the 1Click exit: follow the last deposit address the file says this key
 * signed for; with none, the exit ran in another browser: `done`, and watch the balance for
 * a refund until REFUND_WATCH_MS has passed.
 */
export function restoredExit(signedDeposits: readonly string[] | undefined): { kind: 'follow'; depositAddress: string } | { kind: 'watch' } {
  const last = signedDeposits?.[signedDeposits.length - 1];
  return last ? { kind: 'follow', depositAddress: last } : { kind: 'watch' };
}
