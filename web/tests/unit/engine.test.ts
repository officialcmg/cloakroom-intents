import { describe, expect, it } from 'vitest';
import sample from '../../public/maker-directory.json';
import type { CreateSwapParams, EngineEvent, Stage, SwapState, SimFailure } from '../../src/engine/api';
import { EngineError, leftoverPending } from '../../src/engine/api';
import { applyPatch, reduceSwap } from '../../src/engine/reducer';
import { connectEngine, serveEngine, type EngineClient } from '../../src/engine/rpc';
import { SimEngine } from '../../src/engine/sim/simEngine';
import { memoryStore, type KvStore } from '../../src/engine/sim/store';
import { remainingMs } from '../../src/engine/timeline';

const RETURN = 'utest13x4neahaknjvdwe5j4h5le2zhfh502nava4jf5g43z9n8ynk9g5whlvsztnn9lm88eq83zzpvsjst0vnfda32lywxv9uz8qgk8wtgr84d6da980fg0wrj2en9hnywv4khl35008f263gztnlqa3zf0he97k0ahttlmmhm7e7ycmn45nm';

function baseState(): SwapState {
  return {
    version: 1,
    id: 's1',
    engine: 'simulated',
    network: 'testnet',
    createdAt: 0,
    stage: 'awaiting_deposit',
    stageAt: 0,
    history: [{ stage: 'awaiting_deposit', at: 0 }],
    params: {
      amountZats: '100000000',
      destination: { assetId: 'nep141:sol.omft.near', symbol: 'SOL', chain: 'sol', decimals: 9, address: '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM' },
      minUsdc: '1',
      minDestinationAmount: '1',
      shown: { usdcExpected: '2', destinationExpected: '2', premiumZats: '200000', networkFeeZats: '30000' },
    },
    deposit: { address: 'utest1x', uri: 'zcash:utest1x?amount=1', expectedZats: '100000000', receivedZats: '0', confirmations: 0, requiredConfirmations: 1 },
    txs: [],
  };
}

describe('reducer', () => {
  it('replaces the swap on a swap event and ignores events for other swaps', () => {
    const s = baseState();
    expect(reduceSwap(null, { type: 'swap', swap: s })).toBe(s);
    expect(reduceSwap(s, { type: 'stage', swapId: 'other', stage: 'done', at: 5 })).toBe(s);
    expect(reduceSwap(null, { type: 'stage', swapId: 's1', stage: 'done', at: 5 })).toBeNull();
  });
  it('moves the stage, records history, merges patches one level deep and ends the old activity', () => {
    let s: SwapState | null = { ...baseState(), activity: { kind: 'waiting', label: 'x', startedAt: 0 } };
    s = reduceSwap(s, { type: 'stage', swapId: 's1', stage: 'deposit_seen', at: 10, patch: { deposit: { receivedZats: '100000000' } } })!;
    expect(s.stage).toBe('deposit_seen');
    expect(s.stageAt).toBe(10);
    expect(s.activity).toBeNull();
    expect(s.deposit.receivedZats).toBe('100000000');
    expect(s.deposit.address).toBe('utest1x');
    expect(s.history.map((h) => h.stage)).toEqual(['awaiting_deposit', 'deposit_seen']);
  });
  it('treats a repeated stage as an update: no new history entry, same stageAt', () => {
    let s: SwapState | null = { ...baseState(), stage: 'funding_confirming', stageAt: 3, history: [{ stage: 'funding_confirming', at: 3 }], funding: { amountZats: '1', confirmations: 1, required: 2 } };
    s = reduceSwap(s, { type: 'stage', swapId: 's1', stage: 'funding_confirming', at: 9, patch: { funding: { confirmations: 2 } } })!;
    expect(s.history).toHaveLength(1);
    expect(s.stageAt).toBe(3);
    expect(s.funding).toEqual({ amountZats: '1', confirmations: 2, required: 2 });
  });
  it('adds and replaces transactions by id, and clears fields with null', () => {
    let s = applyPatch(baseState(), { addTxs: [{ id: 'a', chain: 'zcash', label: 'A', hash: '1', at: 1 }], needs: 'return_address' });
    s = applyPatch(s, { addTxs: [{ id: 'a', chain: 'zcash', label: 'A', hash: '2', at: 2 }, { id: 'b', chain: 'near', label: 'B', hash: '3', at: 3 }], needs: null });
    expect(s.txs.map((t) => t.hash)).toEqual(['2', '3']);
    expect(s.needs).toBeNull();
  });
  it('applies activity and error events', () => {
    let s: SwapState | null = baseState();
    s = reduceSwap(s, { type: 'activity', swapId: 's1', activity: { kind: 'proving', label: 'p', startedAt: 1, estimatedMs: 2 } })!;
    expect(s.activity?.kind).toBe('proving');
    s = reduceSwap(s, { type: 'error', swapId: 's1', error: { code: 'NETWORK', message: 'x', transient: true } })!;
    expect(s.lastError?.code).toBe('NETWORK');
  });
});

