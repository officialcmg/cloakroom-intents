// The check for the browser wallet spike. Exits 0 only if every step passes.
//
// 1. A page with the wallet in a Web Worker creates a wallet and shows its address.
// 2. A native `node/` wallet, funded by a coinbase, pays it with a memo; the page reports
//    the amount and the decrypted memo.
// 3. The page builds, proves and broadcasts a fully shielded transaction with a 512-byte
//    memo to a second (native) address.
// 4. The transaction is mined; independent checks: Zebra's own view of the transaction,
//    `memocheck` (orchard trial decryption, not the wallet code) and `cloakroom balance`
//    from `node/`.
// 5. The page is reloaded; the wallet comes back from IndexedDB and finds its change.
//
//   node run.mjs        (see README.md for the build steps and environment variables)
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const ZEBRA = process.env.ZEBRA_RPC || 'http://127.0.0.1:18232';
const PROXY_PORT = 18300, WEB_PORT = 18380;
const CLOAKROOM = process.env.CLOAKROOM_BIN || path.join(REPO, 'node/target/debug/cloakroom');
const SENDMEMO = path.join(HERE, 'target/release/sendmemo');
const MEMOCHECK = path.join(HERE, 'target/release/memocheck');
const NU7 = 2, FUND = 250_000, SEND = 100_000, FEE = 10_000;

const T0 = Date.now();
const secs = (ms) => (ms / 1000).toFixed(1) + ' s';
const procs = [];
let browser;
function step(s) { console.log(`[${((Date.now() - T0) / 1000).toFixed(1).padStart(6)} s] ${s}`); }
function check(cond, what) {
  if (!cond) throw new Error('CHECK FAILED: ' + what);
  step('ok: ' + what);
}

