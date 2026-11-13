// The simulated engine. It implements the same backend as the real engine will, runs in a
// Web Worker, keeps its state in IndexedDB, and plays a swap on the real timeline divided
// by `speed`. It moves no money and talks to no network.
//
// Two stores stand in for two kinds of knowledge:
//   - `local`: what only this browser knows (the swap's keys). A cleared browser loses it.
//   - `chain`: what the Zcash and NEAR networks and the maker know. It survives a cleared
//     browser, so a recovery file can pick the swap up again.
import {
  EngineError,
  type CreateSwapParams,
  type EngineConfig,
  type EngineEvent,
  type ExitRequest,
  type InitStatus,
  type Network,
  type RecoveryFile,
  type SimControls,
  type SimOptions,
  type Stage,
  type SwapPatch,
  type SwapState,
  type TxRef,
  type Activity,
  type ErrorCode,
  SIM_FAILURES,
  TERMINAL_STAGES,
} from '../api';
import type { EngineBackend } from '../rpc';
import { reduceSwap } from '../reducer';
import {
  DEPOSIT_CONFIRMATIONS,
  FUNDING_CONFIRMATIONS,
  KEY_BUILD_MS,
  NEAR_FINAL_MS,
  PROOF_MS,
  PUNISH_AFTER_MS,
  QUOTE_TIMEOUT_MS,
  READY_WINDOW_MS,
  STIPEND_YOCTO,
  TYPICAL_MS,
  ZCASH_BLOCK_MS,
} from '../timeline';
import { parseDirectory, type MakerTerms } from '../../lib/directory';
import { encodeUnifiedAddress, validateZcashReturnAddress, ZCASH_TYPECODES } from '../../lib/address/zcash';
import { chainInfo, validateDestinationAddress } from '../../lib/address/chains';
import { base58Encode, bytesToHex } from '../../lib/address/encoding';
import { paymentUri } from '../../lib/zip321';
import type { KvStore } from './store';

const CURRENT = 'current';

interface LocalRecord {
  swapId: string;
  secrets: { spendingKey: string; keyHalf: string; nearKey: string };
  recoveryExportedAt?: number;
}

interface ChainRecord {
  state: SwapState;
  sim: { speed: number; fail?: (typeof SIM_FAILURES)[number]; paidAt?: number; exitFailures: number; leftoverSent?: boolean; exitEstimateMs?: number };
}

interface Step {
  at: number;
  actor: 'taker' | 'network';
  proving?: boolean;
  run: () => SwapEventBatch;
}

type SwapEventBatch = { stage: Stage; patch?: SwapPatch; record?: Partial<ChainRecord['sim']> };

function randomBytes(n: number): Uint8Array {
  const b = new Uint8Array(n);
  crypto.getRandomValues(b);
  return b;
}
const randomHex = (n: number) => bytesToHex(randomBytes(n));
const nearHash = () => base58Encode(randomBytes(32));

function zcashTxUrl(network: Network, txid: string): string | undefined {
  if (network === 'mainnet') return `https://mainnet.zcashexplorer.app/transactions/${txid}`;
  if (network === 'testnet') return `https://testnet.zcashexplorer.app/transactions/${txid}`;
  return undefined;
}

function nearTxUrl(network: Network, hash: string): string | undefined {
  if (network === 'mainnet') return `https://nearblocks.io/txns/${hash}`;
  if (network === 'testnet') return `https://testnet.nearblocks.io/txns/${hash}`;
  return undefined;
}

const isUint = (s: unknown) => typeof s === 'string' && /^\d+$/.test(s);

export class SimEngine implements EngineBackend {
  private emitWire: (e: EngineEvent) => void = () => {};
  private store: KvStore;
  private terms?: MakerTerms;
  private escrowContract = '';
  private queue: Promise<void> = Promise.resolve();
  private network: Network = 'testnet';
  private options: SimOptions = { speed: 1 };
  private initStatus?: InitStatus;
  private keysReadyAt = 0;
  private sessionStart = Date.now();
  private state: SwapState | null = null;
  private local: LocalRecord | null = null;
  private chain: ChainRecord | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private pending?: Step;
  private now: () => number;

  constructor(store: KvStore, now: () => number = () => Date.now()) {
    this.store = store;
    this.now = now;
  }

  attach(emit: (event: EngineEvent) => void): void {
    this.emitWire = emit;
  }

