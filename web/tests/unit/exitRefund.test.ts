// Security review 6: the 1Click exit believes the Verifier balance of the taker key, not
// only 1Click's unsigned status. The decisions (src/engine/real/exitPlan.ts) and the real
// engine's use of them, with a scripted stand-in for the core and for the 1Click calls.
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineEvent, SwapState } from '../../src/engine/api';
import { afterFinalStatus, believeSuccess, planExitAmount, REFUND_WATCH_MS, restoredExit } from '../../src/engine/real/exitPlan';
import { memoryRecords } from '../../src/engine/real/idb';
import type { PayoutFacts, PayoutReader } from '../../src/engine/real/payoutCheck';
import { RealEngine } from '../../src/engine/real/realEngine';
import type { WasmModule, WasmTaker } from '../../src/engine/real/wasm';
import { parseNetworkConfig, type NetworkConfig } from '../../src/lib/netconfig';

describe('planExitAmount', () => {
  it('moves the payout when the key holds it, with the approved minimum', () => {
    expect(planExitAmount(2_000_000n, 1_970_000n, 0n, 2_000_000n)).toEqual({ kind: 'quote', amountIn: 2_000_000n, minAmountOut: 1_970_000n });
  });
  it('never moves more than the payout (tokens that were not part of this swap stay)', () => {
    expect(planExitAmount(2_000_000n, 1_970_000n, 0n, 2_000_005n)).toEqual({ kind: 'quote', amountIn: 2_000_000n, minAmountOut: 1_970_000n });
  });
  it('moves what came back after a short refund, at the same rate, rounded up', () => {
    // 1_970_000 * 1_000_001 / 2_000_000 = 985_000.985 -> 985_001
    expect(planExitAmount(2_000_000n, 1_970_000n, 0n, 1_000_001n)).toEqual({ kind: 'quote', amountIn: 1_000_001n, minAmountOut: 985_001n });
  });
  it('keeps an accepted offer exact for the amount it was offered for', () => {
    expect(planExitAmount(2_000_000n, 900_000n, 1_000_000n, 1_000_000n)).toEqual({ kind: 'quote', amountIn: 1_000_000n, minAmountOut: 900_000n });
  });
  it('quotes nothing when the key holds nothing', () => {
    expect(planExitAmount(2_000_000n, 1n, 0n, 0n)).toEqual({ kind: 'empty' });
  });
});

describe('believeSuccess, afterFinalStatus, restoredExit', () => {
  it('believes SUCCESS only after the signed USDC left the key', () => {
    expect(believeSuccess(2_000_000n, 2_000_000n)).toBe(false);
    expect(believeSuccess(0n, 2_000_000n)).toBe(true);
    expect(believeSuccess(5n, 2_000_000n)).toBe(true); // dust stays
  });
  it('waits for a refund that is not under the key yet', () => {
    expect(afterFinalStatus(0n)).toBe('wait');
    expect(afterFinalStatus(1n)).toBe('back');
  });
  it('follows the last deposit address of the file, or watches for a refund', () => {
    expect(restoredExit(['a', 'b'])).toEqual({ kind: 'follow', depositAddress: 'b' });
    expect(restoredExit([])).toEqual({ kind: 'watch' });
    expect(restoredExit(undefined)).toEqual({ kind: 'watch' });
  });
});

// ---------------------------------------------------------------- the engine

const UA =
  'utest13x4neahaknjvdwe5j4h5le2zhfh502nava4jf5g43z9n8ynk9g5whlvsztnn9lm88eq83zzpvsjst0vnfda32lywxv9uz8qgk8wtgr84d6da980fg0wrj2en9hnywv4khl35008f263gztnlqa3zf0he97k0ahttlmmhm7e7ycmn45nm';
const SWAP = 'ab'.repeat(16);
const ESCROW_ID = 'ee'.repeat(32);
const TAKER = '04'.repeat(32);
const PRICE = '2000000';
const DEPOSIT = 'd1'.repeat(32);
const testnet = parseNetworkConfig(JSON.parse(readFileSync(new URL('../../public/config/testnet.json', import.meta.url), 'utf8')));
/** The testnet file with the 1Click exit, as mainnet runs it. */
const cfg: NetworkConfig = { ...testnet, exit: { mode: 'oneclick', oneClickUrl: 'https://1click.chaindefuser.com' } };
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
const params = {
  amountZats: '100000000',
  destination: { assetId: 'nep141:base-usdc', symbol: 'USDC', chain: 'base', decimals: 6, address: '0x1111111111111111111111111111111111111111' },
  returnAddress: UA,
  minUsdc: '1',
  minDestinationAmount: '1970000',
  shown: { usdcExpected: PRICE, destinationExpected: PRICE, premiumZats: '1000', networkFeeZats: '30000' },
};

