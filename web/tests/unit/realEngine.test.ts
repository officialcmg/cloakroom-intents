// The real engine's worker layer (src/engine/real/realEngine.ts) with a scripted stand-in
// for the WebAssembly module. Security review 3: H1 (a failed write stops the engine until
// the same records are written), H6 (a WebAssembly trap stops the engine), NV3-02 (an
// older tab's write is refused) and H2 (a recovery file's payment address is computed
// again from its wallet key).
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EngineError, type CreateSwapParams, type EngineEvent } from '../../src/engine/api';
import { memoryRecords, StaleWriteError, VERSION_KEY, type RecordStore } from '../../src/engine/real/idb';
import { RealEngine } from '../../src/engine/real/realEngine';
import type { WasmModule, WasmTaker } from '../../src/engine/real/wasm';
import { parseNetworkConfig } from '../../src/lib/netconfig';

const UA =
  'utest13x4neahaknjvdwe5j4h5le2zhfh502nava4jf5g43z9n8ynk9g5whlvsztnn9lm88eq83zzpvsjst0vnfda32lywxv9uz8qgk8wtgr84d6da980fg0wrj2en9hnywv4khl35008f263gztnlqa3zf0he97k0ahttlmmhm7e7ycmn45nm';
const SWAP = 'ab'.repeat(16);
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
  minUsdc: '1',
  minDestinationAmount: '1',
  shown: { usdcExpected: '900000000', destinationExpected: '900000000', premiumZats: '450000', networkFeeZats: '30000' },
};

/** The scripted core: counts its calls; `trapAt` makes the n-th tick trap like a Rust panic. */
class Core {
  ticks = 0;
  trapAt = 0;
  ua = UA;
  takers = 0;
  module(): WasmModule {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const core = this;
    class FakeTaker implements WasmTaker {
      private pending: [string, string][] = [];
      constructor(_cfg: string, records: string) {
        core.takers++;
        void records;
      }
      ua() {
        return core.ua;
      }
      setMaxPremium() {}
      dirty() {
        const d = this.pending;
        this.pending = [];
        return JSON.stringify(d);
      }
      record() {
        return undefined;
      }
      start() {
        this.pending = [
          ['wallet', 'k'],
          [`swap/${SWAP}`, '{"n":0}'],
        ];
        return JSON.stringify({ rfq_id: SWAP, stage: 'WaitingDeposit', zats: 1, near_sk: '00', taker_account: 't', quote: null, log: [] });
      }
      tick(now: number) {
        core.ticks++;
        if (core.trapAt && core.ticks >= core.trapAt) throw new WebAssembly.RuntimeError('unreachable');
        this.pending = [[`swap/${SWAP}`, JSON.stringify({ n: core.ticks })]];
        return JSON.stringify({ next: { kind: 'at', at: now + 1 }, events: [] });
      }
      reconcile() {
        return false;
      }
      swap() {
        return JSON.stringify({ rfq_id: SWAP, stage: 'WaitingDeposit', zats: 1, near_sk: '00', taker_account: 't', quote: null, log: [] });
      }
      facts() {
        return JSON.stringify({ balance: 0, height: 10, fundHeight: null, sweepHeight: null });
      }
      sync() {
        return 10;
      }
      txHeight() {
        return undefined;
      }
      escrowState() {
        return undefined;
      }
      intentsBalance() {
        return '0';
      }
      tokenBalance() {
        return '0';
      }
      returnLeftover() {
        return 'tx';
      }
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
      nearAccount: () => 't',
      exitQuote: () => '',
      exitSubmit: () => '',
      exitStatus: () => '',
      directPrepare: () => '',
      directRelay: () => '',
    } as unknown as WasmModule;
  }
}

/** A record store whose writes fail while `failing` is set. */
function flakyStore(inner: RecordStore = memoryRecords()) {
  const writes: [string, string][][] = [];
  const s = {
    failing: false,
    writes,
    persistent: true,
    all: () => inner.all(),
    putAll: async (entries: [string, string][]) => {
      writes.push(entries);
      if (s.failing) throw new Error('QuotaExceededError');
      return inner.putAll(entries);
    },
    clear: () => inner.clear(),
  };
  return s;
}

