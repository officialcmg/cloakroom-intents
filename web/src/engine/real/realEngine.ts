// The real engine: crates/zecret-core's taker compiled to WebAssembly (crates/zecret-wasm),
// behind the website's Engine interface (ENGINE_API.md). It runs in a Web Worker.
//
// What this layer adds to the core (docs/ENGINE.md §2.5):
//   - the website's stages, synthesized from the core's stages, events and chain facts
//     (deposit_seen, funding_confirming n/k, ready);
//   - units: seconds -> milliseconds, token base units -> micro-USDC;
//   - the exit (1Click on mainnet; the direct exit through the maker's relay elsewhere);
//   - the refund of the ZEC to the return address (`returnLeftover`), and leftovers;
//   - IndexedDB persistence, the recovery file, and restoring from it.
//
// Durability: after every call into the engine, the changed records are written to
// IndexedDB and the write is awaited (the transaction has committed) before the next
// call, and every website event is stored before it is posted. The core never uses a new
// secret, or broadcasts a transaction, in the call that created it (docs/ENGINE.md §1.2),
// so nothing leaves this browser that storage does not already hold.
import {
  EngineError,
  STAGES,
  TERMINAL_STAGES,
  type Activity,
  type CreateSwapParams,
  type EngineConfig,
  type EngineEvent,
  type ErrorCode,
  type ExitRequest,
  type InitStatus,
  type RecoveryFile,
  type Stage,
  type SwapPatch,
  type SwapState,
  type TxRef,
} from '../api';
import type { EngineBackend } from '../rpc';
import { reduceSwap } from '../reducer';
import { KEY_BUILD_MS, PROOF_MS, NEAR_FINAL_MS } from '../timeline';
import { parseDirectory, type MakerDirectory, type MakerTerms } from '../../lib/directory';
import { validateZcashReturnAddress } from '../../lib/address/zcash';
import { validateDestinationAddress } from '../../lib/address/chains';
import { paymentUri } from '../../lib/zip321';
import { coreNetwork, explorerUrl, safeLink, type NetworkConfig } from '../../lib/netconfig';
import { StaleWriteError, type RecordStore } from './idb';
import { GONE_STABLE_MS, judgePayout, readPayoutFacts, type PayoutReader } from './payoutCheck';
import { abortReason, coreStageName, loadWasm, type CoreEvent, type CoreSwap, type WasmModule, type WasmTaker } from './wasm';

const STATE = 'web/state';
const META = 'web/meta';
/** New Zcash blocks a restored swap waits for (reconciling with the chain) before it acts. */
const RESTORE_BLOCKS = 2;

interface DirectPlan {
  signerId: string;
  verifier: string;
  token: string;
  receiver: string;
  amount: string;
  deadlineMs: number;
  nonce: string;
  intentHash: string;
}

/** The worker's own record, next to the core's. */
interface Meta {
  version: 1;
  swapId: string;
  /** The premium the page showed: the largest the engine accepts. */
  maxPremium: number;
  /** First Zcash block the wallet scans (light-wallet networks). */
  birthday?: number;
  /** Set by importRecovery: the scan height at import. */
  restoreHeight?: number;
  restoreDone?: boolean;
  /** The core's swap reached a terminal stage. */
  coreDone?: boolean;
  directPlan?: DirectPlan;
  /** The receiver's token balance before the direct exit was sent. */
  directBefore?: string;
  exitQuote?: { raw: unknown; amountIn: string; amountOut: string; minAmountOut: string; depositAddress: string; timeEstimateS?: number; feeBps: number; at: number };
  exitSubmitted?: { depositAddress: string; intentHash: string };
  /** Every 1Click deposit address this swap signed for (docs/INTENTS.md §6). */
  signedDeposits?: string[];
  refundTxid?: string;
  refundEmptySince?: number;
  leftoverDone?: boolean;
  leftoverTxid?: string;
  warnings?: number;
}

type WasmResult<T> = { ok: true; value: T } | { ok: false; error: { code: ErrorCode; message: string; transient: boolean; offered?: { amountOut: string; minAmountOut: string } } };

const PRE_FUNDING: Stage[] = ['awaiting_deposit', 'deposit_seen', 'deposit_confirmed', 'request_sent', 'quote_received', 'accepted', 'escrow_open'];
/** From the funding until the claim is final, NEAR deadlines run: no 20 s key build then. */
const DEADLINE_STAGES: Stage[] = ['funding_sent', 'funding_confirming', 'ready', 'committing', 'claiming'];
const ORDER: Stage[] = [...PRE_FUNDING, 'funding_sent', 'funding_confirming', 'ready', 'committing', 'claiming', 'claimed', 'paid_out', 'exiting', 'done'];

const isUint = (s: unknown) => typeof s === 'string' && /^\d+$/.test(s);
const NEAR_ACCOUNT = /^(?=.{2,64}$)[a-z0-9]+([-_][a-z0-9]+)*(\.[a-z0-9]+([-_][a-z0-9]+)*)*$/;
const now = () => Date.now();
const secs = () => Math.floor(Date.now() / 1000);
const log = (...a: unknown[]) => console.log('[zecret-engine]', ...a);
/** Timing lines for the test runner: `[zecret-timing] <what> <ms>`. */
const timing = (what: string, ms: number) => console.log(`[zecret-timing] ${what} ${Math.round(ms)}`);

function parse<T>(json: string): T {
  return JSON.parse(json) as T;
}

export class RealEngine implements EngineBackend {
  private emitWire: (e: EngineEvent) => void = () => {};
  private wasm?: WasmModule;
  private cfg?: NetworkConfig;
  private directory?: MakerDirectory;
  private terms?: MakerTerms;
  private taker?: WasmTaker;
  private state: SwapState | null = null;
  private meta: Meta | null = null;
  private queue: Promise<unknown> = Promise.resolve();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private initStatus?: InitStatus;
  private keys: 'idle' | 'building' | 'ready' = 'idle';
  private lastPayout?: Extract<CoreEvent, { type: 'payout' }>;
  private provingSince?: { what: string; at: number };
  private directCheckSince?: number;
  /** Since when the taker's Verifier balance reads 0 in the direct exit's `delivering`. */
  private directEmptySince?: number;
  /** When the claimed swap's payout was last checked against the escrow and the Verifier. */
  private payoutCheckedAt?: number;
  /** Since when the escrow shows the payout made and the taker key's Verifier balance reads 0. */
  private payoutGoneSince?: number;
  /**
   * Records a failed write did not store (review 3, H1). Until they are written, the engine
   * calls nothing in the core: every call first retries this write.
   */
  private unsaved: Map<string, string> | null = null;
  /** The engine stopped for good in this worker: another tab writes, or the WebAssembly trapped. */
  private halted?: { code: 'OTHER_TAB' | 'ENGINE_CRASHED'; message: string };
  private readonly store: RecordStore;
  private readonly base: string;
  private readonly load: (base: string) => Promise<WasmModule>;
  private readonly readPayout: PayoutReader;

  constructor(store: RecordStore, base = '/', load: (base: string) => Promise<WasmModule> = loadWasm, readPayout: PayoutReader = readPayoutFacts) {
    this.store = store;
    this.base = base;
    this.load = load;
    this.readPayout = readPayout;
  }

  attach(emit: (event: EngineEvent) => void): void {
    this.emitWire = emit;
  }

  // ------------------------------------------------------------ plumbing

  /**
   * Runs engine work one piece at a time. Calls from the page and loop steps share it.
   * Before any work: a stopped engine refuses, and records a failed write left behind are
   * written first; if that write fails again, nothing else runs (review 3, H1).
   */
  private run<T>(fn: () => Promise<T>): Promise<T> {
    const p = this.queue.then(async () => {
      if (this.halted) throw new EngineError(this.halted.code, this.halted.message);
      if (this.unsaved) await this.persist();
      return fn();
    });
    this.queue = p.catch(() => undefined);
    return p;
  }