  // ------------------------------------------------------------ helpers

  private scaled(ms: number): number {
    return Math.max(30, Math.round(ms / this.options.speed));
  }

  private requireInit(): MakerTerms {
    if (!this.terms) throw new EngineError('NOT_READY', 'Call init first.');
    return this.terms;
  }

  private requireSwap(): SwapState {
    if (!this.state || !this.chain || !this.local) throw new EngineError('NO_SWAP', 'There is no swap in this browser.');
    return this.state;
  }

  private async persist(): Promise<void> {
    if (this.chain && this.state) {
      this.chain.state = this.state;
      await this.store.put('chain', this.state.id, this.chain);
    }
    if (this.local) await this.store.put('local', CURRENT, this.local);
  }

  private async emit(event: EngineEvent): Promise<void> {
    this.state = reduceSwap(this.state, event);
    await this.persist();
    this.emitWire(event);
  }

  private tx(id: string, chain: TxRef['chain'], label: string, at: number): TxRef {
    const network = this.network;
    if (chain === 'zcash') {
      const hash = randomHex(32);
      return { id, chain, label, hash, url: zcashTxUrl(network, hash), at };
    }
    if (chain === 'near') {
      const hash = nearHash();
      return { id, chain, label, hash, url: nearTxUrl(network, hash), at };
    }
    const s = this.state!;
    const info = chainInfo(s.params.destination.chain);
    const hash = info?.family === 'evm' ? `0x${randomHex(32)}` : base58Encode(randomBytes(64));
    return { id, chain, label, hash, url: info?.txUrl?.(hash), at };
  }

  // ------------------------------------------------------------ init and resume

  async init(config: EngineConfig): Promise<InitStatus> {
    let terms: MakerTerms;
    try {
      const parsed = parseDirectory(config.directory);
      terms = parsed.terms;
      this.escrowContract = parsed.directory.maker.escrowContract;
    } catch (e) {
      throw new EngineError('INIT_FAILED', `The maker directory is not valid: ${(e as Error).message}`);
    }
    if (config.network === 'mainnet' || terms.network === 'mainnet')
      throw new EngineError('INIT_FAILED', 'The simulated engine runs only on testnet or regtest, so that nobody sends real ZEC to it.');
    if (terms.network !== config.network) throw new EngineError('INIT_FAILED', 'The maker directory is for another network.');
    const sim = config.sim ?? { speed: 1 };
    if (!(sim.speed > 0 && sim.speed <= 10_000)) throw new EngineError('INVALID_PARAMS', 'sim.speed must be between 0 and 10000.');
    if (sim.fail && !SIM_FAILURES.includes(sim.fail)) throw new EngineError('INVALID_PARAMS', `Unknown failure ${sim.fail}.`);
    this.terms = terms;
    this.network = config.network;
    this.options = sim;
    this.sessionStart = this.now();
    const startedAt = this.now();
    const buildMs = this.scaled(KEY_BUILD_MS);
    this.keysReadyAt = startedAt + buildMs;
    this.initStatus = { engine: 'simulated', phase: 'building_keys', startedAt, estimatedMs: buildMs, timeScale: sim.speed };
    this.emitWire({ type: 'init', init: this.initStatus });
    setTimeout(() => {
      this.initStatus = { engine: 'simulated', phase: 'ready', startedAt: this.now(), timeScale: sim.speed };
      this.emitWire({ type: 'init', init: this.initStatus });
      this.pump();
    }, buildMs);
    if (!this.store.persistent) {
      this.emitWire({ type: 'error', error: { code: 'STORAGE', message: 'This browser does not let the page store data. Keep this tab open until the swap ends, and save the recovery file.' } });
    }
    return this.initStatus;
  }

  async resume(): Promise<SwapState | null> {
    this.requireInit();
    const local = await this.store.get<LocalRecord>('local', CURRENT);
    if (!local) {
      this.state = null;
      this.emitWire({ type: 'swap', swap: null });
      return null;
    }
    const chain = await this.store.get<ChainRecord>('chain', local.swapId);
    if (!chain) {
      await this.store.delete('local', CURRENT);
      this.emitWire({ type: 'swap', swap: null });
      return null;
    }
    return this.adopt(local, chain, true);
  }