describe('timeline estimates', () => {
  it('counts down along the normal path', () => {
    const fromStart = remainingMs('deposit_seen', 0)!;
    const later = remainingMs('ready', 0)!;
    expect(fromStart).toBeGreaterThan(8 * 60_000);
    expect(fromStart).toBeLessThan(12 * 60_000);
    expect(later).toBeLessThan(fromStart);
    expect(remainingMs('awaiting_deposit', 0)).toBeUndefined();
    expect(remainingMs('done', 0)).toBeUndefined();
  });
});

// ---------------------------------------------------------------- the simulated engine over the transport

function pair(store: KvStore = memoryStore()): { client: EngineClient; events: EngineEvent[]; close: () => void } {
  const ch = new MessageChannel();
  const backend = new SimEngine(store);
  serveEngine(ch.port2 as unknown as Parameters<typeof serveEngine>[0], backend);
  ch.port1.start();
  ch.port2.start();
  const client = connectEngine(ch.port1 as unknown as Parameters<typeof connectEngine>[0], () => {
    ch.port1.close();
    ch.port2.close();
  });
  const events: EngineEvent[] = [];
  client.subscribe((e) => events.push(e));
  return { client, events, close: () => client.dispose() };
}

const PARAMS: CreateSwapParams = {
  amountZats: '100000000',
  destination: { assetId: 'nep141:sol.omft.near', symbol: 'SOL', chain: 'sol', decimals: 9, address: '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM' },
  minUsdc: '928459620',
  minDestinationAmount: '11369160000',
  shown: { usdcExpected: '937838000', destinationExpected: '11600000000', premiumZats: '200000', networkFeeZats: '30000' },
};

function tracker(client: EngineClient) {
  let state: SwapState | null = null;
  client.subscribe((e) => (state = reduceSwap(state, e)));
  return {
    get: () => state,
    until: (pred: (s: SwapState) => boolean, ms = 8000) =>
      new Promise<SwapState>((resolve, reject) => {
        const t0 = Date.now();
        const tick = () => {
          if (state && pred(state)) return resolve(state);
          if (Date.now() - t0 > ms) return reject(new Error(`timeout; stage ${state?.stage}`));
          setTimeout(tick, 5);
        };
        tick();
      }),
  };
}

const at = (stage: Stage) => (s: SwapState) => s.stage === stage;

async function started(fail?: SimFailure, store?: KvStore) {
  const p = pair(store);
  const init = await p.client.init({ network: 'testnet', directory: sample, sim: { speed: 5000, fail } });
  expect(init).toMatchObject({ engine: 'simulated', phase: 'building_keys', timeScale: 5000 });
  return { ...p, t: tracker(p.client) };
}