async function start(store: RecordStore, core: Core) {
  const engine = new RealEngine(store, '/', async () => core.module());
  const events: EngineEvent[] = [];
  engine.attach((e) => events.push(e));
  await engine.init({ network: 'testnet', directory, real: cfg });
  await engine.resume();
  return { engine, events };
}

const errors = (events: EngineEvent[]) => events.flatMap((e) => (e.type === 'error' ? [e.error] : []));

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('H1: a failed write stops the engine until the same records are stored', () => {
  it('calls the core no more while a write fails, and writes the kept records first when storage works again', async () => {
    const core = new Core();
    const store = flakyStore();
    const { engine, events } = await start(store, core);
    await engine.createSwap(params);
    await vi.advanceTimersByTimeAsync(3_000);
    expect(core.ticks).toBeGreaterThan(0);

    store.failing = true;
    await vi.advanceTimersByTimeAsync(2_000);
    const ticksAtFailure = core.ticks;
    const failedRecord = store.writes.at(-1)!.find(([k]) => k === `swap/${SWAP}`)?.[1];
    expect(failedRecord).toBe(JSON.stringify({ n: ticksAtFailure }));
    const storage = errors(events).filter((e) => e.code === 'STORAGE');
    expect(storage).toHaveLength(1);
    expect(storage[0].transient).toBe(true);

    // A minute of failing writes: the engine retries the write and never ticks.
    const writesBefore = store.writes.length;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(core.ticks).toBe(ticksAtFailure);
    expect(store.writes.length).toBeGreaterThan(writesBefore + 5);
    for (const w of store.writes.slice(writesBefore)) expect(w.find(([k]) => k === `swap/${SWAP}`)?.[1]).toBe(failedRecord);
    // Calls from the page are refused too, before any engine work.
    await expect(engine.exportRecovery()).rejects.toMatchObject({ code: 'STORAGE' });
    expect(errors(events).filter((e) => e.code === 'STORAGE')).toHaveLength(1);

    store.failing = false;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(core.ticks).toBeGreaterThan(ticksAtFailure);
    const firstGood = store.writes.find((w, i) => i >= writesBefore && !store.failing && w.some(([k, v]) => k === `swap/${SWAP}` && v === failedRecord));
    expect(firstGood).toBeDefined();
    expect(Object.fromEntries(await store.all())[`swap/${SWAP}`]).toBe(JSON.stringify({ n: core.ticks }));
    await expect(engine.exportRecovery()).resolves.toMatchObject({ mimeType: 'application/json' });
  });
});

describe('H6: a WebAssembly trap stops the engine', () => {
  it('emits ENGINE_CRASHED once and never calls the trapped instance again', async () => {
    const core = new Core();
    core.trapAt = 3;
    const { engine, events } = await start(memoryRecords(), core);
    await engine.createSwap(params);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(core.ticks).toBe(3);
    const crashed = errors(events).filter((e) => e.code === 'ENGINE_CRASHED');
    expect(crashed).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(core.ticks).toBe(3);
    await expect(engine.setReturnAddress(UA)).rejects.toMatchObject({ code: 'ENGINE_CRASHED' });
  });
});

