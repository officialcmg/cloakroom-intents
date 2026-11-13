// Folds engine events into a SwapState. The page uses it to keep its view of the swap,
// and the simulated engine uses it to keep its own record, so both always agree.
import type { EngineEvent, SwapPatch, SwapState, TxRef } from './api';

function mergeTxs(existing: TxRef[], added: TxRef[] | undefined): TxRef[] {
  if (!added || added.length === 0) return existing;
  const out = [...existing];
  for (const tx of added) {
    const i = out.findIndex((t) => t.id === tx.id);
    if (i === -1) out.push(tx);
    else out[i] = tx;
  }
  return out;
}

/** Applies a patch: object fields merge one level deep, `null` clears a field. */
export function applyPatch(state: SwapState, patch: SwapPatch | undefined): SwapState {
  if (!patch) return state;
  const next: Record<string, unknown> = { ...state };
  for (const [key, value] of Object.entries(patch)) {
    if (key === 'addTxs') continue;
    if (value === undefined) continue;
    const current = next[key];
    if (value === null) {
      next[key] = null;
    } else if (typeof value === 'object' && !Array.isArray(value) && current && typeof current === 'object' && !Array.isArray(current)) {
      next[key] = { ...(current as object), ...(value as object) };
    } else {
      next[key] = value;
    }
  }
  const out = next as unknown as SwapState;
  out.txs = mergeTxs(state.txs, patch.addTxs);
  return out;
}

export function reduceSwap(state: SwapState | null, event: EngineEvent): SwapState | null {
  switch (event.type) {
    case 'swap':
      return event.swap;
    case 'stage': {
      if (!state || state.id !== event.swapId) return state;
      const last = state.history[state.history.length - 1];
      const history = last && last.stage === event.stage ? state.history : [...state.history, { stage: event.stage, at: event.at }];
      const moved = last?.stage !== event.stage;
      const base: SwapState = {
        ...state,
        stage: event.stage,
        stageAt: moved ? event.at : state.stageAt,
        history,
        // A new stage ends the activity of the old one unless the patch sets a new one.
        activity: moved ? null : state.activity,
      };
      return applyPatch(base, event.patch);
    }
    case 'activity':
      if (!state || state.id !== event.swapId) return state;
      return { ...state, activity: event.activity };
    case 'error':
      if (!state || (event.swapId && state.id !== event.swapId)) return state;
      return { ...state, lastError: event.error };
    case 'init':
      return state;
  }
}
