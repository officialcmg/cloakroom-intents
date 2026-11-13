// The taker engine's wallet, in a Web Worker. Every wasm call blocks this worker (network
// calls inside are synchronous XMLHttpRequests), never the page.
import init, * as wasm from './pkg/browser_wallet.js';

const t0 = performance.now();
const ready = init().then(() => ({ instantiate_ms: performance.now() - t0 }));
let wallet = null;

// IndexedDB: one store, one record per wallet key. The spending key never leaves the browser.
function db() {
  return new Promise((res, rej) => {
    const r = indexedDB.open('zecret-spike', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('keys');
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
async function idb(mode, fn) {
  const d = await db();
  return new Promise((res, rej) => {
    const tx = d.transaction('keys', mode);
    const req = fn(tx.objectStore('keys'));
    tx.oncomplete = () => res(req.result);
    tx.onerror = () => rej(tx.error);
  });
}

const cmds = {
  async ready() { return ready; },
  // opts: {backend, url, network, nu7, birthday, restore}
  async create(o) {
    let sk = '';
    if (o.restore) sk = (await idb('readonly', s => s.get('wallet'))) || '';
    if (o.restore && !sk) throw new Error('no stored wallet to restore');
    wallet = new wasm.WebWallet(o.backend, o.url, o.network, o.nu7 ?? undefined, o.birthday != null ? BigInt(o.birthday) : undefined, sk);
    if (!sk && o.store !== false) await idb('readwrite', s => s.put(wallet.sk_hex(), 'wallet'));
    return { address: wallet.address(), restored: !!sk };
  },
  async prepareKey() { return wasm.prepare_proving_key(); },
  async sync() { return wallet.sync(); },
  async balance() { return wallet.balance(); },
  async notes() { return JSON.parse(wallet.notes_json()); },
  async scanInfo() { return JSON.parse(wallet.scan_info()); },
  async skHex() { return wallet.sk_hex(); },
  // memo: array of byte values (up to 512)
  async pay(to, zats, memo) {
    const txid = wallet.pay(to, BigInt(zats), new Uint8Array(memo));
    return { txid, timings: JSON.parse(wallet.last_send_timings()) };
  },
  async lwdProbe(url, count) { return JSON.parse(wasm.lwd_probe(url, BigInt(count))); },
  async paramsBench() { return JSON.parse(wasm.params_bench()); },
};

onmessage = async (e) => {
  const { id, cmd, args } = e.data;
  const t = performance.now();
  try {
    await ready;
    let value = await cmds[cmd](...(args || []));
    if (typeof value === 'bigint') value = Number(value);
    postMessage({ id, ok: true, value, ms: performance.now() - t });
  } catch (err) {
    postMessage({ id, ok: false, error: String(err && err.message || err), ms: performance.now() - t });
  }
};
