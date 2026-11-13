// A claimed swap whose payout already left the taker key: a swap restored after its exit.
// The decision (src/engine/real/payoutCheck.ts) and the real engine's use of it, with
// scripted escrow and Verifier answers and a scripted stand-in for the core.
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CreateSwapParams, EngineEvent, SwapState } from '../../src/engine/api';
import { memoryRecords } from '../../src/engine/real/idb';
import { GONE_STABLE_MS, judgePayout, type PayoutFacts, type PayoutReader } from '../../src/engine/real/payoutCheck';
import { RealEngine } from '../../src/engine/real/realEngine';
import type { WasmModule, WasmTaker } from '../../src/engine/real/wasm';
import { parseNetworkConfig } from '../../src/lib/netconfig';

const UA =
  'utest13x4neahaknjvdwe5j4h5le2zhfh502nava4jf5g43z9n8ynk9g5whlvsztnn9lm88eq83zzpvsjst0vnfda32lywxv9uz8qgk8wtgr84d6da980fg0wrj2en9hnywv4khl35008f263gztnlqa3zf0he97k0ahttlmmhm7e7ycmn45nm';
const SWAP = 'ab'.repeat(16);
const ESCROW_ID = 'ee'.repeat(32);
const TAKER = '04'.repeat(32);
const PRICE = '1487700';
const cfg = parseNetworkConfig(JSON.parse(readFileSync(new URL('../../public/config/testnet.json', import.meta.url), 'utf8')));
const directory = {
  format: 'zecret-maker-directory',
  version: 1,
  network: 'testnet',
  updatedAt: '2026-11-13T00:00:00Z',
  status: 'online',
  maker: { name: 'Test maker', zcashAddress: UA, requestKey: '11'.repeat(32), nearAccount: 'maker.test', escrowContract: cfg.near.escrow, nearPublicKey: 'ed25519:x', epoch: 0 },
  rate: { usdcPerZec: '940', validForSeconds: 300 },
  limits: { minZec: '0.01', maxZec: '2' },
  premium: { bps: 0, minZec: '0.0045' },
  zcashFeePerSendZec: '0.0001',
  signature: 'sig',
};
const params: CreateSwapParams = {
  amountZats: '100000000',
  destination: { assetId: 'nep141:usdc', symbol: 'USDC', chain: 'base', decimals: 6, address: '' },
  directExit: { nearAccount: 'alice.testnet' },
  returnAddress: UA,
  minUsdc: '1',
  minDestinationAmount: '1',
  shown: { usdcExpected: '1487700', destinationExpected: '1487700', premiumZats: '1000', networkFeeZats: '30000' },
};

const facts = (creditIntents: string, verifierBalance: string, escrowState: string | undefined = 'Claimed'): PayoutFacts => ({ escrowState, amount: PRICE, creditIntents, verifierBalance });

describe('judgePayout', () => {
  it('not yet paid: the escrow still holds the credit (also when the maker never withdraws)', () => {
    expect(judgePayout(facts(PRICE, '0'))).toBe('not_paid');
  });
  it('not yet paid: the claim is not visible at a final block', () => {
    expect(judgePayout(facts('0', '0', 'Locked'))).toBe('not_paid');
    expect(judgePayout({ ...facts('0', '0'), escrowState: undefined })).toBe('not_paid');
  });
  it('paid and still in the Verifier', () => {
    expect(judgePayout(facts('0', PRICE))).toBe('in_verifier');
  });
  it('paid and already moved out of the taker key', () => {
    expect(judgePayout(facts('0', '0'))).toBe('gone');
  });
});

