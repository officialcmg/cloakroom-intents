// How much USDC the maker can pay right now, read from NEAR.
//
// The maker opens a swap from its own account in the Verifier. When that holds less than
// the swap's amount, it first deposits the difference from its plain token balance, and
// refuses when both together are short (`open` in crates/zecret-core/src/maker.rs). So the
// maker can pay at most:
//
//   mt_balance_of(maker, "nep141:<token>") on the Verifier  +  ft_balance_of(maker) on the token
//
// Both are read at the same final block, through the configuration's NEAR RPC endpoints in
// order: one try each, as payoutCheck.ts does. The page uses the number to warn before the
// user pays; the maker checks again when it opens.

export interface LiquidityQuery {
  rpcs: string[];
  verifier: string;
  token: string;
  /** The maker's NEAR account, from the signed directory entry. */
  maker: string;
  /** Decimals of the token; the result is scaled to micro-USDC (6 decimals). */
  tokenDecimals: number;
}

export interface LiquidityFacts {
  /** The maker's token balance inside the Verifier (token base units). */
  inVerifier: bigint;
  /** The maker's plain token balance (token base units). */
  plain: bigint;
}

/** What the maker can pay, in micro-USDC (rounded down). */
export function spendableMicroUsdc(f: LiquidityFacts, tokenDecimals: number): bigint {
  const total = f.inVerifier + f.plain;
  const scale = 10n ** BigInt(Math.max(0, tokenDecimals - 6));
  return total / scale;
}

/**
 * Whether a swap that pays `needMicroUsdc` is more than the maker holds. The maker pays
 * its firm price, which is close to the expected amount at its indicative rate.
 */
export function exceedsLiquidity(needMicroUsdc: bigint, liquidityMicroUsdc: bigint | undefined): boolean {
  return liquidityMicroUsdc !== undefined && needMicroUsdc > liquidityMicroUsdc;
}

type Fetch = typeof fetch;

async function rpcCall(f: Fetch, rpc: string, method: string, params: unknown, signal?: AbortSignal): Promise<unknown> {
  const res = await f(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 'zecret', method, params }), signal });
  if (!res.ok) throw new Error(`NEAR RPC answered ${res.status}`);
  const body = (await res.json()) as { result?: unknown; error?: unknown };
  if (body.error !== undefined || body.result === undefined) throw new Error(`NEAR RPC error: ${JSON.stringify(body.error ?? 'no result')}`);
  return body.result;
}

const UINT = /^\d{1,40}$/;

async function readAt(f: Fetch, rpc: string, q: LiquidityQuery, signal?: AbortSignal): Promise<LiquidityFacts> {
  const block = (await rpcCall(f, rpc, 'block', { finality: 'final' }, signal)) as { header?: { hash?: string } };
  const hash = block?.header?.hash;
  if (typeof hash !== 'string') throw new Error('NEAR RPC: no block hash');
  const view = async (account: string, method: string, args: unknown): Promise<bigint> => {
    const r = (await rpcCall(f, rpc, 'query', { request_type: 'call_function', block_id: hash, account_id: account, method_name: method, args_base64: btoa(JSON.stringify(args)) }, signal)) as {
      result?: number[];
    };
    if (!Array.isArray(r?.result)) throw new Error(`${method}: no result`);
    const v: unknown = JSON.parse(new TextDecoder().decode(new Uint8Array(r.result)));
    if (typeof v !== 'string' || !UINT.test(v)) throw new Error(`${method}: not an amount`);
    return BigInt(v);
  };
  const [inVerifier, plain] = await Promise.all([
    view(q.verifier, 'mt_balance_of', { account_id: q.maker, token_id: `nep141:${q.token}` }),
    view(q.token, 'ft_balance_of', { account_id: q.maker }),
  ]);
  return { inVerifier, plain };
}

/** Reads the maker's liquidity in micro-USDC. Throws when no endpoint answers. */
export async function readMakerLiquidity(q: LiquidityQuery, f: Fetch = fetch, signal?: AbortSignal): Promise<bigint> {
  let last: unknown = new Error('no NEAR RPC endpoint');
  for (const rpc of q.rpcs) {
    try {
      return spendableMicroUsdc(await readAt(f, rpc, q, signal), q.tokenDecimals);
    } catch (e) {
      if (signal?.aborted) throw e;
      last = e;
    }
  }
  throw last;
}

/** At most one read per this many milliseconds, also when the tab regains focus. */
export const LIQUIDITY_MIN_INTERVAL_MS = 30_000;
/** The regular refresh while the form is open. */
export const LIQUIDITY_REFRESH_MS = 60_000;