async function zrpc(method, params = []) {
  const r = await fetch(ZEBRA, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error)}`);
  return j.result;
}
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const run = (bin, args) => execFileSync(bin, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();

async function startZebra() {
  if (process.env.ZEBRA_EXTERNAL) { step(`using the running node at ${ZEBRA}`); }
  else {
    const cwd = path.join(REPO, 'harness/zebra'), env = { ...process.env, ZEBRA_CONF: 'zebrad-nu7.toml' };
    execFileSync('docker', ['compose', 'down'], { cwd, env, stdio: 'ignore' });
    execFileSync('docker', ['compose', 'up', '-d'], { cwd, env, stdio: 'ignore' });
  }
  for (let i = 0; i < 90; i++) { try { await zrpc('getblockchaininfo'); break; } catch { await sleep(1000); } }
  await zrpc('generate', [3]);
  const info = await zrpc('getblockchaininfo');
  step(`zebra regtest up: tip ${info.blocks}, next block branch ${info.consensus.nextblock}`);
}

function stopZebra() {
  if (process.env.ZEBRA_EXTERNAL || process.env.KEEP_ZEBRA) return;
  try { execFileSync('docker', ['compose', 'down'], { cwd: path.join(REPO, 'harness/zebra'), stdio: 'ignore' }); } catch {}
}

function startNode(script, args, ready) {
  return new Promise((res, rej) => {
    const p = spawn(process.execPath, [path.join(HERE, script), ...args], { stdio: ['ignore', 'pipe', 'inherit'] });
    procs.push(p);
    p.stdout.on('data', d => { if (String(d).includes(ready)) res(p); });
    p.on('exit', c => rej(new Error(`${script} exited ${c}`)));
  });
}

async function waitMined(txid) {
  for (let i = 0; i < 30; i++) {
    await zrpc('generate', [1]);
    try { const t = await zrpc('getrawtransaction', [txid, 1]); if (t.height) return t; } catch {}
    await sleep(500);
  }
  throw new Error(`${txid} not mined`);
}

function nativeWallet(dir) {
  fs.mkdirSync(dir, { recursive: true });
  // `balance` reads this config; only the Zcash fields are used by it.
  fs.writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ zebra_rpc: ZEBRA, evm_rpc: 'unused', escrow: 'unused', token: 'unused', nu7_height: NU7, lot_zats: 1 }));
  return run(CLOAKROOM, ['init', '--dir', dir]);
}

async function main() {
  for (const f of [CLOAKROOM, SENDMEMO, MEMOCHECK, path.join(HERE, 'web/pkg/browser_wallet_bg.wasm')]) {
    if (!fs.existsSync(f)) throw new Error(`missing ${f}: see README.md for the build steps`);
  }
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'browser-wallet-'));
  await startZebra();
  await startNode('proxy.mjs', [String(PROXY_PORT), ZEBRA], 'CORS proxy');
  await startNode('serve.mjs', [String(WEB_PORT)], 'serving');

  browser = await chromium.launch();
  const page = await browser.newPage();
  page.on('console', m => { if (process.env.VERBOSE || /panic|error/i.test(m.text())) console.log('   browser: ' + m.text()); });
  page.on('pageerror', e => console.log('   pageerror: ' + e));
  page.on('crash', () => console.log('   PAGE CRASHED'));
  // A distinct origin from the proxy, so the JSON-RPC calls are real cross-origin requests.
  await page.goto(`http://127.0.0.1:${WEB_PORT}/`);
  const E = (cmd, ...args) => page.evaluate(([c, a]) => window.engine(c, ...a), [cmd, args]);
  const tBrowser = Date.now();
  const inst = await E('ready');
  step(`wasm module loaded and instantiated in the worker in ${secs(inst.value.instantiate_ms)}`);

  // 1. create
  const created = await E('create', { backend: 'zebra', url: `http://127.0.0.1:${PROXY_PORT}`, network: 'regtest', nu7: NU7, restore: false });
  const ua = created.value.address;
  check(ua.startsWith('uregtest1') && (await page.textContent('#addr')) === ua, `page shows the new shielded address ${ua.slice(0, 24)}...`);

  // the key build starts now, as the real page would start it at load
  const keyBuild = E('prepareKey');

  // 2. a native wallet, funded by a coinbase, pays the browser wallet with a memo
  const funderDir = path.join(work, 'funder'), rcptDir = path.join(work, 'recipient');
  const funderUa = nativeWallet(funderDir), rcptUa = nativeWallet(rcptDir);
  await zrpc('generatetoaddress', [1, funderUa]);
  const memoIn = `Payment to the browser wallet, ${new Date().toISOString()}`;
  const fundTx = run(SENDMEMO, ['--zebra', ZEBRA, '--nu7', String(NU7), '--dir', funderDir, '--to', ua, '--zats', String(FUND), '--memo', memoIn]);
  const fundMined = await waitMined(fundTx);
  step(`native wallet paid ${FUND} zats to the browser wallet: ${fundTx} (height ${fundMined.height})`);

  const kb = await keyBuild;
  const keyMs = kb.value;
  step(`proving key built in the worker: ${secs(keyMs)}`);

  const tScan = Date.now();
  const synced = await E('sync');
  const scanMs = Date.now() - tScan;
  const bal = (await E('balance')).value;
  const notes = (await E('notes')).value;
  step(`scan of blocks 1..${synced.value}: ${secs(scanMs)}`);
  check(bal === FUND, `browser wallet balance is ${bal} zats`);
  check(notes.length === 1 && notes[0].memo === memoIn && notes[0].txid === fundTx, `browser decrypted the memo: "${notes[0] && notes[0].memo}"`);
  check((await page.textContent('#memo')).includes(memoIn), 'page shows the amount and the memo');

  // 3. fully shielded send with a 512-byte memo
  const filler = ' The taker engine sends this memo from a Web Worker.';
  let memoOut = `zecret browser spike ${Date.now()}.`;
  while (memoOut.length < 512) memoOut += filler;
  memoOut = memoOut.slice(0, 512);
  const memoBytes = Array.from(new TextEncoder().encode(memoOut));
  check(memoBytes.length === 512, 'outgoing memo is 512 bytes');
  const tPay = Date.now();
  const paid = await E('pay', rcptUa, SEND, memoBytes);
  const payMs = Date.now() - tPay;
  const { txid, timings } = paid.value;
  step(`browser built, proved, signed and broadcast ${txid} in ${secs(payMs)}`);
  step(`  send timings (ms): ${JSON.stringify(Object.fromEntries(Object.entries(timings).map(([k, v]) => [k, Math.round(v)])))}`);

  // 4. mined, and independent checks
  const mined = await waitMined(txid);
  check(mined.height > fundMined.height, `transaction mined at height ${mined.height}`);
  check(mined.vin.length === 0 && mined.vout.length === 0 && mined.vShieldedSpend.length === 0 && mined.vShieldedOutput.length === 0
    && mined.orchard.actions.length === 0 && mined.ironwood.actions.length === 2,
    'Zebra: no transparent inputs or outputs, no Sapling, no Orchard, 2 Ironwood actions');
  const memoHex = Buffer.from(memoBytes).toString('hex');
  const audit = run(MEMOCHECK, ['--zebra', ZEBRA, '--nu7', String(NU7), '--txid', txid, '--dir', rcptDir, '--zats', String(SEND), '--memo-hex', memoHex]);
  for (const l of audit.split('\n')) console.log('   memocheck: ' + l);
  check(audit.includes('CHECK OK'), 'memocheck: fully shielded, recipient decrypts the value and the exact 512-byte memo');
  const rb = JSON.parse(run(CLOAKROOM, ['balance', '--dir', rcptDir]));
  check(rb.zats === SEND, `node/ client (cloakroom balance) sees ${rb.zats} zats at the recipient`);

  await E('sync');
  const change = (await E('balance')).value;
  check(change === FUND - SEND - FEE, `browser wallet sees its change: ${change} zats`);

  // 5. reload: the key comes back from IndexedDB, the scan finds the same state
  await page.reload();
  await E('ready');
  const restored = await E('create', { backend: 'zebra', url: `http://127.0.0.1:${PROXY_PORT}`, network: 'regtest', nu7: NU7, restore: true });
  await E('sync');
  const after = (await E('balance')).value;
  check(restored.value.restored && restored.value.address === ua && after === change, 'after a page reload the wallet is restored from IndexedDB with the same balance');
  const browserMs = Date.now() - tBrowser;

  console.log('\nTIMINGS (headless Chromium, one worker thread)');
  console.log(`  key build      ${secs(keyMs)}`);
  console.log(`  scan           ${secs(scanMs)}  (blocks 1..${synced.value}, regtest)`);
  console.log(`  proof          ${secs(timings.prove_ms)}`);
  console.log(`  send total     ${secs(payMs)}  (sync, tree, build, proof, sign, verify, broadcast)`);
  console.log(`  total          ${secs(browserMs)}  (page loaded to all checks done, includes native funding and mining)`);
  console.log(`  script total   ${secs(Date.now() - T0)}`);
  console.log('\nPASS');
}

let code = 0;
try { await main(); } catch (e) { console.log(String(e && e.stack || e)); console.log('\nFAIL'); code = 1; }
if (browser) await browser.close();
for (const p of procs) p.kill();
stopZebra();
process.exit(code);