  private async adopt(local: LocalRecord, chain: ChainRecord, resumed: boolean): Promise<SwapState> {
    this.local = local;
    this.chain = chain;
    this.state = { ...chain.state, recoveryExportedAt: local.recoveryExportedAt, activity: null };
    await this.persist();
    this.emitWire({ type: 'swap', swap: this.state });
    if (resumed && !TERMINAL_STAGES.includes(this.state.stage)) {
      // The real engine scans the chain from the wallet's birthday after a reload.
      const ms = this.scaled(3_000);
      await this.emit({ type: 'activity', swapId: this.state.id, activity: { kind: 'syncing', label: 'Catching up with the Zcash and NEAR networks', startedAt: this.now(), estimatedMs: ms } });
      this.sessionStart = this.now() + ms;
      setTimeout(() => this.enqueue(() => this.startActivity()), ms);
    }
    this.pump();
    return this.state;
  }

  async getState(): Promise<SwapState | null> {
    return this.state;
  }

  // ------------------------------------------------------------ create

  async createSwap(params: CreateSwapParams): Promise<SwapState> {
    const terms = this.requireInit();
    if (this.state && !TERMINAL_STAGES.includes(this.state.stage)) throw new EngineError('SWAP_EXISTS', 'This browser already has a swap in progress.');
    if (!isUint(params.amountZats) || !isUint(params.minUsdc) || !isUint(params.minDestinationAmount))
      throw new EngineError('INVALID_PARAMS', 'Amounts must be whole numbers of base units.');
    const amount = BigInt(params.amountZats);
    if (amount < terms.minZats || amount > terms.maxZats) throw new EngineError('INVALID_PARAMS', 'The amount is outside the maker\'s limits.');
    if (BigInt(params.minUsdc) <= 0n) throw new EngineError('INVALID_PARAMS', 'minUsdc must be positive.');
    const d = params.destination;
    if (!d || !d.assetId || !d.chain || !Number.isInteger(d.decimals)) throw new EngineError('INVALID_PARAMS', 'The destination is incomplete.');
    const dest = validateDestinationAddress(d.chain, d.address);
    if (!dest.ok) throw new EngineError('INVALID_ADDRESS', dest.reason);
    if (params.returnAddress) {
      const r = validateZcashReturnAddress(params.returnAddress, this.network);
      if (!r.ok) throw new EngineError('INVALID_ADDRESS', r.reason);
    }
    // Keys of the swap: the temporary wallet, key half a, and the NEAR key.
    const secrets = { spendingKey: randomHex(32), keyHalf: randomHex(32), nearKey: randomHex(32) };
    const id = randomHex(16);
    const depositAddress = encodeUnifiedAddress(this.network, [{ typecode: ZCASH_TYPECODES.orchard, data: randomBytes(43) }]);
    const now = this.now();
    const state: SwapState = {
      version: 1,
      id,
      engine: 'simulated',
      network: this.network,
      createdAt: now,
      stage: 'awaiting_deposit',
      stageAt: now,
      history: [{ stage: 'awaiting_deposit', at: now }],
      params: { ...params, destination: { ...d, address: dest.address } },
      deposit: {
        address: depositAddress,
        uri: paymentUri(depositAddress, amount),
        expectedZats: params.amountZats,
        receivedZats: '0',
        confirmations: 0,
        requiredConfirmations: DEPOSIT_CONFIRMATIONS,
      },
      txs: [],
      needs: null,
      activity: null,
    };
    // A finished swap is replaced by the new one.
    if (this.state) await this.store.delete('chain', this.state.id);
    this.state = state;
    this.local = { swapId: id, secrets };
    this.chain = { state, sim: { speed: this.options.speed, fail: this.options.fail, exitFailures: 0 } };
    await this.persist();
    this.emitWire({ type: 'swap', swap: state });
    return state;
  }

  async setReturnAddress(address: string): Promise<SwapState> {
    const s = this.requireSwap();
    if (s.stage === 'refunded') throw new EngineError('WRONG_STAGE', 'The refund was already sent.');
    if (s.stage === 'done' && this.chain!.sim.leftoverSent) throw new EngineError('WRONG_STAGE', 'The swap is finished.');
    const r = validateZcashReturnAddress(address, this.network);
    if (!r.ok) throw new EngineError('INVALID_ADDRESS', r.reason);
    await this.emit({ type: 'stage', swapId: s.id, stage: s.stage, at: this.now(), patch: { params: { returnAddress: r.address }, needs: null } });
    this.pump();
    return this.state!;
  }

  // ------------------------------------------------------------ recovery file