describe('NV3-02: the version guard', () => {
  it('refuses the write of a store that has not seen the newest version', async () => {
    const shared = new Map<string, string>();
    const a = memoryRecords(shared);
    const b = memoryRecords(shared);
    await a.all();
    await b.all();
    await a.putAll([['k', '1']]);
    await expect(b.putAll([['k', 'old']])).rejects.toBeInstanceOf(StaleWriteError);
    expect(shared.get('k')).toBe('1');
    await expect(b.clear()).rejects.toBeInstanceOf(StaleWriteError);
    expect(shared.get('k')).toBe('1');
    // After reading again, b is current and may write.
    await b.all();
    await b.putAll([['k', '2']]);
    await expect(a.putAll([['k', 'older']])).rejects.toBeInstanceOf(StaleWriteError);
    expect(shared.get('k')).toBe('2');
    expect(Number(shared.get(VERSION_KEY))).toBe(2);
    expect((await a.all()).map(([k]) => k)).toEqual(['k']);
  });

  it('stops the engine of the older tab with OTHER_TAB', async () => {
    const shared = new Map<string, string>();
    const coreA = new Core();
    const first = await start(memoryRecords(shared), coreA);
    await first.engine.createSwap(params);
    await vi.advanceTimersByTimeAsync(2_000);
    // A second tab loads the same swap and runs it.
    const coreB = new Core();
    const second = await start(memoryRecords(shared), coreB);
    await vi.advanceTimersByTimeAsync(3_000);
    expect(coreB.ticks).toBeGreaterThan(0);
    const aTicks = coreA.ticks;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(errors(first.events).map((e) => e.code)).toContain('OTHER_TAB');
    // The first tab's engine made at most one more call and stopped.
    expect(coreA.ticks).toBeLessThanOrEqual(aTicks + 1);
    const stopped = coreA.ticks;
    await vi.advanceTimersByTimeAsync(30_000);
    expect(coreA.ticks).toBe(stopped);
    await expect(first.engine.exportRecovery()).rejects.toBeInstanceOf(EngineError);
    expect(errors(second.events).map((e) => e.code)).not.toContain('OTHER_TAB');
  });
});

describe('H2: a recovery file cannot choose the payment address', () => {
  async function exported() {
    const core = new Core();
    const { engine } = await start(memoryRecords(), core);
    await engine.createSwap(params);
    return JSON.parse((await engine.exportRecovery()).text);
  }

  it('computes the address and the payment link from the wallet key', async () => {
    const file = await exported();
    const planted = 'utest1plantedaddressofsomeoneelse';
    file.web.state.deposit.address = planted;
    file.web.state.deposit.uri = `zcash:${planted}?amount=1`;
    file.web.state.txs = [{ id: 'x', chain: 'zcash', label: 'x', hash: 'h', url: 'javascript:alert(1)', at: 1 }];
    const core = new Core();
    const { engine } = await start(memoryRecords(), core);
    const st = await engine.importRecovery(JSON.stringify(file));
    expect(st.deposit.address).toBe(UA);
    expect(st.deposit.uri).toBe(`zcash:${UA}?amount=1`);
    expect(st.restoredAt).toBeDefined();
    expect(st.txs[0].url).toBeUndefined();
  });

  it('names a new file cloakroom-intents-recovery-<id>.json and keeps the format label, so files saved as zecret-recovery-<id>.json still restore', async () => {
    const core = new Core();
    const { engine } = await start(memoryRecords(), core);
    const created = await engine.createSwap(params);
    const out = await engine.exportRecovery();
    expect(out.fileName).toBe(`cloakroom-intents-recovery-${created.id.slice(0, 8)}.json`);
    const file = JSON.parse(out.text);
    expect(file).toMatchObject({ format: 'zecret-recovery', version: 1, engine: 'wasm' });
    // A file from before the rename has the same content; the restore never reads the name.
    const st = await (await start(memoryRecords(), new Core())).engine.importRecovery(out.text);
    expect(st.id).toBe(created.id);
    const renamed = { ...file, format: 'cloakroom-intents-recovery' };
    await expect((await start(memoryRecords(), new Core())).engine.importRecovery(JSON.stringify(renamed))).rejects.toMatchObject({ code: 'RECOVERY_INVALID' });
  });

  it('checks the addresses in the file as createSwap does', async () => {
    const file = await exported();
    const engineFor = async () => (await start(memoryRecords(), new Core())).engine;
    const badAccount = structuredClone(file);
    badAccount.web.state.params.directExit.nearAccount = 'Not An Account';
    await expect((await engineFor()).importRecovery(JSON.stringify(badAccount))).rejects.toMatchObject({ code: 'INVALID_ADDRESS' });
    const badReturn = structuredClone(file);
    badReturn.web.state.params.returnAddress = 't1abc';
    await expect((await engineFor()).importRecovery(JSON.stringify(badReturn))).rejects.toMatchObject({ code: 'INVALID_ADDRESS' });
    const otherSwap = structuredClone(file);
    otherSwap.web.meta.swapId = 'cd'.repeat(16);
    await expect((await engineFor()).importRecovery(JSON.stringify(otherSwap))).rejects.toMatchObject({ code: 'RECOVERY_INVALID' });
  });
});