describe('simulated engine', () => {
  it('runs the full journey and ends with the delivery', async () => {
    const { client, close, t, events } = await started();
    expect(await client.resume()).toBeNull();
    const s = await client.createSwap(PARAMS);
    expect(s.deposit.address).toMatch(/^utest1/);
    expect(s.deposit.uri).toBe(`zcash:${s.deposit.address}?amount=1`);
    await client.sim.pay();
    const paid = await t.until(at('paid_out'));
    expect(paid.payout?.usdc).toBe('937838000');
    expect(paid.escrow?.stipendYocto).toBe('60000000000000000000000');
    // The engine runs the exit by itself: firm price, check and sign, submit, deliver.
    const phases = new Set<string>();
    client.subscribe((e) => {
      if (e.type === 'stage' && e.patch?.exit?.phase) phases.add(e.patch.exit.phase);
    });
    const done = await t.until(at('done'));
    expect([...phases]).toEqual(['quoting', 'signing', 'submitted', 'delivering', 'done']);
    expect(done.exit).toMatchObject({ deliveredAmount: '11600000000', minAmountOut: '11484000000', approvedMinimum: PARAMS.minDestinationAmount, feeBps: 20 });
    expect(done.exit?.depositAddress).toMatch(/^[0-9a-f]{64}$/);
    expect(done.history.map((h) => h.stage)).toEqual([
      'awaiting_deposit', 'deposit_seen', 'deposit_confirmed', 'request_sent', 'quote_received', 'accepted', 'escrow_open',
      'funding_sent', 'funding_confirming', 'ready', 'committing', 'claiming', 'claimed', 'paid_out', 'exiting', 'done',
    ]);
    expect(done.txs.map((x) => x.id)).toEqual(['request', 'accept', 'open', 'stipend', 'funding', 'ready', 'commit', 'claim', 'withdraw', 'exit-1', 'delivery']);
    expect(events.some((e) => e.type === 'activity' && e.activity?.kind === 'proving')).toBe(true);
    expect(events.some((e) => e.type === 'init' && e.init.phase === 'ready')).toBe(true);
    // The page's copy of the state equals the engine's own.
    expect(await client.getState()).toEqual(done);
    close();
  });

  it('rejects bad input with stable codes', async () => {
    const { client, close } = await started();
    const fail = (p: Promise<unknown>) => p.then(() => { throw new Error('no error'); }, (e: EngineError) => e.code);
    expect(await fail(client.createSwap({ ...PARAMS, amountZats: '1' }))).toBe('INVALID_PARAMS');
    expect(await fail(client.createSwap({ ...PARAMS, destination: { ...PARAMS.destination, address: 'nope' } }))).toBe('INVALID_ADDRESS');
    expect(await fail(client.createSwap({ ...PARAMS, returnAddress: 't1Rv4exT7bqhZqi2j7xz8bUHDMxwosrjADU' }))).toBe('INVALID_ADDRESS');
    expect(await fail(client.exportRecovery())).toBe('NO_SWAP');
    await client.createSwap(PARAMS);
    expect(await fail(client.createSwap(PARAMS))).toBe('SWAP_EXISTS');
    expect(await fail(client.exit())).toBe('WRONG_STAGE');
    expect(await fail(client.init({ network: 'mainnet', directory: { ...sample, network: 'mainnet' } }))).toBe('INIT_FAILED');
    close();
  });

  it('keeps waiting after a short payment and shows what is still owed', async () => {
    const { client, close, t } = await started();
    const s = await client.createSwap(PARAMS);
    await client.sim.pay('40000000');
    const part = await t.until((x) => x.deposit.receivedZats === '40000000');
    expect(part.stage).toBe('awaiting_deposit');
    expect(part.deposit.uri).toBe(`zcash:${s.deposit.address}?amount=0.6`);
    await client.sim.pay();
    await t.until(at('deposit_seen'));
    close();
  });

  it('refunds when the maker quotes below the minimum, after asking for a return address', async () => {
    const { client, close, t } = await started('quote-low');
    await client.createSwap(PARAMS);
    await client.sim.pay();
    const r = await t.until(at('refunding'));
    expect(r.refund?.reason).toBe('QUOTE_BELOW_MINIMUM');
    expect(BigInt(r.quote!.usdc) < BigInt(PARAMS.minUsdc)).toBe(true);
    expect(r.needs).toBe('return_address');
    await client.setReturnAddress(RETURN);
    const done = await t.until(at('refunded'));
    // 1 ZEC minus the request fee and the refund fee. No premium was paid.
    expect(done.refund?.amountZats).toBe('99980000');
    expect(done.refund?.toAddress).toBe(RETURN);
    close();
  });

  it('refunds when the maker never answers', async () => {
    const { client, close, t } = await started('no-quote');
    await client.createSwap({ ...PARAMS, returnAddress: RETURN });
    await client.sim.pay();
    const done = await t.until(at('refunded'));
    expect(done.refund?.reason).toBe('MAKER_TIMEOUT');
    close();
  });

  it('recovers the ZEC when the maker refunds the escrow instead of setting it ready', async () => {
    const { client, close, t } = await started('maker-quiet');
    await client.createSwap({ ...PARAMS, returnAddress: RETURN });
    await client.sim.pay();
    await t.until(at('recovering'), 20000);
    const done = await t.until(at('refunded'), 20000);
    expect(done.history.map((h) => h.stage)).toContain('recovered');
    expect(done.refund?.reason).toBe('MAKER_REFUNDED');
    // 1 ZEC - premium - request, accept, funding, recovery and refund fees.
    expect(done.refund?.amountZats).toBe(String(100000000 - 200000 - 5 * 10000));
    close();
  }, 30000);

  it('claims through the punish path when the maker disappears', async () => {
    const { client, close, t } = await started('maker-gone');
    await client.createSwap(PARAMS);
    await client.sim.pay();
    const s = await t.until(at('paid_out'), 20000);
    expect(s.claim?.punishPath).toBe(true);
    close();
  }, 30000);

  it('stops at exit_retry when the delivery is refunded, and succeeds on retry', async () => {
    const { client, close, t } = await started('exit-fail');
    await client.createSwap(PARAMS);
    await client.sim.pay();
    const r = await t.until(at('exit_retry'));
    expect(r.lastError?.code).toBe('EXIT_FAILED');
    expect(r.exit?.phase).toBe('refunded');
    expect(r.needs).toBe('exit_decision');
    await client.exit();
    const done = await t.until(at('done'));
    expect(done.exit?.attempts).toBe(2);
    close();
  });

  it('retries by itself when NEAR Intents has no price for a moment', async () => {
    const { client, close, t } = await started('exit-busy');
    await client.createSwap(PARAMS);
    await client.sim.pay();
    const busy = await t.until((s) => s.stage === 'exiting' && s.lastError?.code === 'EXIT_NO_QUOTE');
    expect(busy.lastError?.transient).toBe(true);
    const done = await t.until(at('done'));
    expect(done.lastError).toBeNull();
    close();
  });

  it('asks before delivering below the minimum, and only lets the user lower it', async () => {
    const { client, close, t } = await started('exit-price');
    await client.createSwap(PARAMS);
    await client.sim.pay();
    const r = await t.until(at('exit_retry'));
    expect(r.lastError?.code).toBe('EXIT_BELOW_MINIMUM');
    const offered = r.exit!.offered!;
    expect(BigInt(offered.minAmountOut) < BigInt(PARAMS.minDestinationAmount)).toBe(true);
    await expect(client.exit({ acceptMinimum: '99999999999999' })).rejects.toMatchObject({ code: 'INVALID_PARAMS' });
    await expect(client.exit({ acceptMinimum: 'x' })).rejects.toMatchObject({ code: 'INVALID_PARAMS' });
    await client.exit({ acceptMinimum: offered.minAmountOut });
    const done = await t.until(at('done'));
    expect(done.exit?.approvedMinimum).toBe(offered.minAmountOut);
    close();
  });

  it('resumes after a reload, and restores from a recovery file after the browser is cleared', async () => {
    const store = memoryStore();
    const a = await started(undefined, store);
    const created = await a.client.createSwap(PARAMS);
    const file = await a.client.exportRecovery();
    expect(file.fileName).toBe(`cloakroom-intents-recovery-${created.id.slice(0, 8)}.json`);
    // The format label stays "zecret-recovery" after the rename, so files saved before it restore.
    expect(JSON.parse(file.text)).toMatchObject({ format: 'zecret-recovery', swapId: created.id, network: 'testnet' });
    await a.client.sim.pay();
    await a.t.until(at('request_sent'));
    a.close();

    // Reload: a new engine on the same storage.
    const b = await started(undefined, store);
    const resumed = await b.client.resume();
    expect(resumed?.id).toBe(created.id);
    expect(resumed?.recoveryExportedAt).toBeTypeOf('number');
    await b.t.until(at('escrow_open'));
    await b.client.sim.clearLocal();
    b.close();

    // Cleared browser: nothing to resume until the file is imported.
    const c = await started(undefined, store);
    expect(await c.client.resume()).toBeNull();
    await expect(c.client.importRecovery('{"nope":1}')).rejects.toMatchObject({ code: 'RECOVERY_INVALID' });
    await expect(c.client.importRecovery(file.text.replace('"testnet"', '"mainnet"'))).rejects.toMatchObject({ code: 'RECOVERY_MISMATCH' });
    const restored = await c.client.importRecovery(file.text);
    expect(restored.id).toBe(created.id);
    await c.t.until(at('paid_out'));
    c.close();

    // A fresh browser with no stored chain: the file's snapshot is enough to continue.
    const d = await started(undefined, memoryStore());
    const fromFile = await d.client.importRecovery(file.text);
    expect(fromFile.stage).toBe('awaiting_deposit');
    d.close();
  });

  it('forget is refused once money arrived, and allowed after the end', async () => {
    const { client, close, t } = await started();
    await client.createSwap(PARAMS);
    await client.sim.pay();
    await t.until(at('deposit_seen'));
    await expect(client.forget()).rejects.toMatchObject({ code: 'WRONG_STAGE' });
    await t.until(at('done'));
    await client.forget();
    expect(await client.getState()).toBeNull();
    close();
  });

  it('keeps a finished swap while its leftover ZEC has no return address, unless the user discards it', async () => {
    const { client, close, t } = await started();
    await client.createSwap(PARAMS);
    await client.sim.pay('150000000');
    const done = await t.until(at('done'));
    expect(done.needs).toBe('return_address');
    expect(leftoverPending(done)).toBe(true);
    await expect(client.forget()).rejects.toMatchObject({ code: 'LEFTOVER_PENDING', message: expect.stringContaining('0.4999 ZEC') });
    await expect(client.createSwap(PARAMS)).rejects.toMatchObject({ code: 'LEFTOVER_PENDING' });
    expect((await client.getState())?.stage).toBe('done');
    await client.forget({ discardLeftover: true });
    expect(await client.getState()).toBeNull();
    close();
  });

  it('allows a new swap once the leftover reached the return address', async () => {
    const { client, close, t } = await started();
    await client.createSwap({ ...PARAMS, returnAddress: RETURN });
    await client.sim.pay('150000000');
    const done = await t.until((x) => x.stage === 'done' && !!x.leftover?.returned);
    expect(done.leftover).toMatchObject({ toAddress: RETURN, returned: true });
    expect(leftoverPending(done)).toBe(false);
    await client.forget();
    expect(await client.getState()).toBeNull();
    close();
  });
});

describe('leftoverPending', () => {
  const done = (patch: Partial<SwapState>): SwapState => ({ ...baseState(), stage: 'done', ...patch });
  it('is true at done while the leftover has no address or is not mined', () => {
    expect(leftoverPending(done({ leftover: { amountZats: '5' }, needs: 'return_address' }))).toBe(true);
    expect(leftoverPending(done({ leftover: { amountZats: '5', toAddress: RETURN } }))).toBe(true);
    expect(leftoverPending(done({ leftover: { amountZats: '5', toAddress: RETURN, returned: true } }))).toBe(false);
  });
  it('is false without a leftover, for a leftover below the fee, and before the end', () => {
    expect(leftoverPending(done({}))).toBe(false);
    expect(leftoverPending(done({ leftover: { amountZats: '-5000' } }))).toBe(false);
    expect(leftoverPending({ ...baseState(), stage: 'refunding', needs: 'return_address' })).toBe(false);
    expect(leftoverPending({ ...baseState(), stage: 'refunded' })).toBe(false);
  });
});