/** The core, waiting in `Claimed` as `Taker::on_claimed` does: it completes once the Verifier holds the price. */
class Core {
  stage = 'Claimed';
  chain: PayoutFacts = facts(PRICE, '0');
  walletZats = 95_000_000;
  leftoverTx?: string;
  /** Whether the leftover transaction is in a block. */
  leftoverMined = true;
  /** Sends the node refuses; a refused send leaves the notes reserved in that taker, as the wallet does. */
  refuse = 0;
  takers = 0;
  module(): WasmModule {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const core = this;
    const swap = () => JSON.stringify({ rfq_id: SWAP, stage: core.stage, zats: 4_959_000, near_sk: '00', taker_account: TAKER, quote: { price: PRICE, premium: 1000, stipend: '1' }, escrow_id: ESCROW_ID, log: [] });
    class FakeTaker implements WasmTaker {
      reserved = false;
      constructor() {
        core.takers++;
      }
      ua = () => UA;
      setMaxPremium() {}
      dirty = () => '[]';
      record = () => swap();
      start = () => swap();
      tick(now: number) {
        if (core.stage === 'Claimed' && core.chain.creditIntents === '0' && BigInt(core.chain.verifierBalance) >= BigInt(PRICE)) {
          core.stage = 'Completed';
          return JSON.stringify({
            next: { kind: 'done' },
            events: [
              { type: 'payout', swap: SWAP, account: TAKER, token: cfg.near.token, amount: core.chain.verifierBalance },
              { type: 'stage', swap: SWAP, stage: 'Completed' },
            ],
          });
        }
        return JSON.stringify({ next: core.stage === 'Completed' ? { kind: 'done' } : { kind: 'at', at: now + 2 }, events: [] });
      }
      reconcile = () => false;
      swap = () => swap();
      facts = () => JSON.stringify({ balance: core.leftoverTx || this.reserved ? 0 : core.walletZats, height: 10, fundHeight: 5, sweepHeight: null });
      sync = () => 10;
      txHeight = (txid: string) => (txid === core.leftoverTx && core.leftoverMined ? 11 : undefined);
      escrowState = () => core.chain.escrowState;
      intentsBalance = () => core.chain.verifierBalance;
      tokenBalance = () => '0';
      returnLeftover = () => {
        if (core.refuse > 0) {
          core.refuse--;
          this.reserved = true;
          throw new Error('transaction refused: unknown anchor');
        }
        return (core.leftoverTx = 'leftovertx');
      };
      free() {}
    }
    return {
      default: async () => undefined,
      WasmTaker: FakeTaker,
      prepareProvingKey: () => 0,
      requiredDeposit: (_z: number, premium: number) => 40_000 + premium,
      feePerSend: () => 10_000,
      zcashTip: () => 1000,
      checkAddress: () => true,
      verifyMaker: () => undefined,
      nearAccount: () => TAKER,
      // The direct exit: the relay is busy, so the exit stays in `exiting`.
      directPrepare: () => JSON.stringify({ ok: false, error: { code: 'NETWORK', message: 'busy', transient: true } }),
      directRelay: () => '',
      exitQuote: () => '',
      exitSubmit: () => '',
      exitStatus: () => '',
    } as unknown as WasmModule;
  }
}

/** A restored swap that the restore moved to `claimed` (the browser closed at `done`, before the leftover). */
function stored(stage: SwapState['stage'] = 'claimed', meta: Record<string, unknown> = {}): Map<string, string> {
  const state: SwapState = {
    version: 1,
    id: SWAP,
    engine: 'wasm',
    network: 'testnet',
    createdAt: 1,
    stage,
    stageAt: 1,
    history: [{ stage, at: 1 }],
    params,
    deposit: { address: UA, uri: '', expectedZats: params.amountZats, receivedZats: params.amountZats, confirmations: 1, requiredConfirmations: 1 },
    txs: [],
    needs: null,
    activity: null,
    restoredAt: 1,
  };
  return new Map([
    ['web/state', JSON.stringify(state)],
    ['web/meta', JSON.stringify({ version: 1, swapId: SWAP, maxPremium: 1000, restoreHeight: 1, restoreDone: true, ...meta })],
    ['wallet', 'k'],
    [`swap/${SWAP}`, '{}'],
  ]);
}

