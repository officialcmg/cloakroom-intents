// Did the payout of a claimed swap already leave the taker key? (A restore after the exit.)
//
// The core waits in `Claimed` until the taker key holds the price in the Verifier
// (`Taker::on_claimed`). A browser that restores a swap after its exit finished never sees
// that: the escrow paid the credit into the Verifier, and the exit then moved the tokens
// out. The escrow and the Verifier tell the cases apart (contracts/near-escrow/src/lib.rs):
//
// - `claim` sets the swap to `Claimed` and adds the amount to the taker's Intents credit in
//   the same receipt. Only `withdraw` lowers that credit, and the escrow restores it when the
//   transfer into the Verifier fails. So `Claimed` with an Intents credit of zero means the
//   escrow paid the taker key (or the payment is still in flight for a few blocks).
// - The taker key's account in the Verifier is new for each swap, and only the taker key can
//   move tokens out of it. A zero balance after the payment means the exit moved them.
//
// The engine therefore acts on `gone` only when it holds at final blocks for a while
// (`GONE_STABLE_MS`), which covers a withdraw still in flight.

/** How long `gone` must hold before the engine concludes the exit happened. */
export const GONE_STABLE_MS = 30_000;

/** What a NEAR view returns, at a final block. */
export interface PayoutFacts {
  /** The escrow's swap state (`get_swap(id).state`), or undefined if the swap is not there. */
  escrowState: string | undefined;
  /** The swap's amount in the escrow (`get_swap(id).amount`), token base units. */
  amount: string | undefined;
  /** The taker's Intents credit in the escrow (`get_credit(token, taker).intents`). */
  creditIntents: string;
  /** The taker key's token balance in the Verifier (`mt_balance_of`). */
  verifierBalance: string;
}

/**
 * - `not_paid`: the escrow still owes the taker (credit above zero), or the claim is not
 *   visible: the core keeps waiting (and reports PayoutPending after its timeout).
 * - `in_verifier`: the tokens are under the taker key: the core sees them and completes.
 * - `gone`: the escrow paid and the tokens left the taker key: the exit already happened.
 */
export type PayoutVerdict = 'not_paid' | 'in_verifier' | 'gone';

export function judgePayout(f: PayoutFacts): PayoutVerdict {
  if (f.escrowState !== 'Claimed') return 'not_paid';
  if (BigInt(f.creditIntents) > 0n) return 'not_paid';
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

/** All views at one final block, so the three answers describe the same moment. */
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
  const swap = (await view(escrow, 'get_swap', { id: swapId })) as { state: string; amount: string } | null;
  const credit = (await view(escrow, 'get_credit', { token, owner: taker })) as { intents: string };
  const balance = (await view(verifier, 'mt_balance_of', { account_id: taker, token_id: `nep141:${token}` })) as string;
  return { escrowState: swap?.state, amount: swap?.amount, creditIntents: String(credit.intents), verifierBalance: String(balance) };
};