  private schedule(ms: number) {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = undefined;
      // A refused step (storage still failing) tries again; a halted engine stays stopped.
      this.run(() => this.step()).catch(() => !this.halted && this.schedule(5_000));
    }, Math.max(0, ms));
  }

  /** Stops this worker's engine for good and tells the page (which may start a new worker). */
  private halt(code: 'OTHER_TAB' | 'ENGINE_CRASHED', message: string): EngineError {
    if (!this.halted) {
      this.halted = { code, message };
      if (this.timer !== undefined) clearTimeout(this.timer);
      this.timer = undefined;
      this.emitWire({ type: 'error', error: { code, message } });
    }
    return new EngineError(code, message);
  }

  /**
   * Wraps a WebAssembly object so that a trap (a Rust panic: `RuntimeError: unreachable`)
   * stops this engine at once, and no later call reaches the trapped instance, also on
   * paths that catch errors and retry (review 3, H6).
   */
  private guardWasm<T extends object>(target: T): T {
    return new Proxy(target, {
      get: (t, key) => {
        const v = Reflect.get(t, key, t);
        // Classes (WasmTaker) are constructed, not called; their instances are wrapped in openTaker.
        if (typeof v !== 'function' || (typeof key === 'string' && /^[A-Z]/.test(key))) return v;
        return (...args: unknown[]) => {
          if (this.halted?.code === 'ENGINE_CRASHED') throw new EngineError('ENGINE_CRASHED', this.halted.message);
          try {
            return (v as (...a: unknown[]) => unknown).apply(t, args);
          } catch (e) {
            if (e instanceof WebAssembly.RuntimeError) throw this.halt('ENGINE_CRASHED', `The WebAssembly engine stopped (${e.message}). The page restarts it from the stored swap.`);
            throw e;
          }
        };
      },
    });
  }

  private need(): { wasm: WasmModule; cfg: NetworkConfig; terms: MakerTerms } {
    if (!this.wasm || !this.cfg || !this.terms) throw new EngineError('NOT_READY', 'Call init first.');
    return { wasm: this.wasm, cfg: this.cfg, terms: this.terms };
  }

  private requireSwap(): SwapState {
    if (!this.state || !this.meta) throw new EngineError('NO_SWAP', 'There is no swap in this browser.');
    return this.state;
  }

  /** micro-USDC per token base unit: 10^(decimals - 6). */
  private scale(): bigint {
    return 10n ** BigInt(this.cfg!.near.tokenDecimals - 6);
  }

  private toMicro(base: string): string {
    return (BigInt(base) / this.scale()).toString();
  }

  private async persist(extra: [string, string][] = []): Promise<void> {
    // Records of an earlier failed write go first, so none is lost (review 3, H1).
    const pending = new Map(this.unsaved ?? []);
    for (const [k, v] of extra) pending.set(k, v);
    const entries: [string, string][] = [...pending];
    if (this.state) entries.push([STATE, JSON.stringify(this.state)]);
    if (this.meta) entries.push([META, JSON.stringify(this.meta)]);
    try {
      await this.store.putAll(entries);
    } catch (e) {
      if (e instanceof StaleWriteError) throw this.halt('OTHER_TAB', 'Another tab of this browser changed this swap after this tab loaded it. This tab stopped; use the other tab, or reload this one.');
      const first = this.unsaved === null;
      this.unsaved = pending;
      if (first) {
        this.emitWire({
          type: 'error',
          error: { code: 'STORAGE', transient: true, message: `This browser refused to store the swap: ${(e as Error).message}. The engine waits and tries the write again.` },
        });
      }
      throw new EngineError('STORAGE', `The swap could not be stored: ${(e as Error).message}`, true);
    }
    this.unsaved = null;
  }

  /** Writes the core's changed records; resolves when they are durable. */
  private async persistCore(): Promise<void> {
    if (!this.taker) return;
    const dirty = parse<[string, string][]>(this.taker.dirty());
    if (dirty.length) await this.persist(dirty);
  }

  private async emit(event: EngineEvent): Promise<void> {
    this.state = reduceSwap(this.state, event);
    await this.persist();
    this.emitWire(event);
  }

  private async stage(stage: Stage, patch?: SwapPatch): Promise<void> {
    const s = this.state!;
    await this.emit({ type: 'stage', swapId: s.id, stage, at: now(), patch });
    const a = this.waitingActivity();
    if (a && this.state?.stage === stage && !this.state.activity) await this.activity(a);
  }

  private async activity(a: Activity | null): Promise<void> {
    await this.emit({ type: 'activity', swapId: this.state!.id, activity: a });
  }

  private zcashTx(id: string, label: string, txid: string): TxRef {
    return { id, chain: 'zcash', label, hash: txid, url: explorerUrl(this.cfg!.zcash.explorer, txid), at: now() };
  }

  private nearTx(id: string, label: string, hash: string): TxRef {
    return { id, chain: 'near', label, hash, url: explorerUrl(this.cfg!.near.explorer, hash), at: now() };
  }

  private blockMs(): number {
    // Zcash testnet blocks come about every 20 s since NU7 (measured; docs/security/fixes-2-taker.md).
    return this.cfg!.network === 'regtest' ? 5_000 : this.cfg!.network === 'testnet' ? 25_000 : 75_000;
  }

  /** A "waiting" activity for stages where the swap waits on a chain or the maker. */
  private waitingActivity(): Activity | null {
    const s = this.state;
    if (!s) return null;
    const wait = (label: string, ms: number): Activity => ({ kind: 'waiting', label, startedAt: now(), estimatedMs: ms });
    const block = this.blockMs();
    switch (s.stage) {
      case 'request_sent':
        return wait('Waiting for the request and the maker’s answer to be mined', 2 * block);
      case 'accepted':
        return wait('Waiting for the maker to lock the USDC on NEAR', block + NEAR_FINAL_MS);
      case 'funding_sent':
      case 'funding_confirming': {
        const f = s.funding;
        const n = (f?.confirmations ?? 0) + 1;
        if (f && n > f.required) return wait('Waiting for the maker to set the escrow to Ready', NEAR_FINAL_MS);
        return wait(`Waiting for Zcash block ${n} of ${f?.required ?? 2}`, block);
      }
      case 'committing':
        return wait('Waiting for the commitment to be final on NEAR', NEAR_FINAL_MS);
      case 'claiming':
        return wait('Waiting for the claim to be final on NEAR', NEAR_FINAL_MS);
      case 'claimed':
        return wait('Waiting for the USDC to move into NEAR Intents', NEAR_FINAL_MS);
      default:
        return null;
    }
  }

  private coreConfig(meta: Pick<Meta, 'maxPremium' | 'birthday'>): string {
    const { cfg } = this.need();
    const d = this.directory!;
    const chains: Record<string, unknown> = {
      network: coreNetwork(cfg.network),
      near_rpc: cfg.near.rpc,
      escrow: cfg.near.escrow,
      token: cfg.near.token,
      intents: cfg.near.verifier,
    };
    if (cfg.zcash.lightwalletd) {
      chains.lightwalletd = cfg.zcash.lightwalletd;
      chains.birthday = meta.birthday ?? 1;
    } else {
      chains.zebra_rpc = cfg.zcash.zebraRpc;
      if (cfg.zcash.nu7Height !== undefined) chains.nu7_height = cfg.zcash.nu7Height;
    }
    return JSON.stringify({
      chains,
      timing: { margin_secs: 0, ...cfg.timing },
      maker: {
        ua: d.maker.zcashAddress,
        epoch: d.maker.epoch,
        rfq_pub: d.maker.requestKey,
        near_account: d.maker.nearAccount,
        near_pk: d.maker.nearPublicKey,
        sig: d.signature,
        // The maker's limits, covered by the signature when present (`zecret directory`).
        ...(d.maker.signedLimits ? { limits: d.maker.signedLimits } : {}),
      },
      escrow_code_hash: cfg.near.escrowCodeHash,
      min_price: '0',
      max_premium: meta.maxPremium,
      min_stipend: cfg.minStipend,
      max_horizon_secs: cfg.maxHorizonSecs ?? 172800,
      quote_timeout_secs: cfg.quoteTimeoutSecs ?? 600,
      cover: { min_secs: 0, max_secs: 0, k: 0 },
    });
  }

  private openTaker(records: [string, string][]): WasmTaker {
    const { wasm } = this.need();
    this.taker?.free();
    try {
      this.taker = this.guardWasm(new wasm.WasmTaker(this.coreConfig(this.meta!), JSON.stringify(records)));
    } catch (e) {
      if (e instanceof EngineError) throw e;
      throw new EngineError('INTERNAL', `The engine could not open the swap: ${(e as Error).message}`);
    }
    this.taker.setMaxPremium(this.meta!.maxPremium);
    return this.taker;
  }

  private coreSwap(): CoreSwap | undefined {
    const t = this.taker;
    if (!t || !this.meta) return undefined;
    const live = t.swap();
    if (live) return parse<CoreSwap>(live);
    const rec = t.record(`swap/${this.meta.swapId}`);
    return rec ? parse<CoreSwap>(rec) : undefined;
  }

  // ------------------------------------------------------------ init and resume

  async init(config: EngineConfig): Promise<InitStatus> {
    return this.run(async () => {
      const cfg = config.real;
      if (!cfg) throw new EngineError('INIT_FAILED', 'The real engine needs a network configuration.');
      let parsed: ReturnType<typeof parseDirectory>;
      try {
        parsed = parseDirectory(config.directory);
      } catch (e) {
        throw new EngineError('INIT_FAILED', `The maker directory is not valid: ${(e as Error).message}`);
      }
      const d = parsed.directory;
      if (parsed.terms.network !== cfg.network) throw new EngineError('INIT_FAILED', `The maker directory is for ${parsed.terms.network}; this page runs on ${cfg.network}.`);
      if (d.maker.escrowContract !== cfg.near.escrow) throw new EngineError('INIT_FAILED', `The maker names the escrow ${d.maker.escrowContract}; this page trusts ${cfg.near.escrow}.`);
      if (!d.signature || !d.maker.nearPublicKey || d.maker.epoch === undefined)
        throw new EngineError('INIT_FAILED', 'The maker directory has no signed entry (maker.nearPublicKey, maker.epoch, signature).');
      this.cfg = cfg;
      this.directory = d;
      this.terms = parsed.terms;
      try {
        const t = now();
        this.wasm = this.guardWasm(await this.load(this.base));
        timing('wasm_load', now() - t);
      } catch (e) {
        throw new EngineError('INIT_FAILED', `The WebAssembly engine did not load: ${(e as Error).message}`);
      }
      try {
        const t = now();
        this.wasm.verifyMaker(this.coreConfig({ maxPremium: 0 }));
        timing('verify_maker', now() - t);
      } catch (e) {
        throw new EngineError('INIT_FAILED', `The maker's directory entry did not pass the check: ${(e as Error).message}`);
      }
      this.initStatus = { engine: 'wasm', phase: 'building_keys', startedAt: now(), estimatedMs: KEY_BUILD_MS, timeScale: 1 };
      if (!this.store.persistent) {
        this.emitWire({ type: 'error', error: { code: 'STORAGE', message: 'This browser does not let the page store data. Keep this tab open until the swap ends, and save the recovery file.' } });
      }
      // The key build blocks this worker for about 20 s. It starts after resume() has
      // answered, unless the stored swap is inside its NEAR deadlines (see buildKeys).
      setTimeout(() => this.maybeBuildKeys(), 1_000);
      return this.initStatus;
    });
  }

  /** Builds the proving keys when that cannot delay a NEAR deadline. */
  private maybeBuildKeys() {
    if (this.keys !== 'idle') return;
    if (this.state && DEADLINE_STAGES.includes(this.state.stage)) return;
    this.keys = 'building';
    void this.run(async () => {
      const startedAt = now();
      this.initStatus = { engine: 'wasm', phase: 'building_keys', startedAt, estimatedMs: KEY_BUILD_MS, timeScale: 1 };
      this.emitWire({ type: 'init', init: this.initStatus });
      // Let the event reach the page before the worker blocks.
      await new Promise((r) => setTimeout(r, 20));
      try {
        const ms = this.wasm!.prepareProvingKey();
        timing('proving_key_build', ms);
        this.keys = 'ready';
        this.initStatus = { engine: 'wasm', phase: 'ready', startedAt: now(), timeScale: 1 };
      } catch (e) {
        this.keys = 'idle';
        this.initStatus = { engine: 'wasm', phase: 'failed', startedAt: now(), timeScale: 1, error: { code: 'INIT_FAILED', message: (e as Error).message } };
      }
      this.emitWire({ type: 'init', init: this.initStatus });
    });
  }

  async resume(): Promise<SwapState | null> {
    return this.run(async () => {
      this.need();
      const records = await this.store.all();
      const get = (k: string) => records.find(([key]) => key === k)?.[1];
      const st = get(STATE);
      const mt = get(META);
      if (!st || !mt) {
        this.state = null;
        this.meta = null;
        this.emitWire({ type: 'swap', swap: null });
        return null;
      }
      this.state = parse<SwapState>(st);
      this.meta = parse<Meta>(mt);
      const core = records.filter(([k]) => !k.startsWith('web/'));
      this.openTaker(core);
      await this.persistCore();
      const terminal = TERMINAL_STAGES.includes(this.state.stage);
      this.state = { ...this.state, activity: terminal ? null : { kind: 'syncing', label: 'Catching up with the Zcash and NEAR networks', startedAt: now(), estimatedMs: 3_000 } };
      await this.persist();
      this.emitWire({ type: 'swap', swap: this.state });
      // A swap that reached `done` may still owe the leftover ZEC to the return address.
      if (!terminal || (this.state.stage === 'done' && !this.meta.leftoverDone)) this.schedule(0);
      return this.state;
    });
  }

  async getState(): Promise<SwapState | null> {
    return this.state;
  }

  // ------------------------------------------------------------ create

  async createSwap(params: CreateSwapParams): Promise<SwapState> {
    return this.run(async () => {
      const { wasm, cfg, terms } = this.need();
      if (this.state && !TERMINAL_STAGES.includes(this.state.stage)) throw new EngineError('SWAP_EXISTS', 'This browser already has a swap in progress.');
      if (!isUint(params.amountZats) || !isUint(params.minUsdc) || !isUint(params.minDestinationAmount) || !isUint(params.shown?.premiumZats))
        throw new EngineError('INVALID_PARAMS', 'Amounts must be whole numbers of base units.');
      const amount = BigInt(params.amountZats);
      if (amount < terms.minZats || amount > terms.maxZats) throw new EngineError('INVALID_PARAMS', 'The amount is outside the maker’s limits.');
      if (BigInt(params.minUsdc) <= 0n) throw new EngineError('INVALID_PARAMS', 'minUsdc must be positive.');
      const d = params.destination;
      if (!d || !d.assetId || !d.chain || !Number.isInteger(d.decimals)) throw new EngineError('INVALID_PARAMS', 'The destination is incomplete.');
      let destination = d;
      if (cfg.exit.mode === 'oneclick') {
        const dest = validateDestinationAddress(d.chain, d.address);
        if (!dest.ok) throw new EngineError('INVALID_ADDRESS', dest.reason);
        destination = { ...d, address: dest.address };
      } else {
        const acct = params.directExit?.nearAccount?.trim() ?? '';
        if (!NEAR_ACCOUNT.test(acct))
          throw new EngineError('INVALID_ADDRESS', 'Enter the NEAR account that receives the tokens on this network, for example alice.testnet.');
      }
      let returnAddress = params.returnAddress;
      if (returnAddress) {
        const r = validateZcashReturnAddress(returnAddress, cfg.network);
        if (!r.ok) throw new EngineError('INVALID_ADDRESS', r.reason);
        if (!wasm.checkAddress(r.address, coreNetwork(cfg.network))) throw new EngineError('INVALID_ADDRESS', 'The engine cannot pay this Zcash address.');
        returnAddress = r.address;
      }
      const premium = Number(params.shown.premiumZats);
      const zats = Number(amount) - wasm.requiredDeposit(0, premium);
      if (zats <= 0) throw new EngineError('INVALID_PARAMS', 'The amount does not cover the fees and the premium.');
      const minPrice = (BigInt(params.minUsdc) * this.scale()).toString();
      // A finished swap is replaced by the new one, with a new temporary wallet.
      if (this.state) await this.wipe();
      let birthday: number | undefined;
      if (cfg.zcash.lightwalletd) {
        try {
          birthday = Math.max(1, wasm.zcashTip(this.coreConfig({ maxPremium: premium, birthday: 1 })) - 5);
        } catch (e) {
          throw new EngineError('NETWORK', `The Zcash light-wallet server did not answer: ${(e as Error).message}`, true);
        }
      }
      this.meta = { version: 1, swapId: '', maxPremium: premium, birthday };
      const taker = this.openTaker([]);
      let core: CoreSwap;
      try {
        core = parse<CoreSwap>(taker.start(zats, minPrice, secs()));
      } catch (e) {
        this.taker?.free();
        this.taker = undefined;
        this.meta = null;
        throw new EngineError('INIT_FAILED', `The swap could not start: ${(e as Error).message}`);
      }
      this.meta.swapId = core.rfq_id;
      const ua = taker.ua();
      const t = now();
      this.state = {
        version: 1,
        id: core.rfq_id,
        engine: 'wasm',
        network: cfg.network,
        createdAt: t,
        stage: 'awaiting_deposit',
        stageAt: t,
        history: [{ stage: 'awaiting_deposit', at: t }],
        params: { ...params, destination, returnAddress },
        deposit: { address: ua, uri: paymentUri(ua, amount), expectedZats: params.amountZats, receivedZats: '0', confirmations: 0, requiredConfirmations: 1 },
        txs: [],
        needs: null,
        activity: null,
      };
      // The wallet key, a, the seeds and the NEAR key are durable before this returns.
      await this.persist(parse<[string, string][]>(taker.dirty()));
      this.emitWire({ type: 'swap', swap: this.state });
      log(`swap ${core.rfq_id} created: sells ${zats} zats, deposit ${params.amountZats} to ${ua}`);
      this.schedule(0);
      return this.state;
    });
  }

  async setReturnAddress(address: string): Promise<SwapState> {
    return this.run(async () => {
      const { wasm, cfg } = this.need();
      const s = this.requireSwap();
      if (s.stage === 'refunded') throw new EngineError('WRONG_STAGE', 'The refund was already sent.');
      if (s.stage === 'done' && this.meta!.leftoverDone) throw new EngineError('WRONG_STAGE', 'The swap is finished.');
      const r = validateZcashReturnAddress(address, cfg.network);
      if (!r.ok) throw new EngineError('INVALID_ADDRESS', r.reason);
      if (!wasm.checkAddress(r.address, coreNetwork(cfg.network))) throw new EngineError('INVALID_ADDRESS', 'The engine cannot pay this Zcash address.');
      await this.emit({ type: 'stage', swapId: s.id, stage: s.stage, at: now(), patch: { params: { returnAddress: r.address }, needs: null } });
      this.schedule(0);
      return this.state!;
    });
  }

  // ------------------------------------------------------------ recovery file

  async exportRecovery(): Promise<RecoveryFile> {
    return this.run(async () => {
      const { cfg } = this.need();
      const s = this.requireSwap();
      const t = now();
      await this.emit({ type: 'stage', swapId: s.id, stage: s.stage, at: t, patch: { recoveryExportedAt: t } });
      const records = Object.fromEntries((await this.store.all()).filter(([k]) => !k.startsWith('web/')));
      const file = {
        format: 'zecret-recovery',
        version: 1,
        engine: 'wasm',
        network: cfg.network,
        swapId: s.id,
        createdAt: new Date(s.createdAt).toISOString(),
        warning: 'Anyone with this file can move the funds of this swap. Keep it private. You can delete it after the swap is done.',
        contents:
          'records.wallet: the temporary Zcash wallet key. records["swap/<id>"]: key half a, the swap seeds, the NEAR key (near_sk) that owns the payout in NEAR Intents, and the swap terms. web: the page settings of the swap (amounts, destination, return address). The engine reads the rest back from the Zcash and NEAR chains.',
        config: { escrow: cfg.near.escrow, escrowCodeHash: cfg.near.escrowCodeHash, token: cfg.near.token, verifier: cfg.near.verifier, maker: this.directory!.maker.nearAccount },
        records,
        web: { state: { ...this.state!, activity: null }, meta: this.meta },
      };
      return { fileName: `zecret-recovery-${s.id.slice(0, 8)}.json`, mimeType: 'application/json', text: JSON.stringify(file, null, 2) };
    });
  }

  async importRecovery(text: string): Promise<SwapState> {
    return this.run(async () => {
      const { cfg } = this.need();
      let file: Record<string, unknown>;
      try {
        file = JSON.parse(text);
      } catch {
        throw new EngineError('RECOVERY_INVALID', 'This file is not a Zecret recovery file.');
      }
      if (file?.format !== 'zecret-recovery' || file.version !== 1 || typeof file.swapId !== 'string') throw new EngineError('RECOVERY_INVALID', 'This file is not a Zecret recovery file.');
      if (file.engine !== 'wasm') throw new EngineError('RECOVERY_INVALID', 'This file is from the demo with a simulated engine. Open it in the demo.');
      if (file.network !== cfg.network) throw new EngineError('RECOVERY_MISMATCH', `This file is for Zcash ${String(file.network)}, and this page runs on ${cfg.network}.`);
      const fc = file.config as Record<string, string> | undefined;
      if (!fc || fc.escrow !== cfg.near.escrow || fc.token !== cfg.near.token || fc.verifier !== cfg.near.verifier)
        throw new EngineError('RECOVERY_MISMATCH', `This file belongs to another deployment (escrow ${fc?.escrow ?? '?'}); this page uses ${cfg.near.escrow}.`);
      const records = file.records as Record<string, string> | undefined;
      const web = file.web as { state?: SwapState; meta?: Meta } | undefined;
      if (!records?.wallet || !records[`swap/${file.swapId}`] || !web?.state || !web.meta || web.state.id !== file.swapId)
        throw new EngineError('RECOVERY_INVALID', 'The keys in this file are incomplete.');
      if (this.state && this.state.id === file.swapId) return this.state;
      if (this.state && !TERMINAL_STAGES.includes(this.state.stage)) throw new EngineError('SWAP_EXISTS', 'This browser has another swap in progress. Finish it first.');
      // The page state in the file is not trusted (review 3, H2): its addresses are checked
      // again as createSwap checks them, and the payment address and link are computed from
      // the file's wallet key below.
      const restored = this.checkRestoredState(web.state, web.meta, file.swapId);
      if (this.state) await this.wipe();
      this.meta = { ...web.meta, restoreHeight: undefined, restoreDone: false };
      this.state = { ...restored, activity: null, recoveryExportedAt: now(), restoredAt: now() };
      const core = Object.entries(records);
      const taker = this.openTaker(core);
      const ua = taker.ua();
      const d = this.state.deposit;
      const rest = BigInt(d.expectedZats) - BigInt(d.receivedZats);
      this.state = { ...this.state, deposit: { ...d, address: ua, uri: paymentUri(ua, rest > 0n ? rest : BigInt(d.expectedZats)) } };
      await this.persist(core);
      try {
        this.meta.restoreHeight = taker.sync();
      } catch (e) {
        throw new EngineError('NETWORK', `The Zcash node did not answer: ${(e as Error).message}`, true);
      }
      this.state = { ...this.state, activity: { kind: 'syncing', label: 'Reading this swap back from the Zcash and NEAR chains', startedAt: now(), estimatedMs: (RESTORE_BLOCKS + 1) * this.blockMs() } };
      await this.persist();
      this.emitWire({ type: 'swap', swap: this.state });
      log(`restored swap ${file.swapId} at Zcash height ${this.meta.restoreHeight}`);
      this.schedule(0);
      return this.state;
    });
  }

  /** The page state of a recovery file, checked as createSwap checks its parameters. */
  private checkRestoredState(st: SwapState, meta: Meta, swapId: string): SwapState {
    const { wasm, cfg } = this.need();
    const bad = (why: string) => new EngineError('RECOVERY_INVALID', `The swap settings in this file are not valid: ${why}.`);
    if (meta.swapId !== swapId || typeof meta.maxPremium !== 'number' || !Number.isSafeInteger(meta.maxPremium) || meta.maxPremium < 0) throw bad('the engine record does not match');
    if (!STAGES.includes(st.stage)) throw bad('unknown stage');
    const p = st.params;
    if (!p || !isUint(p.amountZats) || !isUint(p.minUsdc) || !isUint(p.minDestinationAmount)) throw bad('amounts');
    if (!st.deposit || !isUint(st.deposit.expectedZats) || !isUint(st.deposit.receivedZats) || st.deposit.expectedZats !== p.amountZats) throw bad('the deposit');
    const d = p.destination;
    if (!d || !d.assetId || !d.chain || !Number.isInteger(d.decimals)) throw bad('the destination');
    let destination = d;
    let directExit = p.directExit;
    if (cfg.exit.mode === 'oneclick') {
      const r = validateDestinationAddress(d.chain, d.address);
      if (!r.ok) throw new EngineError('INVALID_ADDRESS', `The destination address in this file is not valid: ${r.reason}`);
      destination = { ...d, address: r.address };
    } else {
      const acct = p.directExit?.nearAccount?.trim() ?? '';
      if (!NEAR_ACCOUNT.test(acct)) throw new EngineError('INVALID_ADDRESS', 'The NEAR account in this file is not valid.');
      directExit = { nearAccount: acct };
    }
    let returnAddress = p.returnAddress;
    if (returnAddress) {
      const r = validateZcashReturnAddress(returnAddress, cfg.network);
      if (!r.ok || !wasm.checkAddress(r.address, coreNetwork(cfg.network))) throw new EngineError('INVALID_ADDRESS', 'The return address in this file is not valid.');
      returnAddress = r.address;
    }
    // Links from the file are kept only when they are https.
    const txs = (st.txs ?? []).map((t) => ({ ...t, url: safeLink(t.url) }));
    return { ...st, id: swapId, engine: 'wasm', network: cfg.network, params: { ...p, destination, directExit, returnAddress }, txs };
  }

  // ------------------------------------------------------------ exit and forget

  async exit(request: ExitRequest = {}): Promise<SwapState> {
    return this.run(async () => {
      const s = this.requireSwap();
      if (s.stage !== 'exit_retry') {
        if (s.stage === 'paid_out' || s.stage === 'exiting') return s;
        throw new EngineError('WRONG_STAGE', 'The exit runs after the USDC is paid out.');
      }
      let approvedMinimum = s.exit?.approvedMinimum ?? s.params.minDestinationAmount;
      if (request.acceptMinimum !== undefined) {
        if (!isUint(request.acceptMinimum) || BigInt(request.acceptMinimum) <= 0n) throw new EngineError('INVALID_PARAMS', 'acceptMinimum must be a positive whole number.');
        if (BigInt(request.acceptMinimum) > BigInt(approvedMinimum)) throw new EngineError('INVALID_PARAMS', 'acceptMinimum may only lower the minimum.');
        if (!s.exit?.offered) throw new EngineError('WRONG_STAGE', 'There is no lower price to accept.');
        approvedMinimum = request.acceptMinimum;
      }
      const m = this.meta!;
      m.exitQuote = undefined;
      m.exitSubmitted = undefined;
      if (s.lastError?.code !== 'NETWORK') m.directPlan = undefined;
      const direct = this.cfg!.exit.mode === 'direct';
      await this.stage('exiting', {
        exit: { attempts: (s.exit?.attempts ?? 0) + 1, phase: direct ? 'signing' : 'quoting', approvedMinimum, offered: undefined, amountOut: undefined, minAmountOut: undefined, depositAddress: undefined, intentHash: undefined },
        needs: null,
        lastError: null,
      });
      this.schedule(0);
      return this.state!;
    });
  }

  async forget(): Promise<void> {
    return this.run(async () => {
      const s = this.state;
      if (s) {
        const unpaid = s.stage === 'awaiting_deposit' && s.deposit.receivedZats === '0';
        if (!unpaid && !TERMINAL_STAGES.includes(s.stage)) throw new EngineError('WRONG_STAGE', 'A swap with money in it cannot be deleted. Let it finish or refund.');
        if (unpaid && this.taker) {
          // A payment may have arrived since the last look.
          this.taker.sync();
          if (parse<{ balance: number }>(this.taker.facts()).balance > 0) throw new EngineError('WRONG_STAGE', 'A payment arrived. The swap continues.');
        }
      }
      await this.wipe();
      this.emitWire({ type: 'swap', swap: null });
    });
  }

  private async wipe(): Promise<void> {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.taker?.free();
    this.taker = undefined;
    this.state = null;
    this.meta = null;
    this.lastPayout = undefined;
    await this.store.clear();
  }

  // ------------------------------------------------------------ the loop

  private async step(): Promise<void> {
    if (!this.taker || !this.state || !this.meta) return;
    let delay: number | null;
    try {
      delay = await this.stepInner();
    } catch (e) {
      log('step failed', e);
      if (this.halted) return;
      // A failed write already told the page (STORAGE); the next step retries it first.
      if (this.state && !(e instanceof EngineError && e.code === 'STORAGE'))
        await this.emit({ type: 'error', swapId: this.state.id, error: { code: 'INTERNAL', message: (e as Error).message } }).catch(() => undefined);
      delay = 5_000;
    }
    if (delay !== null && this.state) this.schedule(delay);
    this.maybeBuildKeys();
  }

  private async stepInner(): Promise<number | null> {
    const m = this.meta!;
    const taker = this.taker!;
    if (m.restoreHeight !== undefined && !m.restoreDone) return this.restoreStep();
    if (!m.coreDone) {
      const t0 = now();
      const r = parse<{ next: { kind: 'at'; at: number } | { kind: 'done' }; events: CoreEvent[] }>(taker.tick(secs()));
      const tickMs = now() - t0;
      // The tick's records are durable before anything else happens.
      await this.persistCore();
      if (this.provingSince) {
        timing(`proof_${this.provingSince.what}`, now() - this.provingSince.at);
        this.provingSince = undefined;
      }
      if (tickMs > 3_000) log(`tick took ${tickMs} ms`);
      await this.applyCoreEvents(r.events);
      await this.afterTick();
      if (await this.exitedBeforeThisBrowser()) return 0;
      if (r.next.kind === 'done') {
        const core = this.coreSwap();
        const name = core ? coreStageName(core.stage) : '';
        if (['Completed', 'Recovered', 'Aborted'].includes(name)) {
          m.coreDone = true;
          await this.persist();
          return 0;
        }
        return 3_000;
      }
      const at = r.next.at * 1000;
      return Math.max(0, Math.min(at - now(), 5_000));
    }
    switch (this.state!.stage) {
      case 'paid_out':
      case 'exiting':
        return this.exitStep();
      case 'recovered':
      case 'refunding':
        return this.refundStep();
      case 'done':
        return this.leftoverStep();
      default:
        return null;
    }
  }

  // ------------------------------------------------------------ core events -> website

  private async applyCoreEvents(events: CoreEvent[]): Promise<void> {
    let txs: TxRef[] = [];
    const flush = () => {
      const t = txs;
      txs = [];
      return t.length ? t : undefined;
    };
    for (const e of events) {
      switch (e.type) {
        case 'log':
          log(e.line);
          break;
        case 'warning': {
          const m = this.meta!;
          m.warnings = (m.warnings ?? 0) + 1;
          log(`transient: ${e.message}`);
          if (m.warnings === 3) await this.emit({ type: 'error', swapId: this.state!.id, error: { code: 'NETWORK', message: `A server did not answer; the engine retries. (${e.message})`, transient: true } });
          break;
        }
        case 'balance':
          await this.onBalance(e.zats);
          break;
        case 'activity':
          this.provingSince = { what: e.label.includes('request') ? 'request' : e.label.includes('acceptance') ? 'accept' : e.label.includes('back') ? 'sweep' : e.label.includes('shared address') ? 'fund' : 'other', at: now() };
          await this.activity({ kind: 'proving', label: e.label, startedAt: now(), estimatedMs: PROOF_MS });
          break;
        case 'zcash_tx': {
          const label = { rfq: 'Request to the maker (encrypted memo)', accept: 'Acceptance and premium to the maker', fund: 'Your ZEC to the shared address', sweep: 'ZEC taken back from the shared address' }[e.what] ?? e.what;
          const id = { rfq: 'request', accept: 'accept', fund: 'funding', sweep: 'recover' }[e.what] ?? e.what;
          txs.push(this.zcashTx(id, label, e.txid));
          break;
        }
        case 'near_tx': {
          const label = e.what === 'commit_claim' ? 'Claim commitment (reveals nothing)' : e.what === 'claim' ? 'Claim with your key half' : e.what;
          txs.push(this.nearTx(e.what === 'commit_claim' ? 'commit' : e.what, label, e.hash));
          break;
        }
        case 'quote':
          await this.depositStages();
          await this.stage('quote_received', { quote: { usdc: this.toMicro(e.price), premiumZats: String(e.premium), expiresAt: now() + (this.cfg!.quoteTimeoutSecs ?? 600) * 1000 }, addTxs: flush() });
          break;
        case 'escrow_verified': {
          const core = this.coreSwap();
          await this.stage('escrow_open', {
            escrow: { contract: this.cfg!.near.escrow, swapId: e.escrow_id, takerAccount: core?.taker_account ?? '', stipendYocto: core?.quote?.stipend ?? this.cfg!.minStipend },
            deadlines: { readyBy: e.t0 * 1000, punishFrom: e.t2 * 1000 },
            addTxs: flush(),
          });
          break;
        }
        case 'payout':
          this.lastPayout = e;
          break;
        case 'stage':
          await this.onCoreStage(e.stage, flush());
          break;
        case 'done':
        case 'alert':
          break;
      }
    }
    const rest = flush();
    if (rest) await this.emit({ type: 'stage', swapId: this.state!.id, stage: this.state!.stage, at: now(), patch: { addTxs: rest } });
  }

  private async onBalance(zats: number): Promise<void> {
    const s = this.state!;
    if (s.stage !== 'awaiting_deposit') return;
    if (String(zats) === s.deposit.receivedZats) return;
    const expected = BigInt(s.deposit.expectedZats);
    const got = BigInt(zats);
    if (got < expected) {
      const rest = expected - got;
      await this.stage('awaiting_deposit', { deposit: { receivedZats: got.toString(), uri: got > 0n ? paymentUri(s.deposit.address, rest) : s.deposit.uri } });
      return;
    }
    await this.stage('awaiting_deposit', { deposit: { receivedZats: got.toString() } });
    await this.depositStages();
  }

  /** The wallet sees mined notes only, and a mined note is spendable: seen and confirmed arrive together. */
  private async depositStages(): Promise<void> {
    const s = this.state!;
    if (s.stage !== 'awaiting_deposit') return;
    await this.stage('deposit_seen', { deposit: { confirmations: 0 } });
    await this.stage('deposit_confirmed', { deposit: { confirmations: 1 } });
  }

  private reached(stage: Stage): boolean {
    return this.state!.history.some((h) => h.stage === stage);
  }

  private async onCoreStage(name: string, addTxs?: TxRef[]): Promise<void> {
    const s = this.state!;
    const core = this.coreSwap();
    switch (name) {
      case 'WaitingDeposit':
        return;
      case 'RfqSent':
        await this.depositStages();
        return this.stage('request_sent', { addTxs });
      case 'Accepted':
        return this.stage('accepted', { addTxs });
      case 'Funded':
        if (ORDER.indexOf(s.stage) >= ORDER.indexOf('funding_sent')) {
          if (addTxs) await this.emit({ type: 'stage', swapId: s.id, stage: s.stage, at: now(), patch: { addTxs } });
          return;
        }
        return this.stage('funding_sent', { funding: { amountZats: String((core?.zats ?? 0) + 10_000), confirmations: 0, required: this.cfg!.timing.k_z }, addTxs });
      case 'Committed': {
        const punish = !!core?.log.some((l) => l.includes('punish path'));
        if (!punish && !this.reached('ready')) await this.stage('ready');
        return this.stage('committing', { claim: { punishPath: punish }, addTxs });
      }
      case 'Revealed':
        return this.stage('claiming', { addTxs });
      case 'Claimed':
        return this.stage('claimed', { addTxs });
      case 'PayoutPending':
        // The claim is final, but the maker has not sent `withdraw` (the stipend cannot pay
        // its gas). The core's `payout_pending` event and log name the call; any NEAR
        // account can send it. The page stays on `claimed` and says so.
        if (!this.reached('claimed')) await this.stage('claimed', { addTxs });
        return this.activity({
          kind: 'waiting',
          label: `Claimed. The payout is pending: the maker has not sent withdraw. Any NEAR account can send it: call ${this.cfg!.near.escrow} withdraw for the owner ${core?.taker_account ?? ''} with 200 Tgas.`,
          startedAt: now(),
          estimatedMs: 60_000,
        });
      case 'Completed': {
        const p = this.lastPayout;
        return this.stage('paid_out', {
          payout: { usdc: this.toMicro(p?.amount ?? core?.quote?.price ?? '0'), intentsAccount: p?.account ?? core?.taker_account ?? '' },
          addTxs,
        });
      }
      case 'Recovering':
        return this.stage('recovering', {
          refund: { reason: 'MAKER_REFUNDED' },
          lastError: { code: 'MAKER_REFUNDED', message: 'The maker did not set the escrow to Ready and took the USDC back. That revealed the maker’s key half, so your ZEC can come back.' },
          addTxs,
        });
      case 'Recovered':
        if (!this.reached('recovering')) await this.stage('recovering', { refund: { reason: 'MAKER_REFUNDED' }, addTxs });
        await this.stage('recovered', this.reached('recovering') ? { addTxs } : undefined);
        return this.stage('refunding', { needs: s.params.returnAddress ? null : 'return_address' });
      case 'Aborted': {
        const why = core ? abortReason(core.stage) : '';
        // `MAKER REFUSED (<code>): <reason>`: the maker answered with a signed refusal (over
        // its limit, under its minimum, busy, unsupported). Shown on the existing
        // MAKER_CHECK_FAILED screen with the maker's reason.
        if (why.startsWith('MAKER REFUSED')) {
          return this.stage('refunding', {
            refund: { reason: 'MAKER_CHECK_FAILED' },
            lastError: { code: 'MAKER_CHECK_FAILED', message: `The maker refused the request: ${why.replace(/^MAKER REFUSED \([a-z_]+\): /, '').replace(/; nothing was funded$/, '')}. Nothing was funded and no premium was paid` },
            needs: this.state!.params.returnAddress ? null : 'return_address',
            addTxs,
          });
        }
        const code: ErrorCode = why.startsWith('QUOTE REJECTED: price')
          ? 'QUOTE_BELOW_MINIMUM'
          : why.includes('no quote before the timeout') || why.includes('never opened the escrow')
            ? 'MAKER_TIMEOUT'
            : 'MAKER_CHECK_FAILED';
        const message =
          code === 'QUOTE_BELOW_MINIMUM' ? 'The maker’s firm price is below your minimum.' : code === 'MAKER_TIMEOUT' ? 'The maker did not answer in time.' : `The engine refused the maker’s offer: ${why}`;
        // The rejected quote's price, for the page.
        const price = /price (\d+) below/.exec(why)?.[1];
        return this.stage('refunding', {
          ...(price ? { quote: { usdc: this.toMicro(price), premiumZats: '0', expiresAt: now() } } : {}),
          refund: { reason: code },
          lastError: { code, message },
          needs: this.state!.params.returnAddress ? null : 'return_address',
          addTxs,
        });
      }
    }
  }

  /** Chain facts the core does not emit: funding confirmations and Ready. */
  private async afterTick(): Promise<void> {
    const s = this.state!;
    if (s.stage !== 'funding_sent' && s.stage !== 'funding_confirming') return;
    const f = parse<{ height: number; fundHeight: number | null }>(this.taker!.facts());
    const required = s.funding?.required ?? this.cfg!.timing.k_z;
    if (f.fundHeight) {
      const conf = Math.min(required, f.height - f.fundHeight + 1);
      if (conf > (s.funding?.confirmations ?? 0)) {
        await this.stage('funding_confirming', { funding: { confirmations: conf } });
        await this.activity(this.waitingActivity());
      }
      if (conf >= required && s.escrow?.swapId) {
        try {
          if (this.taker!.escrowState(s.escrow.swapId) === 'Ready') await this.stage('ready');
        } catch (e) {
          log(`escrow state: ${(e as Error).message}`);
        }
      }
    }
  }

  /**
   * A claimed swap whose payout already left the taker key (a swap restored after its exit
   * finished). The core waits in `Claimed` for the price to appear in the Verifier, which
   * never happens then. When the escrow shows the payout made (state `Claimed`, Intents
   * credit 0) and the taker key's Verifier balance reads 0, at final blocks, for
   * GONE_STABLE_MS, the exit happened before: the swap moves to `done` (with
   * `exit.doneElsewhere`), and the leftover step follows. Returns true when it moved.
   */
  private async exitedBeforeThisBrowser(): Promise<boolean> {
    const s = this.state!;
    const m = this.meta!;
    if (s.stage !== 'claimed') return false;
    const core = this.coreSwap();
    if (!core?.escrow_id || !core.taker_account || !['Claimed', 'PayoutPending'].includes(coreStageName(core.stage))) return false;
    if (this.payoutCheckedAt !== undefined && now() - this.payoutCheckedAt < 10_000) return false;
    this.payoutCheckedAt = now();
    const { cfg } = this.need();
    let facts;
    try {
      facts = await this.readPayout({ rpc: cfg.near.rpc, escrow: cfg.near.escrow, verifier: cfg.near.verifier, token: cfg.near.token, swapId: core.escrow_id, taker: core.taker_account });
    } catch (e) {
      log(`payout check: ${(e as Error).message}`);
      return false;
    }
    if (judgePayout(facts) !== 'gone') {
      this.payoutGoneSince = undefined;
      return false;
    }
    if (this.payoutGoneSince === undefined) {
      this.payoutGoneSince = now();
      log('payout check: the escrow paid the taker key and its Verifier balance is 0; checking again before concluding that the exit already happened');
    }
    if (now() - this.payoutGoneSince < GONE_STABLE_MS) return false;
    this.payoutGoneSince = undefined;
    log(`payout check: the escrow paid ${facts.amount ?? '?'} to ${core.taker_account} and the tokens left that key: the exit happened before this browser followed the swap`);
    m.coreDone = true;
    await this.persist();
    const direct = cfg.exit.mode === 'direct';
    await this.emit({
      type: 'stage',
      swapId: s.id,
      stage: 'paid_out',
      at: now(),
      patch: { payout: { usdc: this.toMicro(facts.amount ?? core.quote?.price ?? '0'), intentsAccount: core.taker_account } },
    });
    await this.emit({
      type: 'stage',
      swapId: s.id,
      stage: 'done',
      at: now(),
      patch: {
        exit: {
          attempts: s.exit?.attempts ?? 0,
          phase: 'done',
          approvedMinimum: s.exit?.approvedMinimum ?? s.params.minDestinationAmount,
          doneElsewhere: true,
          ...(direct ? { direct: { nearAccount: s.params.directExit!.nearAccount, token: cfg.near.token, symbol: cfg.near.tokenSymbol, decimals: cfg.near.tokenDecimals } } : {}),
        },
        needs: null,
        lastError: null,
      },
    });
    return true;
  }

  // ------------------------------------------------------------ restore

  private async restoreStep(): Promise<number> {
    const m = this.meta!;
    const taker = this.taker!;
    const h = taker.sync();
    const changed = taker.reconcile(secs());
    await this.persistCore();
    if (changed) log(`reconciled the restored swap with the chain at height ${h}`);
    if (h < (m.restoreHeight ?? 0) + RESTORE_BLOCKS) return 2_000;
    m.restoreDone = true;
    const core = this.coreSwap();
    if (core) await this.catchUp(core);
    await this.persist();
    log(`restore done at height ${h}; core stage ${core ? coreStageName(core.stage) : '?'}`);
    return 0;
  }

  /** Moves the website's state to where the restored core record is. */
  private async catchUp(core: CoreSwap): Promise<void> {
    const s = this.state!;
    const name = coreStageName(core.stage);
    const target: Partial<Record<string, Stage>> = { RfqSent: 'request_sent', Accepted: 'accepted', Funded: 'funding_sent', Committed: 'committing', Revealed: 'claiming', Claimed: 'claimed', PayoutPending: 'claimed' };
    const want = target[name];
    const patch: SwapPatch = { addTxs: [] };
    if (core.quote && !s.quote) patch.quote = { usdc: this.toMicro(core.quote.price), premiumZats: String(core.quote.premium), expiresAt: now() };
    if (core.escrow_id && !s.escrow) patch.escrow = { contract: this.cfg!.near.escrow, swapId: core.escrow_id, takerAccount: core.taker_account, stipendYocto: core.quote?.stipend ?? this.cfg!.minStipend };
    if (core.fund_txid && !s.funding) patch.funding = { amountZats: String(core.zats + 10_000), confirmations: 0, required: this.cfg!.timing.k_z };
    const known = new Set(s.txs.map((t) => t.id));
    const add = (id: string, t: TxRef) => !known.has(id) && patch.addTxs!.push(t);
    if (core.rfq_txid) add('request', this.zcashTx('request', 'Request to the maker (encrypted memo)', core.rfq_txid));
    if (core.accept_txid) add('accept', this.zcashTx('accept', 'Acceptance and premium to the maker', core.accept_txid));
    if (core.fund_txid) add('funding', this.zcashTx('funding', 'Your ZEC to the shared address', core.fund_txid));
    if (want && ORDER.indexOf(want) > ORDER.indexOf(s.stage)) {
      if (s.stage === 'awaiting_deposit') await this.depositStages();
      await this.stage(want, patch);
    } else {
      await this.activity(null);
      if (patch.addTxs!.length) await this.emit({ type: 'stage', swapId: s.id, stage: s.stage, at: now(), patch });
    }
  }

  // ------------------------------------------------------------ refund and leftover

  private returnAddress(): string | undefined {
    return this.state!.params.returnAddress;
  }

  private async refundStep(): Promise<number | null> {
    const s = this.state!;
    const m = this.meta!;
    const taker = this.taker!;
    if (s.stage === 'recovered') {
      await this.stage('refunding', { needs: this.returnAddress() ? null : 'return_address' });
      return 0;
    }
    const to = this.returnAddress();
    if (!to) {
      if (s.needs !== 'return_address') await this.stage('refunding', { needs: 'return_address' });
      return null;
    }
    taker.sync();
    if (m.refundTxid) {
      const h = taker.txHeight(m.refundTxid);
      if (h === undefined) return 3_000;
      await this.stage('refunded', { needs: null });
      return null;
    }
    const fee = this.wasm!.feePerSend();
    const bal = parse<{ balance: number }>(taker.facts()).balance;
    if (bal <= fee) {
      // Change from the last send may still be unmined; wait a few blocks before giving up.
      m.refundEmptySince ??= now();
      await this.persist();
      if (now() - m.refundEmptySince < 6 * this.blockMs()) return 3_000;
      await this.stage('refunded', { refund: { amountZats: '0', toAddress: to }, needs: null });
      return null;
    }
    await this.activity({ kind: 'proving', label: 'Building the refund to your return address', startedAt: now(), estimatedMs: PROOF_MS });
    const t = now();
    let txid: string;
    try {
      txid = taker.returnLeftover(to);
    } catch (e) {
      log(`refund failed: ${(e as Error).message}`);
      await this.releaseFailedSend();
      return 10_000;
    }
    timing('proof_refund', now() - t);
    m.refundTxid = txid;
    await this.persist();
    await this.stage('refunding', { refund: { amountZats: String(bal - fee), toAddress: to }, addTxs: [this.zcashTx('refund', 'ZEC back to your return address', txid)] });
    return 3_000;
  }

  /**
   * After a refund or leftover send failed. The wallet reserves the notes of every
   * transaction it builds until that transaction is mined or expires (about 40 blocks), also
   * when the node refused it, and the balance leaves reserved notes out. The refund and
   * leftover steps would then read a balance of 0 and end with nothing sent. The engine
   * therefore opens the swap again from storage: the reservations are not stored, so the
   * notes count again after the rescan. A send that did reach the network still wins: the
   * notes it spends are seen as spent, and a second send of them is refused.
   */
  private async releaseFailedSend(): Promise<void> {
    // The activity write also runs the version check (another tab) before the read below.
    await this.activity(null);
    const records = (await this.store.all()).filter(([k]) => !k.startsWith('web/'));
    this.openTaker(records);
    await this.persistCore();
  }

  private async leftoverStep(): Promise<number | null> {
    const m = this.meta!;
    const taker = this.taker!;
    if (m.leftoverDone) return null;
    if (m.leftoverTxid) {
      taker.sync();
      if (taker.txHeight(m.leftoverTxid) === undefined) return 3_000;
      m.leftoverDone = true;
      await this.persist();
      return null;
    }
    taker.sync();
    const fee = this.wasm!.feePerSend();
    const bal = parse<{ balance: number }>(taker.facts()).balance;
    if (bal <= fee) {
      m.leftoverDone = true;
      await this.persist();
      return null;
    }
    const to = this.returnAddress();
    if (!to) {
      await this.stage('done', { leftover: { amountZats: String(bal - fee) }, needs: 'return_address' });
      return null;
    }
    await this.activity({ kind: 'proving', label: 'Sending the leftover ZEC to your return address', startedAt: now(), estimatedMs: PROOF_MS });
    let txid: string;
    try {
      txid = taker.returnLeftover(to);
    } catch (e) {
      log(`leftover failed: ${(e as Error).message}`);
      await this.releaseFailedSend();
      return 10_000;
    }
    m.leftoverTxid = txid;
    await this.persist();
    log(`leftover of ${bal - fee} zats sent to the return address in tx ${txid}`);
    await this.stage('done', { leftover: { amountZats: String(bal - fee), toAddress: to }, needs: null, addTxs: [this.zcashTx('leftover', 'Leftover ZEC to your return address', txid)] });
    return 3_000;
  }

  // ------------------------------------------------------------ the exit

  private nearSeed(): string {
    const core = this.coreSwap();
    if (!core?.near_sk) throw new EngineError('INTERNAL', 'The swap record has no NEAR key.');
    return core.near_sk;
  }

  private async exitFailed(err: { code: ErrorCode; message: string; offered?: { amountOut: string; minAmountOut: string } }, phase?: 'refunded' | 'failed'): Promise<null> {
    const s = this.state!;
    await this.stage('exit_retry', {
      exit: { ...(err.offered ? { offered: err.offered } : {}), ...(phase ? { phase } : {}) },
      needs: 'exit_decision',
      lastError: { code: err.code, message: err.message },
    });
    log(`exit stopped: ${err.code} ${err.message} (stage was ${s.stage})`);
    return null;
  }

  private async exitStep(): Promise<number | null> {
    const s = this.state!;
    const { cfg, wasm } = this.need();
    const direct = cfg.exit.mode === 'direct';
    if (s.stage === 'paid_out') {
      await this.stage('exiting', {
        exit: {
          attempts: 1,
          phase: direct ? 'signing' : 'quoting',
          approvedMinimum: s.params.minDestinationAmount,
          ...(direct ? { direct: { nearAccount: s.params.directExit!.nearAccount, token: cfg.near.token, symbol: cfg.near.tokenSymbol, decimals: cfg.near.tokenDecimals } } : {}),
        },
      });
      return 0;
    }
    return direct ? this.directExitStep(wasm, cfg) : this.oneClickStep(wasm, cfg);
  }

  private async directExitStep(wasm: WasmModule, cfg: NetworkConfig): Promise<number | null> {
    const s = this.state!;
    const m = this.meta!;
    if (cfg.exit.mode !== 'direct') return null;
    const receiver = s.params.directExit!.nearAccount;
    if (!m.directPlan) {
      await this.activity({ kind: 'signing', label: 'Preparing the transfer of your tokens to your NEAR account', startedAt: now(), estimatedMs: 2_000 });
      const r = parse<WasmResult<DirectPlan>>(wasm.directPrepare(cfg.near.rpc, cfg.near.verifier, cfg.near.token, this.nearSeed(), receiver, now()));
      if (!r.ok) {
        if (r.error.transient) return 5_000;
        return this.exitFailed(r.error);
      }
      m.directPlan = r.value;
      await this.persist();
      await this.stage('exiting', {
        exit: { phase: 'signing', amountIn: r.value.amount, amountOut: r.value.amount, minAmountOut: r.value.amount, depositAddress: receiver, intentHash: r.value.intentHash },
      });
      return 0;
    }
    const plan = m.directPlan;
    if (s.exit?.phase === 'signing' || s.exit?.phase === 'submitted') {
      if (m.directBefore === undefined) {
        m.directBefore = this.taker!.tokenBalance(receiver);
        await this.persist();
      }
      await this.activity({ kind: 'broadcasting', label: 'Signing the transfer and handing it to the maker’s relay', startedAt: now(), estimatedMs: 5_000 });
      const t = now();
      const r = parse<WasmResult<{ txHash: string | null; delivered: string }>>(wasm.directRelay(cfg.near.rpc, cfg.exit.relayUrl, this.nearSeed(), JSON.stringify(plan), now()));
      timing('direct_exit_relay', now() - t);
      if (!r.ok) {
        if (r.error.transient) {
          await this.emit({ type: 'error', swapId: s.id, error: { code: 'NETWORK', message: r.error.message, transient: true } });
          return 5_000;
        }
        return this.exitFailed(r.error, 'failed');
      }
      const hash = r.value.txHash ?? plan.intentHash;
      await this.stage('exiting', { exit: { phase: 'delivering' }, addTxs: [this.nearTx(`exit-${s.exit.attempts}`, 'Exit intent executed (ft_withdraw from NEAR Intents)', hash)] });
      return 0;
    }
    // delivering: done once the receiver holds the amount more, at a final block. (A failed
    // ft_transfer is refunded into the Verifier by the Verifier itself.)
    const got = BigInt(this.taker!.tokenBalance(receiver)) - BigInt(m.directBefore ?? '0');
    if (got < BigInt(plan.amount)) {
      const left = this.taker!.intentsBalance(s.payout?.intentsAccount ?? plan.signerId);
      // A receiver that moves the tokens on at once (an exchange) never shows the rise
      // (review 3, H5). The relay reported the intent executed (or its nonce used), and a
      // failed ft_transfer is refunded into the Verifier within a few blocks: a zero
      // balance that stays zero for 15 s at final blocks means the tokens left.
      if (left === '0') {
        this.directEmptySince ??= now();
        if (now() - this.directEmptySince < 15_000) return 3_000;
        this.directEmptySince = undefined;
        log(`direct exit: the receiver's balance did not rise by ${plan.amount}, and the taker's Verifier balance is 0: delivered`);
        return this.directDone(plan, receiver);
      }
      this.directEmptySince = undefined;
      // Views read final blocks, which trail the executed transaction by a few seconds.
      this.directCheckSince ??= now();
      if (left === plan.amount && now() - this.directCheckSince > 30_000) {
        this.directCheckSince = undefined;
        m.directPlan = undefined;
        m.directBefore = undefined;
        return this.exitFailed({ code: 'EXIT_FAILED', message: `The transfer to ${receiver} did not arrive and the tokens are back under your key in NEAR Intents.` }, 'refunded');
      }
      return 3_000;
    }
    return this.directDone(plan, receiver);
  }

  private async directDone(plan: DirectPlan, receiver: string): Promise<number> {
    const s = this.state!;
    await this.stage('done', {
      exit: { phase: 'done', deliveredAmount: plan.amount },
      addTxs: [{ ...this.nearTx('delivery', `${this.cfg!.near.tokenSymbol} sent to ${receiver}`, s.txs.find((t) => t.id === `exit-${s.exit?.attempts}`)?.hash ?? plan.intentHash) }],
    });
    return 0;
  }

  private approval(cfg: NetworkConfig) {
    const s = this.state!;
    return {
      signerId: s.payout!.intentsAccount,
      verifyingContract: cfg.near.verifier,
      originAsset: `nep141:${cfg.near.token}`,
      amountIn: (BigInt(s.payout!.usdc) * this.scale()).toString(),
      destinationAsset: s.params.destination.assetId,
      recipient: s.params.destination.address,
      minAmountOut: s.exit!.approvedMinimum,
    };
  }

  private async oneClickStep(wasm: WasmModule, cfg: NetworkConfig): Promise<number | null> {
    const s = this.state!;
    const m = this.meta!;
    if (cfg.exit.mode !== 'oneclick') return null;
    const base = cfg.exit.oneClickUrl;
    const exit = s.exit!;
    if (exit.phase === 'quoting') {
      // An earlier signed exit may have delivered although its submit answer was lost
      // (review 3, H4): look at every deposit address signed for before a new quote.
      if (await this.earlierExitDelivered(wasm, base)) return 0;
      const r = parse<WasmResult<NonNullable<Meta['exitQuote']>>>(wasm.exitQuote(base, JSON.stringify(this.approval(cfg)), now()));
      if (!r.ok) {
        if (r.error.code === 'EXIT_NO_QUOTE') {
          await this.emit({ type: 'stage', swapId: s.id, stage: 'exiting', at: now(), patch: { lastError: { code: 'EXIT_NO_QUOTE', message: r.error.message, transient: true } } });
          await this.activity({ kind: 'waiting', label: 'Waiting to ask NEAR Intents again', startedAt: now(), estimatedMs: 20_000 });
          return 20_000;
        }
        if (r.error.transient) return 10_000;
        return this.exitFailed(r.error);
      }
      m.exitQuote = { ...r.value, at: now() };
      await this.persist();
      const q = r.value;
      await this.stage('exiting', {
        exit: { phase: 'signing', amountIn: q.amountIn, amountOut: q.amountOut, minAmountOut: q.minAmountOut, depositAddress: q.depositAddress, timeEstimateS: q.timeEstimateS, feeBps: q.feeBps },
        lastError: null,
      });
      return 0;
    }
    if (exit.phase === 'signing') {
      if (!m.exitQuote) {
        await this.stage('exiting', { exit: { phase: 'quoting' } });
        return 0;
      }
      await this.activity({ kind: 'signing', label: 'Checking the delivery request and signing it with your key', startedAt: now(), estimatedMs: 3_000 });
      // Recorded before the submit, so that a lost answer still leaves the address to poll (review 3, H4).
      if (!(m.signedDeposits ?? []).includes(m.exitQuote.depositAddress)) {
        m.signedDeposits = [...(m.signedDeposits ?? []), m.exitQuote.depositAddress];
        await this.persist();
      }
      const r = parse<WasmResult<{ intentHash: string; depositAddress: string }>>(wasm.exitSubmit(base, this.nearSeed(), JSON.stringify(this.approval(cfg)), JSON.stringify(m.exitQuote.raw), now()));
      if (!r.ok) {
        if (r.error.transient) return 10_000;
        if (/too old|expired/i.test(r.error.message)) {
          m.exitQuote = undefined;
          await this.stage('exiting', { exit: { phase: 'quoting' } });
          return 0;
        }
        return this.exitFailed(r.error);
      }
      m.exitSubmitted = r.value;
      if (!(m.signedDeposits ?? []).includes(r.value.depositAddress)) m.signedDeposits = [...(m.signedDeposits ?? []), r.value.depositAddress];
      await this.persist();
      await this.stage('exiting', { exit: { phase: 'submitted', intentHash: r.value.intentHash }, addTxs: [this.nearTx(`exit-${exit.attempts}`, 'Exit intent signed and submitted', r.value.intentHash)] });
      return 3_000;
    }
    const deposit = m.exitSubmitted?.depositAddress ?? exit.depositAddress;
    if (!deposit) return this.exitFailed({ code: 'INTERNAL', message: 'The submitted exit has no deposit address.' });
    const r = parse<WasmResult<{ status: string; isFinal: boolean; amountOut?: string; destinationTxs: { hash: string; url?: string }[] }>>(wasm.exitStatus(base, deposit));
    if (!r.ok) return 10_000;
    const st = r.value;
    if (st.status === 'SUCCESS') return this.exitDelivered(st, exit.amountOut);
    // Another address this swap signed for may be the one that delivered (review 3, H4).
    if (await this.earlierExitDelivered(wasm, base, deposit)) return 0;
    if (!st.isFinal) {
      if (exit.phase !== 'delivering') await this.stage('exiting', { exit: { phase: 'delivering' } });
      return 5_000;
    }
    m.exitQuote = undefined;
    m.exitSubmitted = undefined;
    return this.exitFailed({ code: 'EXIT_FAILED', message: `NEAR Intents reported ${st.status}. The USDC is back under your key.` }, st.status === 'REFUNDED' ? 'refunded' : 'failed');
  }

  private async exitDelivered(st: { amountOut?: string; destinationTxs: { hash: string; url?: string }[] }, amountOut?: string): Promise<number> {
    const s = this.state!;
    await this.stage('done', {
      exit: { phase: 'done', deliveredAmount: st.amountOut ?? amountOut },
      // 1Click's explorer links become links on the page: https only (review 3, H3).
      addTxs: st.destinationTxs.map((t, i) => ({ id: i === 0 ? 'delivery' : `delivery-${i}`, chain: 'destination' as const, label: `${s.params.destination.symbol} delivered`, hash: t.hash, url: safeLink(t.url), at: now() })),
    });
    return 0;
  }

  /** True (and the swap is done) when a deposit address signed for earlier, other than `skip`, reports SUCCESS. */
  private async earlierExitDelivered(wasm: WasmModule, base: string, skip?: string): Promise<boolean> {
    for (const addr of this.meta!.signedDeposits ?? []) {
      if (addr === skip) continue;
      const r = parse<WasmResult<{ status: string; isFinal: boolean; amountOut?: string; destinationTxs: { hash: string; url?: string }[] }>>(wasm.exitStatus(base, addr));
      if (r.ok && r.value.status === 'SUCCESS') {
        log(`exit: the earlier deposit address ${addr} delivered`);
        await this.exitDelivered(r.value);
        return true;
      }
    }
    return false;
  }
}