type Status = { status: string; isFinal: boolean; amountOut?: string; destinationTxs: { hash: string; url?: string }[]; refundReason?: string };
const ok = (value: unknown) => JSON.stringify({ ok: true, value });

class Core {
  stage = 'Claimed';
  chain: PayoutFacts = { escrowState: 'Claimed', amount: PRICE, payout: 'paid', verifierBalance: '0' };
  /** The taker key's balance in the Verifier that the exit reads. */
  balance = '0';
  status: Status = { status: 'PROCESSING', isFinal: false, destinationTxs: [] };
  quotes: { amountIn: string; minAmountOut: string }[] = [];
  submits = 0;
  module(): WasmModule {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const core = this;
    const swap = () => JSON.stringify({ rfq_id: SWAP, stage: core.stage, zats: 4_959_000, near_sk: '00', taker_account: TAKER, quote: { price: PRICE, premium: 1000, stipend: '1' }, escrow_id: ESCROW_ID, log: [] });
    class FakeTaker implements WasmTaker {
      ua = () => UA;
      setMaxPremium() {}
      dirty = () => '[]';
      record = () => swap();
      start = () => swap();
      tick = (now: number) => JSON.stringify({ next: core.stage === 'Completed' ? { kind: 'done' } : { kind: 'at', at: now + 2 }, events: [] });
      reconcile = () => false;
      swap = () => swap();
      facts = () => JSON.stringify({ balance: 0, height: 10, fundHeight: 5, sweepHeight: null });
      sync = () => 10;
      txHeight = () => undefined;
      escrowState = () => core.chain.escrowState;
      intentsBalance = () => core.balance;
      tokenBalance = () => '0';
      returnLeftover = () => 'leftovertx';
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
      directPrepare: () => '',
      directRelay: () => '',
      exitQuote: (_b: string, approval: string) => {
        const a = JSON.parse(approval) as { amountIn: string; minAmountOut: string };
        core.quotes.push(a);
        return ok({ raw: {}, amountIn: a.amountIn, amountOut: a.minAmountOut, minAmountOut: a.minAmountOut, depositAddress: DEPOSIT, feeBps: 1 });
      },
      exitSubmit: () => {
        core.submits++;
        return ok({ intentHash: 'hash', depositAddress: DEPOSIT });
      },
      exitStatus: () => ok(core.status),
    } as unknown as WasmModule;
  }
}

function stored(stage: SwapState['stage'], exit: SwapState['exit'] | undefined, meta: Record<string, unknown>): Map<string, string> {
  const state = {
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
    ...(stage !== 'claimed' ? { payout: { usdc: PRICE, intentsAccount: TAKER } } : {}),
    ...(exit ? { exit } : {}),
  } as SwapState;
  return new Map([
    ['web/state', JSON.stringify(state)],
    ['web/meta', JSON.stringify({ version: 1, swapId: SWAP, maxPremium: 1000, restoreHeight: 1, restoreDone: true, leftoverDone: true, ...meta })],
    ['wallet', 'k'],
    [`swap/${SWAP}`, '{}'],
  ]);
}