  async exportRecovery(): Promise<RecoveryFile> {
    const s = this.requireSwap();
    const local = this.local!;
    local.recoveryExportedAt = this.now();
    await this.emit({ type: 'stage', swapId: s.id, stage: s.stage, at: this.now(), patch: { recoveryExportedAt: local.recoveryExportedAt } });
    const file = {
      format: 'zecret-recovery',
      version: 1,
      engine: 'simulated',
      network: this.network,
      swapId: s.id,
      createdAt: new Date(s.createdAt).toISOString(),
      warning: 'Anyone with this file can move the funds of this swap. Keep it private. You can delete it after the swap is done.',
      secrets: local.secrets,
      params: s.params,
      depositAddress: s.deposit.address,
      // A simulated swap has no real chain to read back, so the file carries a snapshot of it.
      simulatedChain: this.chain,
    };
    return {
      fileName: `zecret-recovery-${s.id.slice(0, 8)}.json`,
      mimeType: 'application/json',
      text: JSON.stringify(file, null, 2),
    };
  }

  async importRecovery(text: string): Promise<SwapState> {
    this.requireInit();
    let file: Record<string, unknown>;
    try {
      file = JSON.parse(text);
    } catch {
      throw new EngineError('RECOVERY_INVALID', 'This file is not a Zecret recovery file.');
    }
    if (file?.format !== 'zecret-recovery' || file.version !== 1 || typeof file.swapId !== 'string')
      throw new EngineError('RECOVERY_INVALID', 'This file is not a Zecret recovery file.');
    if (file.engine !== 'simulated') throw new EngineError('RECOVERY_INVALID', 'This file belongs to a real swap. Open it on the live site, not in the demo.');
    if (file.network !== this.network) throw new EngineError('RECOVERY_MISMATCH', `This file is for Zcash ${String(file.network)}, and this page runs on ${this.network}.`);
    const secrets = file.secrets as LocalRecord['secrets'] | undefined;
    if (!secrets || typeof secrets.spendingKey !== 'string' || typeof secrets.keyHalf !== 'string' || typeof secrets.nearKey !== 'string')
      throw new EngineError('RECOVERY_INVALID', 'The keys in this file are incomplete.');
    const swapId = file.swapId;
    if (this.state && this.state.id === swapId) return this.state;
    if (this.state && !TERMINAL_STAGES.includes(this.state.stage))
      throw new EngineError('SWAP_EXISTS', 'This browser has another swap in progress. Finish it first.');
    let chain = await this.store.get<ChainRecord>('chain', swapId);
    if (!chain) {
      const snap = file.simulatedChain as ChainRecord | undefined;
      if (!snap?.state || snap.state.id !== swapId) throw new EngineError('RECOVERY_INVALID', 'The file does not describe a swap.');
      chain = snap;
    }
    return this.adopt({ swapId, secrets, recoveryExportedAt: this.now() }, chain, true);
  }

  // ------------------------------------------------------------ exit

  async exit(request: ExitRequest): Promise<SwapState> {
    const s = this.requireSwap();
    if (s.stage !== 'paid_out' && s.stage !== 'exit_retry') throw new EngineError('WRONG_STAGE', 'The exit runs after the USDC is paid out.');
    const q = request?.quote;
    if (!q || !isUint(q.amountIn) || !isUint(q.amountOut) || !isUint(q.minAmountOut) || typeof q.deadline !== 'string')
      throw new EngineError('INVALID_PARAMS', 'The exit quote is incomplete.');
    if (q.amountIn !== s.payout?.usdc) throw new EngineError('INVALID_PARAMS', 'The exit quote must spend exactly the USDC that was paid out.');
    const now = this.now();
    if (Date.parse(q.deadline) <= now) throw new EngineError('EXIT_QUOTE_EXPIRED', 'The price expired. Get a new one.');
    const below = BigInt(q.minAmountOut) < BigInt(s.params.minDestinationAmount);
    if (below && !request.acceptBelowMinimum) {
      await this.emit({
        type: 'stage',
        swapId: s.id,
        stage: 'exit_retry',
        at: now,
        patch: {
          exit: { offered: { amountOut: q.amountOut, minAmountOut: q.minAmountOut } },
          needs: 'exit_decision',
          lastError: { code: 'EXIT_BELOW_MINIMUM', message: 'The current price for the delivery is below your minimum.' },
        },
      });
      return this.state!;
    }
    const attempts = (s.exit?.attempts ?? 0) + 1;
    const tx = this.tx(`exit-${attempts}`, 'near', 'Exit intent signed and submitted', now);
    this.chain!.sim.exitEstimateMs = undefined;
    await this.emit({
      type: 'stage',
      swapId: s.id,
      stage: 'exiting',
      at: now,
      patch: {
        exit: { attempts, amountIn: q.amountIn, amountOut: q.amountOut, minAmountOut: q.minAmountOut, depositAddress: q.depositAddress, offered: undefined },
        needs: null,
        lastError: null,
        addTxs: [tx],
      },
    });
    await this.startActivity();
    this.pump();
    return this.state!;
  }

