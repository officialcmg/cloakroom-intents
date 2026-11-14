// Did the payout of a claimed swap already leave the taker key? (A restore after the exit.)
//
// The escrow pays a claimed swap with one `mt_transfer` inside the Verifier, from the
// escrow's own Verifier account to the taker key's account. Its callback sets the swap's
// `payout` to `paid` only after that transfer succeeded, and `paid` is final
// (contracts/near-escrow/src/lib.rs). The core completes when a final block shows `paid`.
//
// A browser that restores a swap after its exit finished must not start the exit again:
// the tokens are gone. The escrow and the Verifier tell the cases apart, read at one final
// block:
//
// - `payout` is not `paid`: the escrow still holds the tokens (or the transfer is in
//   flight). The core keeps going.
// - `paid` and the taker key's Verifier balance is above zero: the tokens are there; the
//   exit runs.
// - `paid` and a zero balance: the transfer happened before this block, and only the taker
//   key can move tokens out of its Verifier account. So the exit already happened.

/** What NEAR views return, at one final block. */
export interface PayoutFacts {
  /** The escrow's swap state (`get_swap(id).state`), or undefined if the swap is not there. */
  escrowState: string | undefined;
  /** The swap's amount in the escrow (`get_swap(id).amount`), token base units. */
  amount: string | undefined;
  /** The swap's payout state (`get_swap(id).payout`): `none`, `due`, `sending` or `paid`. */
  payout: string | undefined;
  /** The taker key's token balance in the Verifier (`mt_balance_of`). */
  verifierBalance: string;
}

/**
 * - `not_paid`: the escrow has not paid the taker yet (or the claim is not visible): the
 *   core keeps going (it sends withdraw itself and reports PayoutPending after its timeout).
 * - `in_verifier`: the tokens are under the taker key: the exit runs.
 * - `gone`: the escrow paid and the tokens left the taker key: the exit already happened.
 */
export type PayoutVerdict = 'not_paid' | 'in_verifier' | 'gone';

export function judgePayout(f: PayoutFacts): PayoutVerdict {
  if (f.escrowState !== 'Claimed' || f.payout !== 'paid') return 'not_paid';
  if (BigInt(f.verifierBalance) > 0n) return 'in_verifier';
  return 'gone';
}

/** Reads the facts above. Implemented with NEAR JSON-RPC views in the browser. */
export type PayoutReader = (q: { rpc: string; escrow: string; verifier: string; token: string; swapId: string; taker: string }) => Promise<PayoutFacts>;

async function rpcCall(rpc: string, method: string, params: unknown): Promise<unknown> {
  const res = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 'zecret', method, params }) });
  if (!res.ok) throw new Error(`NEAR RPC answered ${res.status}`);
  const body = (await res.json()) as { result?: unknown; error?: unknown };
  if (body.error !== undefined || body.result === undefined) throw new Error(`NEAR RPC error: ${JSON.stringify(body.error ?? 'no result')}`);
  return body.result;
}

/** Both views at one final block, so the answers describe the same moment. */
export const readPayoutFacts: PayoutReader = async ({ rpc, escrow, verifier, token, swapId, taker }) => {
  const block = (await rpcCall(rpc, 'block', { finality: 'final' })) as { header: { hash: string } };
  const view = async (account: string, method: string, args: unknown): Promise<unknown> => {
    const r = (await rpcCall(rpc, 'query', {
      request_type: 'call_function',
      block_id: block.header.hash,
      account_id: account,
      method_name: method,
      args_base64: btoa(JSON.stringify(args)),
    })) as { result: number[] };
    return JSON.parse(new TextDecoder().decode(new Uint8Array(r.result)));
  };
  const swap = (await view(escrow, 'get_swap', { id: swapId })) as { state: string; amount: string; payout: string } | null;
  const balance = (await view(verifier, 'mt_balance_of', { account_id: taker, token_id: `nep141:${token}` })) as string;
  return { escrowState: swap?.state, amount: swap?.amount, payout: swap?.payout, verifierBalance: String(balance) };
};
