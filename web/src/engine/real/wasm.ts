// The WebAssembly engine (crates/zecret-wasm), as the worker sees it. The module is built
// by crates/zecret-wasm/build.sh into public/engine/ and loaded at run time, so the page
// type-checks and builds without it; only the real engine needs it.
//
// Every call blocks the worker: network calls inside are synchronous XMLHttpRequests and
// one shielded send proves for about 25 s.

export interface WasmTaker {
  ua(): string;
  setMaxPremium(zats: number): void;
  /** `[[key, value], ...]` changed since the last call. */
  dirty(): string;
  record(key: string): string | undefined;
  start(zats: number, minPrice: string, now: number): string;
  /** `{next: {kind: 'at', at} | {kind: 'done'}, events: CoreEvent[]}` */
  tick(now: number): string;
  reconcile(now: number): boolean;
  swap(): string | undefined;
  /** `{balance, height, fundHeight, sweepHeight}` */
  facts(): string;
  sync(): number;
  txHeight(txid: string): number | undefined;
  escrowState(escrowId: string): string | undefined;
  intentsBalance(account: string): string;
  tokenBalance(account: string): string;
  returnLeftover(ua: string): string;
  free(): void;
}

export interface WasmModule {
  default: (input?: { module_or_path: string | URL }) => Promise<unknown>;
  WasmTaker: new (cfgJson: string, recordsJson: string) => WasmTaker;
  prepareProvingKey(): number;
  requiredDeposit(zats: number, maxPremium: number): number;
  feePerSend(): number;
  zcashTip(cfgJson: string): number;
  checkAddress(ua: string, network: string): boolean;
  verifyMaker(cfgJson: string): void;
  nearAccount(seedHex: string): string;
  exitQuote(baseUrl: string, approvalJson: string, nowMs: number): string;
  exitSubmit(baseUrl: string, seedHex: string, approvalJson: string, quoteRawJson: string, nowMs: number): string;
  exitStatus(baseUrl: string, depositAddress: string): string;
  directPrepare(rpcUrl: string, verifier: string, token: string, seedHex: string, receiver: string, nowMs: number): string;
  directRelay(rpcUrl: string, relayUrl: string, seedHex: string, planJson: string, nowMs: number): string;
}

/** Loads the engine module from `<base>engine/zecret_wasm.js` (same origin). */
export async function loadWasm(base: string): Promise<WasmModule> {
  const url = new URL(`${base}engine/zecret_wasm.js`, self.location.origin).href;
  const mod = (await import(/* @vite-ignore */ url)) as WasmModule;
  await mod.default({ module_or_path: new URL(`${base}engine/zecret_wasm_bg.wasm`, self.location.origin) });
  return mod;
}

/** Events of crates/zecret-core (docs/ENGINE.md §2.3). */
export type CoreEvent =
  | { type: 'stage'; swap: string; stage: string }
  | { type: 'log'; swap: string; line: string }
  | { type: 'zcash_tx'; swap: string; what: string; txid: string }
  | { type: 'near_tx'; swap: string; what: string; hash: string }
  | { type: 'balance'; zats: number }
  | { type: 'quote'; swap: string; price: string; premium: number; stipend: string; joint_address: string }
  | { type: 'escrow_verified'; swap: string; escrow_id: string; t0: number; t1: number; t2: number }
  | { type: 'payout'; swap: string; account: string; token: string; amount: string }
  | { type: 'activity'; swap: string; kind: string; label: string; estimated_secs: number }
  | { type: 'alert'; swap: string; message: string }
  | { type: 'warning'; swap: string; message: string }
  | { type: 'done'; swap: string; outcome: string };

/** The parts of the core's swap record (`TakerSwap`) the worker reads. */
export interface CoreSwap {
  rfq_id: string;
  stage: string | { Aborted: string };
  zats: number;
  near_sk: string;
  taker_account: string;
  quote: { price: string; premium: number; stipend: string } | null;
  escrow_id: string | null;
  joint_address: string | null;
  rfq_txid: string | null;
  accept_txid: string | null;
  fund_txid: string | null;
  sweep_txid: string | null;
  commit_tx: string | null;
  claim_tx: string | null;
  log: string[];
}

export function coreStageName(s: CoreSwap['stage']): string {
  return typeof s === 'string' ? s : 'Aborted';
}

export function abortReason(s: CoreSwap['stage']): string {
  return typeof s === 'string' ? '' : s.Aborted;
}