  async forget(): Promise<void> {
    const s = this.state;
    if (s) {
      const unpaid = s.stage === 'awaiting_deposit' && s.deposit.receivedZats === '0';
      if (!unpaid && !TERMINAL_STAGES.includes(s.stage)) throw new EngineError('WRONG_STAGE', 'A swap with money in it cannot be deleted. Let it finish or refund.');
      await this.store.delete('chain', s.id);
    }
    await this.store.delete('local', CURRENT);
    this.clearTimer();
    this.state = null;
    this.local = null;
    this.chain = null;
    this.emitWire({ type: 'swap', swap: null });
  }

  // ------------------------------------------------------------ simulation controls

  sim: SimControls = {
    pay: async (amountZats?: string) => {
      const s = this.requireSwap();
      if (s.stage !== 'awaiting_deposit') throw new EngineError('WRONG_STAGE', 'The payment already arrived.');
      const owed = BigInt(s.deposit.expectedZats) - BigInt(s.deposit.receivedZats);
      const amount = amountZats === undefined ? owed : BigInt(amountZats);
      if (amount <= 0n) throw new EngineError('INVALID_PARAMS', 'Pay a positive amount.');
      const received = BigInt(s.deposit.receivedZats) + amount;
      const rest = BigInt(s.deposit.expectedZats) - received;
      this.chain!.sim.paidAt = this.now();
      await this.emit({
        type: 'stage',
        swapId: s.id,
        stage: 'awaiting_deposit',
        at: this.now(),
        patch: { deposit: { receivedZats: received.toString(), uri: rest > 0n ? paymentUri(s.deposit.address, rest) : s.deposit.uri } },
      });
      this.pump();
    },
    skip: async () => {
      const step = this.pending;
      if (!step) return;
      if (this.timer !== undefined) clearTimeout(this.timer);
      this.timer = undefined;
      const now = { ...step, at: this.now() };
      this.pending = now;
      this.enqueue(() => this.fire(now));
      await this.queue;
    },
    clearLocal: async () => {
      this.clearTimer();
      await this.store.delete('local', CURRENT);
      this.state = null;
      this.local = null;
      this.chain = null;
    },
  };

  // ------------------------------------------------------------ the schedule

  private clearTimer() {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.pending = undefined;
  }

  private takerBase(at: number, proving: boolean): number {
    let base = Math.max(at, this.sessionStart);
    if (proving) base = Math.max(base, this.keysReadyAt);
    return base;
  }

