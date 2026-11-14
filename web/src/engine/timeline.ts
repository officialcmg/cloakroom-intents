// Typical durations of each step of a swap, in real time. The page uses them for time
// estimates and the simulated engine uses them (divided by its speed) as its schedule.
// Sources: Zcash block target 75 s; one browser proof about 25 s and the key build about
// 20 s (docs/experiments/browser-wallet.md); NEAR finality about 10 s (docs/DESIGN.md §7).
import type { Stage } from './api';

export const ZCASH_BLOCK_MS = 75_000;
export const PROOF_MS = 26_000;
export const KEY_BUILD_MS = 20_000;
export const NEAR_FINAL_MS = 10_000;
export const FUNDING_CONFIRMATIONS = 2;
export const DEPOSIT_CONFIRMATIONS = 1;
export const STIPEND_YOCTO = '60000000000000000000000'; // 0.06 NEAR

/** Normal path, in order. */
export const NORMAL_PATH: readonly Stage[] = [
  'awaiting_deposit',
  'deposit_seen',
  'deposit_confirmed',
  'request_sent',
  'quote_received',
  'accepted',
  'escrow_open',
  'funding_sent',
  'funding_confirming',
  'ready',
  'committing',
  'claiming',
  'claimed',
  'paid_out',
  'exiting',
  'done',
];

/**
 * Typical time from entering a stage until the next stage on the normal path.
 * `funding_sent` covers the first confirmation; `funding_confirming` covers each further one.
 */
export const TYPICAL_MS: Partial<Record<Stage, number>> = {
  deposit_seen: ZCASH_BLOCK_MS,
  deposit_confirmed: PROOF_MS + 1_000,
  request_sent: 2 * ZCASH_BLOCK_MS, // the request is mined, then the maker's quote is mined
  quote_received: PROOF_MS + 1_000,
  accepted: ZCASH_BLOCK_MS + NEAR_FINAL_MS, // the acceptance is mined, then the escrow opens on NEAR
  escrow_open: NEAR_FINAL_MS + PROOF_MS, // the open is final, then the funding proof
  funding_sent: ZCASH_BLOCK_MS,
  funding_confirming: ZCASH_BLOCK_MS * (FUNDING_CONFIRMATIONS - 1),
  ready: NEAR_FINAL_MS,
  committing: NEAR_FINAL_MS,
  claiming: NEAR_FINAL_MS,
  claimed: NEAR_FINAL_MS,
  paid_out: 5_000,
  exiting: 30_000,
  refunding: PROOF_MS + ZCASH_BLOCK_MS,
  recovering: PROOF_MS + ZCASH_BLOCK_MS,
  recovered: 1_000,
};

/** How long the engine waits for the maker's quote before it gives up and refunds. */
export const QUOTE_TIMEOUT_MS = 6 * ZCASH_BLOCK_MS;
/** Time after the funding is final by which the maker must set the escrow ready (t0 in the paper). */
export const READY_WINDOW_MS = 20 * 60_000;
/** Time after which the taker may claim without Ready (t2, the punish path). */
export const PUNISH_AFTER_MS = 40 * 60_000;

/**
 * Remaining real time on the normal path from `stage`, given how long the swap has been
 * in it. Returns undefined for stages with no estimate (waiting for the user, or ended).
 */
export function remainingMs(stage: Stage, inStageMs: number, extra?: { confirmations?: number; exitEstimateMs?: number }): number | undefined {
  if (stage === 'awaiting_deposit' || stage === 'done' || stage === 'refunded' || stage === 'exit_retry') return undefined;
  if (stage === 'refunding' || stage === 'recovering' || stage === 'recovered') {
    const order: Stage[] = ['recovering', 'recovered', 'refunding'];
    let total = 0;
    for (let i = order.indexOf(stage); i < order.length; i++) total += TYPICAL_MS[order[i]] ?? 0;
    return Math.max(5_000, total - inStageMs);
  }
  const i = NORMAL_PATH.indexOf(stage);
  if (i === -1) return undefined;
  let total = 0;
  for (let j = i; j < NORMAL_PATH.length - 1; j++) {
    const s = NORMAL_PATH[j];
    let ms = TYPICAL_MS[s] ?? 0;
    if (s === 'funding_confirming' && j === i && extra?.confirmations !== undefined) {
      ms = ZCASH_BLOCK_MS * Math.max(0, FUNDING_CONFIRMATIONS - extra.confirmations);
    }
    if (s === 'exiting' && extra?.exitEstimateMs !== undefined) ms = extra.exitEstimateMs;
    total += ms;
  }
  return Math.max(5_000, total - inStageMs);
}