async function start(core: Core, records = stored()) {
  const reads: PayoutFacts[] = [];
  const reader: PayoutReader = async (q) => {
    expect(q).toMatchObject({ escrow: cfg.near.escrow, verifier: cfg.near.verifier, token: cfg.near.token, swapId: ESCROW_ID, taker: TAKER });
    reads.push(core.chain);
    return core.chain;
  };
  const engine = new RealEngine(memoryRecords(records), '/', async () => core.module(), reader);
  const events: EngineEvent[] = [];
  engine.attach((e) => events.push(e));
  await engine.init({ network: 'testnet', directory, real: cfg });
  await engine.resume();
  return { engine, events, reads };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('a restored claimed swap', () => {
  it('not yet paid: waits in claimed while the escrow holds the credit', async () => {
    const core = new Core();
    core.chain = facts(PRICE, '0');
    const { engine, reads } = await start(core);
    await vi.advanceTimersByTimeAsync(10 * GONE_STABLE_MS);
    const s = (await engine.getState())!;
    expect(reads.length).toBeGreaterThan(3);
    expect(s.stage).toBe('claimed');
    expect(s.exit).toBeUndefined();
    expect(core.leftoverTx).toBeUndefined();
  });

  it('PayoutPending (the maker never withdraws): stays on claimed', async () => {
    const core = new Core();
    core.stage = 'PayoutPending';
    core.chain = facts(PRICE, '0');
    const { engine } = await start(core);
    await vi.advanceTimersByTimeAsync(10 * GONE_STABLE_MS);
    expect((await engine.getState())!.stage).toBe('claimed');
    expect(core.leftoverTx).toBeUndefined();
  });

  it('paid and still in the Verifier: the core completes and the engine runs the exit', async () => {
    const core = new Core();
    core.chain = facts('0', PRICE);
    const { engine } = await start(core);
    await vi.advanceTimersByTimeAsync(GONE_STABLE_MS * 2);
    const s = (await engine.getState())!;
    expect(s.history.map((h) => h.stage)).toContain('paid_out');
    expect(s.stage).toBe('exiting');
    expect(s.payout).toEqual({ usdc: PRICE, intentsAccount: TAKER });
    expect(s.exit?.doneElsewhere).toBeUndefined();
    expect(core.leftoverTx).toBeUndefined();
  });

  it('paid and already withdrawn: done (delivery not invented), then the leftover goes to the return address', async () => {
    const core = new Core();
    core.chain = facts('0', '0');
    const { engine } = await start(core);
    // Not before the answer held for GONE_STABLE_MS.
    await vi.advanceTimersByTimeAsync(GONE_STABLE_MS / 2);
    expect((await engine.getState())!.stage).toBe('claimed');
    await vi.advanceTimersByTimeAsync(GONE_STABLE_MS * 2);
    const s = (await engine.getState())!;
    expect(s.stage).toBe('done');
    expect(s.history.map((h) => h.stage)).toEqual(['claimed', 'paid_out', 'done']);
    expect(s.payout).toEqual({ usdc: PRICE, intentsAccount: TAKER });
    expect(s.exit).toMatchObject({ phase: 'done', doneElsewhere: true, direct: { nearAccount: 'alice.testnet' } });
    expect(s.exit?.deliveredAmount).toBeUndefined();
    expect(s.txs.find((t) => t.id === 'delivery')).toBeUndefined();
    expect(core.leftoverTx).toBe('leftovertx');
    expect(s.leftover).toEqual({ amountZats: String(95_000_000 - 10_000), toAddress: UA, returned: true });
    expect(s.txs.find((t) => t.id === 'leftover')?.hash).toBe('leftovertx');
  });

  it('paid and withdrawn without a return address: done, and asks for one', async () => {
    const core = new Core();
    core.chain = facts('0', '0');
    const records = stored();
    const st = JSON.parse(records.get('web/state')!) as SwapState;
    records.set('web/state', JSON.stringify({ ...st, params: { ...st.params, returnAddress: undefined } }));
    const { engine } = await start(core, records);
    await vi.advanceTimersByTimeAsync(GONE_STABLE_MS * 3);
    const s = (await engine.getState())!;
    expect(s.stage).toBe('done');
    expect(s.needs).toBe('return_address');
    expect(s.leftover).toEqual({ amountZats: String(95_000_000 - 10_000) });
    await engine.setReturnAddress(UA);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(core.leftoverTx).toBe('leftovertx');
  });

  it('a withdraw in flight (credit 0, Verifier 0 for a moment) is not read as an exit', async () => {
    const core = new Core();
    core.chain = facts('0', '0');
    const { engine } = await start(core);
    await vi.advanceTimersByTimeAsync(GONE_STABLE_MS / 2);
    core.chain = facts('0', PRICE);
    await vi.advanceTimersByTimeAsync(GONE_STABLE_MS * 2);
    const s = (await engine.getState())!;
    expect(s.stage).toBe('exiting');
    expect(s.exit?.doneElsewhere).toBeUndefined();
  });
});

describe('a reload at done before the leftover was sent', () => {
  it('sends the leftover', async () => {
    const core = new Core();
    core.stage = 'Completed';
    const { engine } = await start(core, stored('done', { coreDone: true }));
    await vi.advanceTimersByTimeAsync(10_000);
    expect(core.leftoverTx).toBe('leftovertx');
    expect((await engine.getState())!.leftover?.toAddress).toBe(UA);
  });
});

describe('a refused leftover send', () => {
  it('opens the swap again (the refused send no longer reserves the notes) and sends again', async () => {
    const core = new Core();
    core.stage = 'Completed';
    core.refuse = 1;
    const { engine } = await start(core, stored('done', { coreDone: true }));
    await vi.advanceTimersByTimeAsync(30_000);
    expect(core.takers).toBe(2);
    expect(core.leftoverTx).toBe('leftovertx');
    expect((await engine.getState())!.leftover?.toAddress).toBe(UA);
  });
});

describe('the leftover guard', () => {
  const finished = () => stored('done', { coreDone: true });

  it('refuses to delete the swap or start another until the engine has looked for a leftover', async () => {
    const core = new Core();
    core.stage = 'Completed';
    const { engine } = await start(core, finished());
    // resume() scheduled the leftover step; it has not run yet.
    await expect(engine.forget()).rejects.toMatchObject({ code: 'LEFTOVER_PENDING', transient: true });
    await expect(engine.createSwap(params)).rejects.toMatchObject({ code: 'LEFTOVER_PENDING' });
    expect((await engine.getState())?.stage).toBe('done');
  });

  it('keeps the keys while the leftover transaction is not mined, and allows a new swap after', async () => {
    const core = new Core();
    core.stage = 'Completed';
    core.leftoverMined = false;
    const { engine } = await start(core, finished());
    await vi.advanceTimersByTimeAsync(10_000);
    let s = (await engine.getState())!;
    expect(s.leftover).toEqual({ amountZats: String(95_000_000 - 10_000), toAddress: UA });
    await expect(engine.forget()).rejects.toMatchObject({ code: 'LEFTOVER_PENDING', message: expect.stringContaining('0.9499 ZEC') });
    await expect(engine.createSwap(params)).rejects.toMatchObject({ code: 'LEFTOVER_PENDING' });
    core.leftoverMined = true;
    await vi.advanceTimersByTimeAsync(5_000);
    s = (await engine.getState())!;
    expect(s.leftover?.returned).toBe(true);
    await engine.forget();
    expect(await engine.getState()).toBeNull();
  });

  it('without a return address: refuses, then deletes when the user discards the leftover', async () => {
    const core = new Core();
    core.stage = 'Completed';
    const records = finished();
    const st = JSON.parse(records.get('web/state')!) as SwapState;
    records.set('web/state', JSON.stringify({ ...st, params: { ...st.params, returnAddress: undefined } }));
    const { engine } = await start(core, records);
    await vi.advanceTimersByTimeAsync(10_000);
    expect((await engine.getState())!.needs).toBe('return_address');
    await expect(engine.forget()).rejects.toMatchObject({ code: 'LEFTOVER_PENDING', message: expect.stringContaining('Enter a return address') });
    await engine.forget({ discardLeftover: true });
    expect(await engine.getState()).toBeNull();
    expect(core.leftoverTx).toBeUndefined();
  });

  it('a swap stored before `returned` existed, with its leftover mined, can be deleted', async () => {
    const core = new Core();
    core.stage = 'Completed';
    const records = stored('done', { coreDone: true, leftoverDone: true, leftoverTxid: 'leftovertx' });
    const st = JSON.parse(records.get('web/state')!) as SwapState;
    records.set('web/state', JSON.stringify({ ...st, leftover: { amountZats: '94990000', toAddress: UA } }));
    const { engine } = await start(core, records);
    expect((await engine.getState())!.leftover?.returned).toBe(true);
    await engine.forget();
    expect(await engine.getState()).toBeNull();
  });
});