  /** The next thing that happens to the current swap, or undefined while it waits for the user. */
  private nextStep(): Step | undefined {
    const s = this.state;
    const c = this.chain;
    if (!s || !c) return undefined;
    const sim = c.sim;
    const d = (ms: number) => this.scaled(ms);
    const at = s.stageAt;
    const taker = (ms: number, run: () => SwapEventBatch, proving = false): Step => ({
      at: this.takerBase(at, proving) + d(ms),
      actor: 'taker',
      proving,
      run,
    });
    const network = (when: number, run: () => SwapEventBatch): Step => ({ at: when, actor: 'network', run });
    const terms = this.terms!;
    const fee = terms.feePerSendZats;

    switch (s.stage) {
      case 'awaiting_deposit': {
        if (BigInt(s.deposit.receivedZats) < BigInt(s.deposit.expectedZats) || sim.paidAt === undefined) return undefined;
        return network(sim.paidAt + d(4_000), () => ({ stage: 'deposit_seen', patch: { deposit: { confirmations: 0 } } }));
      }
      case 'deposit_seen':
        return network(at + d(ZCASH_BLOCK_MS), () => ({ stage: 'deposit_confirmed', patch: { deposit: { confirmations: 1 } } }));
      case 'deposit_confirmed':
        return taker(TYPICAL_MS.deposit_confirmed!, () => ({ stage: 'request_sent', patch: { addTxs: [this.tx('request', 'zcash', 'Request to the maker (encrypted memo)', this.now())] } }), true);
      case 'request_sent': {
        if (sim.fail === 'no-quote') {
          return network(at + d(QUOTE_TIMEOUT_MS), () => ({
            stage: 'refunding',
            patch: { refund: { reason: 'MAKER_TIMEOUT' }, lastError: { code: 'MAKER_TIMEOUT', message: 'The maker did not answer in time.' }, needs: s.params.returnAddress ? null : 'return_address' },
          }));
        }
        return network(at + d(TYPICAL_MS.request_sent!), () => {
          const shown = s.params.shown;
          const minUsdc = BigInt(s.params.minUsdc);
          const usdc = sim.fail === 'quote-low' ? (minUsdc * 97n) / 100n : BigInt(shown.usdcExpected);
          const quote = { usdc: usdc.toString(), premiumZats: shown.premiumZats, expiresAt: this.now() + 10 * 60_000 };
          if (usdc < minUsdc) {
            return {
              stage: 'refunding',
              patch: {
                quote,
                refund: { reason: 'QUOTE_BELOW_MINIMUM' },
                lastError: { code: 'QUOTE_BELOW_MINIMUM', message: 'The maker\'s firm price is below your minimum.' },
                needs: s.params.returnAddress ? null : 'return_address',
              },
            };
          }
          return { stage: 'quote_received', patch: { quote } };
        });
      }
      case 'quote_received':
        return taker(TYPICAL_MS.quote_received!, () => ({ stage: 'accepted', patch: { addTxs: [this.tx('accept', 'zcash', 'Acceptance and premium to the maker', this.now())] } }), true);
      case 'accepted':
        return network(at + d(TYPICAL_MS.accepted!), () => {
          const now = this.now();
          return {
            stage: 'escrow_open',
            patch: {
              escrow: { contract: this.directoryEscrow(), swapId: randomHex(32), takerAccount: randomHex(32), stipendYocto: STIPEND_YOCTO },
              addTxs: [this.tx('open', 'near', 'Maker locked the USDC in the escrow', now), this.tx('stipend', 'near', 'Maker sent 0.03 NEAR to your new key', now)],
            },
          };
        });
      case 'escrow_open':
        return taker(TYPICAL_MS.escrow_open!, () => {
          const swapZats = BigInt(s.params.amountZats) - BigInt(s.params.shown.premiumZats) - fee * 3n;
          const now = this.now();
          const finalAt = now + d(ZCASH_BLOCK_MS) * FUNDING_CONFIRMATIONS;
          return {
            stage: 'funding_sent',
            patch: {
              funding: { amountZats: swapZats.toString(), confirmations: 0, required: FUNDING_CONFIRMATIONS },
              deadlines: { readyBy: finalAt + d(READY_WINDOW_MS), punishFrom: finalAt + d(PUNISH_AFTER_MS) },
              addTxs: [this.tx('funding', 'zcash', 'Your ZEC to the shared address', now)],
            },
          };
        }, true);
      case 'funding_sent':
        return network(at + d(ZCASH_BLOCK_MS), () => ({ stage: 'funding_confirming', patch: { funding: { confirmations: 1 } } }));
      case 'funding_confirming': {
        const f = s.funding!;
        // stageAt is the first confirmation; each further one is one block later.
        if (f.confirmations < f.required) {
          return network(at + d(ZCASH_BLOCK_MS) * f.confirmations, () => ({ stage: 'funding_confirming', patch: { funding: { confirmations: f.confirmations + 1 } } }));
        }
        const finalAt = at + d(ZCASH_BLOCK_MS) * (f.required - 1);
        const deadlines = s.deadlines!;
        if (sim.fail === 'maker-quiet') {
          return network(deadlines.readyBy + d(60_000), () => ({
            stage: 'recovering',
            patch: {
              refund: { reason: 'MAKER_REFUNDED' },
              lastError: { code: 'MAKER_REFUNDED', message: 'The maker did not set the escrow ready and took the USDC back. That reveals the maker\'s key half, so your ZEC can come back.' },
              addTxs: [this.tx('maker-refund', 'near', 'Maker refunded the escrow (reveals its key half)', this.now())],
            },
          }));
        }
        if (sim.fail === 'maker-gone') {
          return { at: Math.max(deadlines.punishFrom, this.sessionStart), actor: 'taker', run: () => ({ stage: 'committing', patch: { claim: { punishPath: true }, addTxs: [this.tx('commit', 'near', 'Claim commitment (after the deadline)', this.now())] } }) };
        }
        return network(finalAt + d(NEAR_FINAL_MS), () => ({ stage: 'ready', patch: { addTxs: [this.tx('ready', 'near', 'Maker set the escrow to Ready', this.now())] } }));
      }
      case 'ready':
        return taker(2_000, () => ({ stage: 'committing', patch: { claim: { punishPath: false }, addTxs: [this.tx('commit', 'near', 'Claim commitment (reveals nothing)', this.now())] } }));
      case 'committing':
        return taker(TYPICAL_MS.committing!, () => ({ stage: 'claiming', patch: { addTxs: [this.tx('claim', 'near', 'Claim with your key half', this.now())] } }));
      case 'claiming':
        return network(at + d(TYPICAL_MS.claiming!), () => ({ stage: 'claimed' }));
      case 'claimed':
        return taker(TYPICAL_MS.claimed!, () => ({
          stage: 'paid_out',
          patch: { payout: { usdc: s.quote!.usdc, intentsAccount: s.escrow!.takerAccount }, addTxs: [this.tx('withdraw', 'near', 'USDC paid into NEAR Intents under your key', this.now())] },
        }));
      case 'exiting': {
        const failNow = sim.fail === 'exit-fail' && sim.exitFailures === 0;
        return network(at + d(TYPICAL_MS.exiting!), () => {
          if (failNow) {
            return {
              stage: 'exit_retry',
              patch: { needs: 'exit_decision', lastError: { code: 'EXIT_FAILED', message: 'NEAR Intents could not complete the delivery. The USDC came back to your key.' } },
              record: { exitFailures: sim.exitFailures + 1 },
            };
          }
          const exit = s.exit!;
          const leftover = BigInt(s.deposit.receivedZats) - BigInt(s.deposit.expectedZats);
          return {
            stage: 'done',
            patch: {
              exit: { deliveredAmount: exit.amountOut },
              leftover: leftover > 0n ? { amountZats: (leftover - fee).toString() } : undefined,
              needs: leftover > fee && !s.params.returnAddress ? 'return_address' : null,
              addTxs: [this.tx('delivery', 'destination', `${s.params.destination.symbol} delivered`, this.now())],
            },
          };
        });
      }
      case 'done': {
        if (!s.leftover || sim.leftoverSent || !s.params.returnAddress) return undefined;
        return taker(PROOF_MS, () => ({
          stage: 'done',
          patch: { leftover: { toAddress: s.params.returnAddress }, needs: null, addTxs: [this.tx('leftover', 'zcash', 'Leftover ZEC to your return address', this.now())] },
          record: { leftoverSent: true },
        }), true);
      }
      case 'recovering':
        return taker(TYPICAL_MS.recovering!, () => ({ stage: 'recovered', patch: { addTxs: [this.tx('recover', 'zcash', 'ZEC taken back from the shared address', this.now())] } }), true);
      case 'recovered':
        return taker(1_000, () => ({ stage: 'refunding', patch: { needs: s.params.returnAddress ? null : 'return_address' } }));
      case 'refunding': {
        if (!s.params.returnAddress) return undefined;
        return taker(TYPICAL_MS.refunding!, () => ({
          stage: 'refunded',
          patch: { refund: { amountZats: this.refundAmount(s).toString(), toAddress: s.params.returnAddress }, needs: null, addTxs: [this.tx('refund', 'zcash', 'ZEC back to your return address', this.now())] },
        }), true);
      }
      default:
        return undefined;
    }
  }