async function start(core: Core, records: Map<string, string>) {
  const reader: PayoutReader = async () => core.chain;
  const engine = new RealEngine(memoryRecords(records), '/', async () => core.module(), reader);
  const events: EngineEvent[] = [];
  engine.attach((e) => events.push(e));
  await engine.init({ network: 'testnet', directory, real: cfg });
  await engine.resume();
  return { engine, events };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const delivering = (meta: Record<string, unknown> = {}) =>
  stored('exiting', { attempts: 1, phase: 'delivering', approvedMinimum: '1970000', depositAddress: DEPOSIT }, { coreDone: true, signedDeposits: [DEPOSIT], signedAmounts: { [DEPOSIT]: PRICE }, exitSubmitted: { depositAddress: DEPOSIT, intentHash: 'h' }, ...meta });

describe('the 1Click exit reads the key balance', () => {
  it('a SUCCESS while the key still holds the signed USDC is not believed; it is once the USDC left', async () => {
    const core = new Core();
    core.stage = 'Completed';
    core.balance = PRICE;
    core.status = { status: 'SUCCESS', isFinal: true, amountOut: '1990000', destinationTxs: [{ hash: '0xabc', url: 'https://basescan.org/tx/0xabc' }] };
    const { engine } = await start(core, delivering());
    await vi.advanceTimersByTimeAsync(60_000);
    expect((await engine.getState())!.stage).toBe('exiting');
    core.balance = '0';
    await vi.advanceTimersByTimeAsync(20_000);
    const s = (await engine.getState())!;
    expect(s.stage).toBe('done');
    expect(s.exit?.deliveredAmount).toBe('1990000');
  });

  it('REFUNDED with the USDC not back yet keeps watching; when it is back the user can exit again', async () => {
    const core = new Core();
    core.stage = 'Completed';
    core.balance = '0';
    core.status = { status: 'REFUNDED', isFinal: true, destinationTxs: [], refundReason: 'price moved' };
    const { engine } = await start(core, delivering());
    await vi.advanceTimersByTimeAsync(120_000);
    let s = (await engine.getState())!;
    expect(s.stage).toBe('exiting');
    expect(s.lastError).toMatchObject({ code: 'EXIT_FAILED', transient: true });
    expect(s.lastError?.message).toContain(DEPOSIT);
    core.balance = PRICE;
    await vi.advanceTimersByTimeAsync(31_000);
    s = (await engine.getState())!;
    expect(s.stage).toBe('exit_retry');
    expect(s.exit?.phase).toBe('refunded');
    expect(s.lastError?.message).toContain('2 USDC is under your key');
    // Trying again quotes what is under the key.
    await engine.exit();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(core.quotes.at(-1)).toEqual(expect.objectContaining({ amountIn: PRICE, minAmountOut: '1970000' }));
  });

  it('a refund that came back short is quoted as it is, at the approved rate', async () => {
    const core = new Core();
    core.stage = 'Completed';
    core.balance = '1000000';
    const { engine } = await start(core, stored('exit_retry', { attempts: 1, phase: 'refunded', approvedMinimum: '1970000' }, { coreDone: true }));
    await engine.exit();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(core.quotes.at(-1)).toEqual(expect.objectContaining({ amountIn: '1000000', minAmountOut: '985000' }));
  });

  it('with nothing under the key, a new attempt follows the deposit address it signed for instead of signing again', async () => {
    const core = new Core();
    core.stage = 'Completed';
    core.balance = '0';
    core.status = { status: 'PROCESSING', isFinal: false, destinationTxs: [] };
    const { engine } = await start(core, stored('exit_retry', { attempts: 1, phase: 'failed', approvedMinimum: '1970000' }, { coreDone: true, signedDeposits: [DEPOSIT] }));
    await engine.exit();
    await vi.advanceTimersByTimeAsync(10_000);
    const s = (await engine.getState())!;
    expect(core.quotes).toHaveLength(0);
    expect(core.submits).toBe(0);
    expect(s.exit).toMatchObject({ phase: 'delivering', depositAddress: DEPOSIT });
  });
});

describe('a swap restored after its 1Click exit ran elsewhere', () => {
  it('is done, and a later refund under the key moves it to exit_retry', async () => {
    const core = new Core();
    core.chain = { escrowState: 'Claimed', amount: PRICE, payout: 'paid', verifierBalance: '0' };
    const { engine } = await start(core, stored('claimed', undefined, { leftoverDone: true }));
    await vi.advanceTimersByTimeAsync(30_000);
    let s = (await engine.getState())!;
    expect(s.stage).toBe('done');
    expect(s.exit).toMatchObject({ doneElsewhere: true });
    // Hours later, 1Click refunds the exit that ran in the other browser.
    await vi.advanceTimersByTimeAsync(3 * 3600_000);
    core.balance = PRICE;
    await vi.advanceTimersByTimeAsync(61_000);
    s = (await engine.getState())!;
    expect(s.stage).toBe('exit_retry');
    expect(s.needs).toBe('exit_decision');
    expect(s.exit).toMatchObject({ phase: 'refunded', doneElsewhere: false });
  });

  it('stops watching after REFUND_WATCH_MS', async () => {
    const core = new Core();
    const { engine } = await start(core, stored('claimed', undefined, { leftoverDone: true }));
    await vi.advanceTimersByTimeAsync(30_000);
    await vi.advanceTimersByTimeAsync(REFUND_WATCH_MS + 120_000);
    core.balance = PRICE;
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect((await engine.getState())!.stage).toBe('done');
  });

  it('with a deposit address in the file, follows it to the delivery', async () => {
    const core = new Core();
    core.status = { status: 'SUCCESS', isFinal: true, amountOut: '1990000', destinationTxs: [] };
    const { engine } = await start(core, stored('claimed', undefined, { leftoverDone: true, signedDeposits: [DEPOSIT] }));
    await vi.advanceTimersByTimeAsync(30_000);
    const s = (await engine.getState())!;
    expect(s.history.map((h) => h.stage)).toContain('exiting');
    expect(s.stage).toBe('done');
    expect(s.exit?.deliveredAmount).toBe('1990000');
    expect(s.exit?.doneElsewhere).toBeUndefined();
  });
});