  private directoryEscrow(): string {
    return this.escrowContract;
  }

  /** What the temporary wallet holds at refund time, minus the refund's own fee. */
  private refundAmount(s: SwapState): bigint {
    const fee = this.terms!.feePerSendZats;
    let balance = BigInt(s.deposit.receivedZats);
    const sent = (id: string) => s.txs.some((t) => t.id === id);
    if (sent('request')) balance -= fee;
    if (sent('accept')) balance -= fee + BigInt(s.params.shown.premiumZats);
    if (sent('funding')) balance -= fee + BigInt(s.funding?.amountZats ?? '0');
    if (sent('recover')) balance += BigInt(s.funding?.amountZats ?? '0') - fee;
    balance -= fee;
    return balance > 0n ? balance : 0n;
  }

  private activityFor(s: SwapState): Activity | null {
    const now = this.now();
    const proof = (label: string): Activity => ({ kind: 'proving', label, startedAt: Math.max(now, this.keysReadyAt), estimatedMs: this.scaled(PROOF_MS) });
    const wait = (label: string, ms: number): Activity => ({ kind: 'waiting', label, startedAt: now, estimatedMs: this.scaled(ms) });
    switch (s.stage) {
      case 'deposit_seen':
        return wait('Waiting for your payment to be mined in a Zcash block', ZCASH_BLOCK_MS);
      case 'deposit_confirmed':
        return proof('Building the private request to the maker');
      case 'request_sent':
        return wait('Waiting for the request and the maker\'s answer to be mined', TYPICAL_MS.request_sent!);
      case 'quote_received':
        return proof('Building the acceptance and the premium payment');
      case 'accepted':
        return wait('Waiting for the maker to lock the USDC on NEAR', TYPICAL_MS.accepted!);
      case 'escrow_open':
        return proof('Building the payment to the shared address');
      case 'funding_sent':
      case 'funding_confirming': {
        const n = (s.funding?.confirmations ?? 0) + 1;
        if (n > (s.funding?.required ?? FUNDING_CONFIRMATIONS)) return wait('Waiting for the maker to set the escrow to Ready', NEAR_FINAL_MS);
        return wait(`Waiting for Zcash block ${n} of ${s.funding?.required ?? FUNDING_CONFIRMATIONS}`, ZCASH_BLOCK_MS);
      }
      case 'ready':
        return { kind: 'signing', label: 'Signing the claim commitment with your NEAR key', startedAt: now, estimatedMs: this.scaled(2_000) };
      case 'committing':
        return wait('Waiting for the commitment to be final on NEAR', NEAR_FINAL_MS);
      case 'claiming':
        return wait('Waiting for the claim to be final on NEAR', NEAR_FINAL_MS);
      case 'claimed':
        return wait('Moving the USDC into NEAR Intents', NEAR_FINAL_MS);
      case 'exiting':
        return wait('NEAR Intents is delivering your tokens', TYPICAL_MS.exiting!);
      case 'recovering':
        return proof('Building the transaction that takes your ZEC back');
      case 'refunding':
        return s.params.returnAddress ? proof('Building the refund to your return address') : null;
      default:
        return null;
    }
  }

  private async startActivity(): Promise<void> {
    const s = this.state;
    if (!s) return;
    await this.emit({ type: 'activity', swapId: s.id, activity: this.activityFor(s) });
  }

  /** Runs engine work one piece at a time, in order. */
  private enqueue(fn: () => Promise<void>): void {
    this.queue = this.queue.then(fn).catch((e) => {
      this.emitWire({ type: 'error', swapId: this.state?.id, error: { code: 'INTERNAL', message: (e as Error).message } });
    });
  }

  private async fire(step: Step): Promise<void> {
    if (this.pending !== step || !this.state) return;
    this.pending = undefined;
    const before = this.state.stage;
    const batch = step.run();
    if (batch.record) Object.assign(this.chain!.sim, batch.record);
    await this.emit({ type: 'stage', swapId: this.state.id, stage: batch.stage, at: Math.min(step.at, this.now()), patch: batch.patch });
    if (batch.stage !== before || batch.stage === 'funding_confirming') await this.startActivity();
    this.pump();
  }

  private busy(ms: number) {
    const end = performance.now() + ms;
    while (performance.now() < end) {
      // Stand-in for the prover, which blocks the worker thread.
    }
  }

  /** Schedules the next step of the current swap. Steps already due run at once, in order. */
  private pump(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    const step = this.nextStep();
    this.pending = step;
    if (!step) return;
    let wait = step.at - this.now();
    let burn = 0;
    if (step.proving && this.options.burnCpu && wait > 0) {
      burn = Math.min(this.scaled(PROOF_MS), wait, 30_000);
      wait -= burn;
    }
    this.timer = setTimeout(() => {
      this.timer = undefined;
      if (burn > 0) this.busy(burn);
      this.enqueue(() => this.fire(step));
    }, Math.max(0, wait));
  }
}

export type { ErrorCode };
