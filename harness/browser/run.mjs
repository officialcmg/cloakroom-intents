#!/usr/bin/env node
// Browser scenarios: the maker is the native `zecret maker` process, the taker runs ONLY in
// headless Chromium through the real website (web/dist) with the WebAssembly engine, on
// Zebra regtest and a NEAR sandbox (real escrow, live wrap.near and intents.near code).
//
//   source .tools/env.sh
//   node harness/browser/run.mjs              # scenarios a b c d e
//   node harness/browser/run.mjs a e          # selected scenarios
//   node harness/browser/run.mjs --serve      # chains, maker and site only, for a person
//
// Environment: SKIP_BUILD=1 skips the wasm, site and cargo builds. LIVE_ONECLICK=0 mocks
// the 1Click preview instead of asking the live API through HTTPS_PROXY.
//
// Evidence: harness/evidence/browser/<scenario>/result.json and harness/BROWSER_RESULTS.md.
import { spawn, spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '../..');
const WEB = path.join(ROOT, 'web');
const WORK = path.join(ROOT, 'harness/work/browser');
const EVID = path.join(ROOT, 'harness/evidence/browser');
const require = createRequire(path.join(WEB, 'package.json'));
const { chromium } = require('playwright');

const PROXY_PORT = 18330; // CORS proxy in front of Zebra
const SITE_PORT = 18380; // the website (web/dist) with the headers of web/vercel.json
const RELAY_PORT = 3410; // the maker's relay for the direct exit
const SITE = `http://127.0.0.1:${SITE_PORT}`;

const ONE_NEAR = 10n ** 24n;
const PRICE_PER_ZEC = 30n * ONE_NEAR; // the maker pays 30 wNEAR per ZEC (wNEAR stands in for USDC)
const PREMIUM = 50_000;
const FEE = 10_000;
const PAY_ZEC = '1.0005'; // what the user pays into the temporary wallet
const PAY_ZATS = 100_050_000;
const SOLD = PAY_ZATS - 40_000 - PREMIUM; // the core's zats: pay - required_deposit(0, premium)
const PRICE = (BigInt(SOLD) * PRICE_PER_ZEC) / 100_000_000n;
const STIPEND = (3n * ONE_NEAR) / 100n;
const TIMING = { inclusion_secs: 3, finality_secs: 3, reorg_budget: 2, margin_secs: 0, fund_secs: 30, k_z: 2 };
// Longer than harness/run.py: a browser proof takes about 30 s, more on a busy machine.
const WINDOWS = { t0_secs: 300, t1_secs: 150, t2_secs: 150, reveal_secs: 60 };

const t0 = Date.now();
const log = (m) => console.log(`[browser ${new Date().toISOString().slice(11, 19)} +${Math.round((Date.now() - t0) / 1000)}s] ${m}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- chains

let chains; // from harness/browser_chains.py
let chainsProc;

async function startChains() {
  chainsProc = spawn('python3', ['-I', path.join(ROOT, 'harness/browser_chains.py')], { stdio: ['pipe', 'pipe', 'inherit'], env: process.env });
  const rl = readline.createInterface({ input: chainsProc.stdout });
  const queue = [];
  const waiting = [];
  chainsLines = { next: () => (queue.length ? Promise.resolve(queue.shift()) : new Promise((r) => waiting.push(r))) };
  chains = await new Promise((resolve, reject) => {
    let first = true;
    rl.on('line', (l) => {
      if (!l.startsWith('{')) return console.log(l);
      if (first) {
        first = false;
        return resolve(JSON.parse(l));
      }
      const w = waiting.shift();
      if (w) w(l);
      else queue.push(l);
    });
    chainsProc.on('exit', (c) => reject(new Error(`chains exited ${c}`)));
  });
  log(`chains up: zebra ${chains.zebra}, near ${chains.near}, escrow ${chains.escrow} (code ${chains.escrow_code_hash}), token ${chains.token}, verifier ${chains.intents}`);
}

async function post(url, body) {
  const r = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  return r.json();
}
async function zrpc(method, params = []) {
  const r = await post(chains.zebra, { jsonrpc: '2.0', id: 1, method, params });
  if (r.error) throw new Error(`${method}: ${JSON.stringify(r.error)}`);
  return r.result;
}
async function nrpc(method, params) {
  const r = await post(chains.near, { jsonrpc: '2.0', id: 'b', method, params });
  if (r.error) throw new Error(`NEAR ${method}: ${JSON.stringify(r.error)}`);
  return r.result;
}
async function view(contract, method, args) {
  const r = await nrpc('query', { request_type: 'call_function', finality: 'final', account_id: contract, method_name: method, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') });
  return r.result.length ? JSON.parse(Buffer.from(r.result).toString()) : null;
}
const wnear = async (a) => BigInt((await view(chains.token, 'ft_balance_of', { account_id: a })) ?? '0');
/** The balance once it reaches `want` (final blocks trail execution), or the last value read after 30 s. */
async function wnearAtLeast(a, want) {
  let v = 0n;
  for (let i = 0; i < 60; i++) {
    v = await wnear(a);
    if (v >= want) break;
    await sleep(500);
  }
  return v;
}
const intentsBal = async (a) => BigInt(await view(chains.intents, 'mt_balance_of', { account_id: a, token_id: `nep141:${chains.token}` }));
const escrowSwap = (id) => view(chains.escrow, 'get_swap', { id });
async function nearBlock(finality = 'final') {
  const h = (await nrpc('block', { finality })).header;
  return { height: h.height, ts: Math.floor(h.timestamp / 1e9) };
}
async function warpTo(ts) {
  const start = (await nearBlock()).ts;
  for (;;) {
    const b = await nearBlock();
    if (b.ts >= ts) break;
    const delta = Math.max(1, Math.min(200, Math.floor((ts - b.ts) / 2)));
    const head = (await nearBlock('optimistic')).height;
    await nrpc('sandbox_fast_forward', { delta_height: delta });
    while ((await nearBlock()).height < head + delta) await sleep(200);
  }
  log(`warped NEAR time forward ${(await nearBlock()).ts - start}s`);
}

function zecret(...args) {
  const r = spawnSync(chains.bin, args, { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`zecret ${args.slice(0, 2).join(' ')}: ${r.stderr || r.stdout}`);
  return r.stdout.trim();
}

let chainsLines; // answers of browser_chains.py to its commands
/** Mines one block paying its coinbase to `ua` (through the miner's lock), then waits until the wallet in `dir` sees it. */
async function coinbase(ua) {
  chainsProc.stdin.write(`fund ${ua}\n`);
  const r = JSON.parse(await chainsLines.next());
  if (!r.ok) throw new Error(`fund: ${r.error}`);
  return r.blocks;
}

/** Waits until the mempool is empty and one more block is mined. */
async function settle() {
  for (let i = 0; i < 120 && (await zrpc('getrawmempool')).length; i++) await sleep(500);
  const h = await zrpc('getblockcount');
  while ((await zrpc('getblockcount')) <= h) await sleep(300);
}

function chainsCfg() {
  return { zebra_rpc: chains.zebra, nu7_height: 2, near_rpc: chains.near, escrow: chains.escrow, token: chains.token, intents: chains.intents };
}

/** A native wallet (`zecret` with its own folder). */
class Wallet {
  constructor(dir) {
    this.dir = dir;
    fs.rmSync(dir, { recursive: true, force: true });
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ chains: chainsCfg() }));
    this.ua = zecret('init', '--dir', dir);
  }
  balance() {
    return JSON.parse(zecret('balance', '--dir', this.dir)).zats;
  }
  pay(to, zats) {
    return zecret('pay', '--dir', this.dir, '--to', to, '--zats', String(zats));
  }
}

class Maker {
  constructor(scen, extra = {}) {
    this.dir = path.join(WORK, scen, 'maker');
    fs.rmSync(this.dir, { recursive: true, force: true });
    fs.mkdirSync(this.dir, { recursive: true });
    const cfg = {
      chains: chainsCfg(),
      timing: TIMING,
      near_account: chains.maker,
      price_per_zec: PRICE_PER_ZEC.toString(),
      min_zats: 1000,
      max_zats: 500_000_000,
      premium_zats: PREMIUM,
      stipend: STIPEND.toString(),
      windows: WINDOWS,
      ...extra,
    };
    fs.writeFileSync(path.join(this.dir, 'config.json'), JSON.stringify(cfg, null, 2));
    this.ua = zecret('init', '--dir', this.dir);
    fs.copyFileSync(chains.maker_key, path.join(this.dir, 'near_key.json'));
    this.entry = JSON.parse(zecret('directory', '--dir', this.dir));
    this.logPath = path.join(EVID, scen, 'maker.log');
  }
  start(...args) {
    const out = fs.openSync(this.logPath, 'a');
    this.proc = spawn(chains.bin, ['maker', '--dir', this.dir, '--relay', `127.0.0.1:${RELAY_PORT}`, ...args], { stdio: ['ignore', out, out] });
  }
  async stop() {
    if (this.proc && this.proc.exitCode === null) {
      this.proc.kill();
      await new Promise((r) => this.proc.once('exit', r));
    }
  }
  swap() {
    const d = path.join(this.dir, 'swap');
    if (!fs.existsSync(d)) return null;
    const f = fs.readdirSync(d).filter((x) => x.endsWith('.json')).sort()[0];
    return f ? JSON.parse(fs.readFileSync(path.join(d, f), 'utf8')) : null;
  }
  logText() {
    return fs.existsSync(this.logPath) ? fs.readFileSync(this.logPath, 'utf8') : '';
  }
}

// ---------------------------------------------------------------- servers

function corsProxy(port, target) {
  const cors = { 'access-control-allow-origin': '*', 'access-control-allow-methods': 'POST, OPTIONS', 'access-control-allow-headers': 'content-type', 'access-control-max-age': '600' };
  return http
    .createServer((req, res) => {
      if (req.method === 'OPTIONS') return res.writeHead(204, cors).end();
      if (req.method !== 'POST') return res.writeHead(405, cors).end();
      const chunks = [];
      req.on('data', (c) => chunks.push(c));
      req.on('end', async () => {
        try {
          const r = await fetch(target, { method: 'POST', headers: { 'content-type': 'application/json' }, body: Buffer.concat(chunks) });
          res.writeHead(r.status, { ...cors, 'content-type': r.headers.get('content-type') || 'application/json' });
          res.end(Buffer.from(await r.arrayBuffer()));
        } catch (e) {
          res.writeHead(502, cors).end(JSON.stringify({ error: String(e) }));
        }
      });
    })
    .listen(port, '127.0.0.1');
}

/** The headers web/vercel.json sets for `p`, with the local endpoints added to connect-src. */
function vercelHeaders(p, extraConnect) {
  const v = JSON.parse(fs.readFileSync(path.join(WEB, 'vercel.json'), 'utf8'));
  const out = {};
  for (const rule of v.headers) {
    const re = new RegExp(`^${rule.source.replace(/\(\.\*\)/g, '.*')}$`);
    if (!re.test(p)) continue;
    for (const h of rule.headers) out[h.key.toLowerCase()] = h.value;
  }
  if (out['content-security-policy']) out['content-security-policy'] = out['content-security-policy'].replace('connect-src', `connect-src ${extraConnect.join(' ')}`);
  return out;
}

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.png': 'image/png' };

function siteServer(port, extraConnect) {
  const dist = path.join(WEB, 'dist');
  return http
    .createServer((req, res) => {
      const p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
      let f;
      // `?config=local` in --serve mode: the configuration written for this run.
      if (p === '/config/local.json' && fs.existsSync(path.join(WORK, 'run/local/config.json'))) f = path.join(WORK, 'run/local/config.json');
      else if (p.startsWith('/run/')) f = path.join(WORK, 'run', p.slice(5));
      else f = path.join(dist, p === '/' ? 'index.html' : p);
      if (!f.startsWith(dist) && !f.startsWith(path.join(WORK, 'run'))) return res.writeHead(403).end();
      if (!fs.existsSync(f) || fs.statSync(f).isDirectory()) f = path.join(dist, 'index.html');
      res.writeHead(200, { 'content-type': TYPES[path.extname(f)] || 'application/octet-stream', 'cache-control': 'no-store', ...vercelHeaders(p, extraConnect) });
      fs.createReadStream(f).pipe(res);
    })
    .listen(port, '127.0.0.1');
}

// ---------------------------------------------------------------- per scenario setup

function writeRun(scen, maker, rate) {
  const dir = path.join(WORK, 'run', scen);
  fs.mkdirSync(dir, { recursive: true });
  const e = maker.entry;
  const directory = {
    format: 'zecret-maker-directory',
    version: 1,
    network: 'regtest',
    updatedAt: new Date().toISOString(),
    status: 'online',
    maker: { name: 'Local test maker', zcashAddress: e.ua, requestKey: e.rfq_pub, nearAccount: e.near_account, escrowContract: chains.escrow, nearPublicKey: e.near_pk, epoch: e.epoch },
    rate: { usdcPerZec: rate, validForSeconds: 300 },
    limits: { minZec: '0.01', maxZec: '5' },
    premium: { bps: 0, minZec: '0.0005' },
    zcashFeePerSendZec: '0.0001',
    signature: e.sig,
  };
  const config = {
    format: 'zecret-config',
    version: 1,
    network: 'regtest',
    label: 'Local test network: Zebra regtest and a NEAR sandbox',
    directoryUrl: `/run/${scen}/directory.json`,
    zcash: { zebraRpc: `http://127.0.0.1:${PROXY_PORT}`, nu7Height: 2 },
    near: { rpc: chains.near, escrow: chains.escrow, escrowCodeHash: chains.escrow_code_hash, token: chains.token, tokenSymbol: 'wNEAR', tokenDecimals: 24, verifier: chains.intents },
    timing: TIMING,
    minStipend: STIPEND.toString(),
    quoteTimeoutSecs: 400,
    exit: { mode: 'direct', relayUrl: `http://127.0.0.1:${RELAY_PORT}/execute-intents` },
    honesty: [
      'wNEAR stands in for USDC.',
      'The Verifier is a copy of the live intents.near code in the sandbox; there is no 1Click here, so the tokens go to your NEAR account and the price for the chosen chain is a mainnet preview.',
    ],
  };
  fs.writeFileSync(path.join(dir, 'directory.json'), JSON.stringify(directory, null, 2));
  fs.writeFileSync(path.join(dir, 'config.json'), JSON.stringify(config, null, 2));
  return `${SITE}/?config=/run/${scen}/config.json`;
}

function receiver(name) {
  const bin = path.join(ROOT, 'crates/target/debug/examples/sandbox_account');
  const r = spawnSync(bin, [chains.near, chains.root_key, name, chains.token], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`sandbox_account: ${r.stderr}`);
  return r.stdout.trim();
}

// ---------------------------------------------------------------- the browser

let browser;
const LIVE = process.env.LIVE_ONECLICK !== '0';

async function newPage(scen, label) {
  const proxy = LIVE && process.env.HTTPS_PROXY ? { server: process.env.HTTPS_PROXY, bypass: '127.0.0.1,localhost' } : undefined;
  const ctx = await browser.newContext({ acceptDownloads: true, viewport: { width: 1280, height: 900 }, ignoreHTTPSErrors: true, proxy });
  const page = await ctx.newPage();
  const consoleLog = fs.createWriteStream(path.join(EVID, scen, `console-${label}.log`), { flags: 'a' });
  const timings = [];
  page.on('console', (m) => {
    const t = m.text();
    consoleLog.write(`${new Date().toISOString()} ${t}\n`);
    const tm = /^\[zecret-timing\] (\S+) (\d+)/.exec(t);
    if (tm) timings.push({ what: tm[1], ms: Number(tm[2]), at: Date.now() });
    if (/Content Security Policy|Refused to/.test(t)) log(`CSP: ${t}`);
  });
  page.on('pageerror', (e) => consoleLog.write(`PAGEERROR ${e.message}\n`));
  page.on('dialog', (d) => d.accept());
  if (!LIVE) await mockOneClick(page);
  return { ctx, page, timings };
}

/** A minimal stand-in for 1Click's token list and dry quotes (LIVE_ONECLICK=0). */
async function mockOneClick(page) {
  const USDC_BASE = { assetId: 'nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near', decimals: 6, blockchain: 'base', symbol: 'USDC', price: 1 };
  await page.route('https://1click.chaindefuser.com/**', async (route) => {
    const req = route.request();
    const json = (status, body) => route.fulfill({ status, contentType: 'application/json', headers: { 'access-control-allow-origin': '*' }, body: JSON.stringify(body) });
    if (req.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': 'content-type' } });
    const u = new URL(req.url());
    if (u.pathname === '/v0/tokens') return json(200, [USDC_BASE, { assetId: 'nep141:zec.omft.near', decimals: 8, blockchain: 'zec', symbol: 'ZEC', price: 40 }]);
    const b = req.postDataJSON();
    const out = (BigInt(b.amount) * 995n) / 1000n;
    return json(201, { quote: { amountIn: b.amount, amountOut: out.toString(), minAmountOut: ((out * 99n) / 100n).toString(), timeEstimate: 12 }, quoteRequest: b, signature: 'ed25519:x', timestamp: new Date().toISOString() });
  });
}

/** The engine's record of the swap, read from the page's IndexedDB (the website's state). */
async function webState(page) {
  return page.evaluate(async () => {
    const dbs = await indexedDB.databases();
    if (!dbs.some((d) => d.name === 'zecret-engine')) return null;
    return new Promise((res) => {
      const r = indexedDB.open('zecret-engine');
      r.onsuccess = () => {
        const db = r.result;
        if (!db.objectStoreNames.contains('records')) return res(null);
        const g = db.transaction('records').objectStore('records').get('web/state');
        g.onsuccess = () => res(g.result ? JSON.parse(g.result) : null);
        g.onerror = () => res(null);
      };
      r.onerror = () => res(null);
    });
  });
}

/** Polls the stored state until `pred(state)`; returns it. */
async function waitState(page, pred, timeoutMs, what) {
  const end = Date.now() + timeoutMs;
  let last;
  while (Date.now() < end) {
    try {
      last = await webState(page);
    } catch {
      last = undefined; // the page is reloading
    }
    if (last && pred(last)) return last;
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${what} (stage ${last?.stage}, error ${JSON.stringify(last?.lastError)})`);
}

/** Fills the form, saves the recovery file, returns {deposit, file}. */
async function startSwap(page, url, { nearAccount, returnAddress, scen }) {
  const t = Date.now();
  await page.goto(url);
  await page.getByLabel('You pay').fill(PAY_ZEC, { timeout: 120_000 });
  await page.getByTestId('near-account').fill(nearAccount);
  if (returnAddress) {
    await page.getByRole('button', { name: /Add a Zcash return address/ }).click();
    await page.getByLabel(/Zcash return address/).fill(returnAddress);
  }
  await page.getByTestId('price-line').getByText('You receive at least').waitFor({ timeout: 90_000 });
  const preview = `${await page.getByTestId('preview-note').innerText()} | ${await page.getByTestId('price-line').innerText()}`;
  // createSwap may wait behind the proving-key build (about 20 s) in the worker.
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await page.getByRole('heading', { name: 'Save your recovery file' }).waitFor({ timeout: 120_000 });
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Save recovery file' }).click();
  const dl = await download;
  const file = path.join(EVID, scen, 'recovery.json');
  await dl.saveAs(file);
  await page.getByRole('button', { name: 'Continue to payment' }).click();
  const st = await waitState(page, (s) => s.stage === 'awaiting_deposit', 30_000, 'the payment screen');
  return { deposit: st.deposit, file, preview, formMs: Date.now() - t };
}

// ---------------------------------------------------------------- checks

class Checks {
  constructor(scen, title) {
    Object.assign(this, { scen, title, items: [], facts: {} });
  }
  ok(desc, cond, detail = '') {
    this.items.push({ check: desc, pass: !!cond, detail: String(detail) });
    log(`  ${cond ? 'PASS' : 'FAIL'}  ${desc} ${detail}`);
  }
  save() {
    fs.writeFileSync(path.join(EVID, this.scen, 'result.json'), JSON.stringify({ scenario: this.scen, title: this.title, checks: this.items, facts: this.facts }, null, 2));
    return this.items.every((i) => i.pass);
  }
}

async function privacyAudit(c, txids) {
  const bad = [];
  const seen = [];
  for (const txid of [...new Set(txids.filter(Boolean))]) {
    let v;
    try {
      v = await zrpc('getrawtransaction', [txid, 1]);
    } catch {
      continue;
    }
    seen.push(txid);
    const iw = v.ironwood || {};
    const shielded = !(v.vin || []).length && !(v.vout || []).length && !((v.orchard || {}).actions || []).length && !(v.vShieldedSpend || []).length && !(v.vShieldedOutput || []).length;
    if (!shielded || !(iw.actions || []).length) bad.push(txid);
  }
  c.facts.zcash_txs = seen;
  c.ok(`privacy audit: all ${seen.length} Zcash transactions (user payment, browser taker, maker) are fully shielded Ironwood, no transparent or Sapling parts`, seen.length > 0 && !bad.length, bad.join(','));
}

function stageTimings(st, timings) {
  const h = st.history;
  const rows = h.map((x, i) => ({ stage: x.stage, at: new Date(x.at).toISOString(), secs: i + 1 < h.length ? Math.round((h[i + 1].at - x.at) / 100) / 10 : null }));
  return { stages: rows, engine: timings.map((t) => ({ what: t.what, secs: Math.round(t.ms / 100) / 10 })), total_secs: Math.round((h[h.length - 1].at - h[0].at) / 1000) };
}

function zcashTxids(st, maker) {
  const m = maker.swap() || {};
  return [...st.txs.filter((t) => t.chain === 'zcash').map((t) => t.hash), m.quote_txid, m.sweep_txid];
}

// ---------------------------------------------------------------- scenarios

async function scenarioA() {
  const scen = 'a_full_swap';
  const c = new Checks(scen, 'Full swap through the page: amount, token and chain, NEAR account, recovery file, payment by QR address from a native wallet, progress to done; the taker key holds the price in the Verifier, then the direct exit delivers it to the NEAR account');
  const maker = new Maker(scen);
  maker.start();
  const url = writeRun(scen, maker, '30');
  const user = new Wallet(path.join(WORK, scen, 'user'));
  await coinbase(user.ua);
  await coinbase(maker.ua);
  const recv = receiver('alicea');
  await settle();
  if (user.balance() <= 0) throw new Error('the user wallet did not receive its coinbase');
  const makerBefore = await wnear(chains.maker);
  const { page, ctx, timings } = await newPage(scen, 'main');
  const { deposit, preview } = await startSwap(page, url, { nearAccount: recv, returnAddress: user.ua, scen });
  c.facts.preview = preview;
  c.ok(`the page showed the ${LIVE ? 'live' : 'mocked'} mainnet 1Click price as a labelled preview`, /Mainnet preview/.test(preview) && /You receive at least/.test(preview), preview);
  c.ok('the deposit QR address is the browser wallet and asks for the amount', deposit.address.startsWith('uregtest1') && deposit.uri.includes(`amount=${PAY_ZEC}`), deposit.uri.slice(0, 40));
  const payTx = user.pay(deposit.address, PAY_ZATS);
  log(`user paid ${PAY_ZATS} zats in ${payTx}`);
  // Watch the taker's Verifier balance from the moment it is known.
  let maxIntents = 0n;
  let acct;
  const watcher = setInterval(async () => {
    try {
      const st = await webState(page);
      acct ??= st?.escrow?.takerAccount;
      if (acct) {
        const b = await intentsBal(acct);
        if (b > maxIntents) maxIntents = b;
      }
    } catch {
      /* page busy */
    }
  }, 300);
  const st = await waitState(page, (s) => s.stage === 'done' || s.stage === 'exit_retry' || s.stage === 'refunding', 20 * 60_000, 'done');
  clearInterval(watcher);
  await page.getByTestId('done-title').waitFor({ timeout: 30_000 }).catch(() => undefined);
  const title = await page.getByTestId('done-title').innerText().catch(() => '');
  await page.screenshot({ path: path.join(EVID, scen, 'done.png'), fullPage: true });
  await maker.stop();
  c.facts.timings = stageTimings(st, timings);
  c.facts.taker_account = acct;
  c.ok('the page reached done', st.stage === 'done', `${st.stage} ${JSON.stringify(st.lastError ?? '')}`);
  c.ok('the done screen names the delivery', title.includes('wNEAR') && (await page.getByText(recv).count()) > 0, title);
  c.ok('the taker key held exactly the price in the Verifier (mt_balance_of)', maxIntents === PRICE, `${maxIntents} vs ${PRICE}`);
  c.ok('the engine recorded the payout of the price (micro-units)', BigInt(st.payout?.usdc ?? 0) === PRICE / 10n ** 18n, st.payout?.usdc);
  c.ok('the exit delivered the whole price to the NEAR account', (await wnearAtLeast(recv, PRICE)) === PRICE, await wnear(recv));
  c.ok('nothing is left under the taker key in the Verifier', acct && (await intentsBal(acct)) === 0n);
  c.ok('the maker paid exactly the price', makerBefore - (await wnear(chains.maker)) === PRICE);
  const e = await escrowSwap(st.escrow.swapId);
  c.ok('escrow Claimed at a final block', e?.state === 'Claimed', e?.state);
  c.ok('the browser sent commit_claim and claim itself', st.txs.some((t) => t.id === 'commit') && st.txs.some((t) => t.id === 'claim'));
  c.ok('stage history has the synthesized stages (deposit_seen, funding_confirming, ready)', ['deposit_seen', 'funding_confirming', 'ready'].every((x) => st.history.some((h) => h.stage === x)), st.history.map((h) => h.stage).join(','));
  await privacyAudit(c, [payTx, ...zcashTxids(st, maker)]);
  await ctx.close();
  return c.save();
}

async function scenarioB() {
  const scen = 'b_maker_quiet_refund';
  const c = new Checks(scen, 'The maker goes quiet after the funding (never sets Ready) and refunds after t1: the browser reads b, takes the ZEC back from the shared address and returns it to the return address');
  const maker = new Maker(scen, { misbehave: 'never-ready' });
  maker.start();
  const url = writeRun(scen, maker, '30');
  const user = new Wallet(path.join(WORK, scen, 'user'));
  await coinbase(user.ua);
  await coinbase(maker.ua);
  const recv = receiver('aliceb');
  await settle();
  if (user.balance() <= 0) throw new Error('the user wallet did not receive its coinbase');
  const userBefore = user.balance();
  const makerBefore = await wnear(chains.maker);
  const { page, ctx, timings } = await newPage(scen, 'main');
  const { deposit } = await startSwap(page, url, { nearAccount: recv, returnAddress: user.ua, scen });
  const payTx = user.pay(deposit.address, PAY_ZATS);
  await waitState(page, (s) => s.stage === 'funding_confirming' && s.funding?.confirmations >= 2, 15 * 60_000, 'the funding confirmations');
  let m;
  for (let i = 0; i < 60 && !(m = maker.swap())?.t1; i++) await sleep(500);
  await sleep(4_000);
  await warpTo(m.t1 + 1);
  const st = await waitState(page, (s) => s.stage === 'refunded', 15 * 60_000, 'refunded');
  await page.getByTestId('refunded-title').waitFor({ timeout: 30_000 });
  await page.screenshot({ path: path.join(EVID, scen, 'refunded.png'), fullPage: true });
  await maker.stop();
  await settle();
  const userAfter = user.balance();
  c.facts.timings = stageTimings(st, timings);
  c.ok('the page went recovering -> recovered -> refunding -> refunded', ['recovering', 'recovered', 'refunding', 'refunded'].every((x) => st.history.some((h) => h.stage === x)), st.history.map((h) => h.stage).join(','));
  c.ok('refund reason MAKER_REFUNDED', st.refund?.reason === 'MAKER_REFUNDED');
  const e = await escrowSwap(st.escrow.swapId);
  c.ok('escrow Refunded, a never published', e?.state === 'Refunded' && !e.a, e?.state);
  c.ok('the maker got its tokens back', (await wnear(chains.maker)) === makerBefore);
  const expectRefund = PAY_ZATS - 5 * FEE - PREMIUM;
  c.ok('the refund sent everything left (pay - premium - 5 fees) to the return address', Number(st.refund?.amountZats) === expectRefund, `${st.refund?.amountZats} vs ${expectRefund}`);
  c.ok('the user wallet lost exactly its payment fee + premium + 5 swap fees', userBefore - userAfter === FEE + PREMIUM + 5 * FEE, userBefore - userAfter);
  await privacyAudit(c, [payTx, ...zcashTxids(st, maker)]);
  await ctx.close();
  return c.save();
}

async function scenarioC() {
  const scen = 'c_reload_twice';
  const c = new Checks(scen, 'The page is reloaded twice mid-swap: before the funding (at the acceptance) and between commit and claim; each time the swap resumes from IndexedDB and completes');
  const maker = new Maker(scen);
  maker.start();
  const url = writeRun(scen, maker, '30');
  const user = new Wallet(path.join(WORK, scen, 'user'));
  await coinbase(user.ua);
  await coinbase(maker.ua);
  const recv = receiver('alicec');
  await settle();
  if (user.balance() <= 0) throw new Error('the user wallet did not receive its coinbase');
  const { page, ctx, timings } = await newPage(scen, 'main');
  const { deposit } = await startSwap(page, url, { nearAccount: recv, returnAddress: user.ua, scen });
  const payTx = user.pay(deposit.address, PAY_ZATS);
  const before = await waitState(page, (s) => ['accepted', 'quote_received', 'escrow_open'].includes(s.stage), 15 * 60_000, 'the acceptance');
  c.facts.reload1_stage = before.stage;
  await page.reload();
  log(`reload 1 at ${before.stage}`);
  const mid = await waitState(page, (s) => ['committing', 'claiming'].includes(s.stage), 15 * 60_000, 'the commit');
  c.facts.reload2_stage = mid.stage;
  await page.reload();
  log(`reload 2 at ${mid.stage}`);
  const st = await waitState(page, (s) => s.stage === 'done' || s.stage === 'exit_retry' || s.stage === 'refunding', 15 * 60_000, 'done');
  await maker.stop();
  c.facts.timings = stageTimings(st, timings);
  c.ok('first reload happened before the funding', ['accepted', 'quote_received', 'escrow_open'].includes(before.stage) && !before.txs.some((t) => t.id === 'funding'), before.stage);
  c.ok('second reload happened between commit and claim', ['committing', 'claiming'].includes(mid.stage) && !mid.history.some((h) => h.stage === 'claimed'), mid.stage);
  c.ok('the swap completed after the reloads', st.stage === 'done', `${st.stage} ${JSON.stringify(st.lastError ?? '')}`);
  c.ok('the NEAR account received the price', (await wnearAtLeast(recv, PRICE)) === PRICE, await wnear(recv));
  const e = await escrowSwap(st.escrow.swapId);
  c.ok('escrow Claimed', e?.state === 'Claimed', e?.state);
  await privacyAudit(c, [payTx, ...zcashTxids(st, maker)]);
  await ctx.close();
  return c.save();
}

async function scenarioD() {
  const scen = 'd_restore_fresh_profile';
  const c = new Checks(scen, 'The browser is lost after the funding; the recovery file saved before the payment is opened in a fresh browser profile; the engine reads the swap back from the chains (request, acceptance and funding already sent) and completes it without sending anything twice');
  const maker = new Maker(scen);
  maker.start();
  const url = writeRun(scen, maker, '30');
  const user = new Wallet(path.join(WORK, scen, 'user'));
  await coinbase(user.ua);
  await coinbase(maker.ua);
  const recv = receiver('aliced');
  await settle();
  if (user.balance() <= 0) throw new Error('the user wallet did not receive its coinbase');
  const a = await newPage(scen, 'first');
  const { deposit, file } = await startSwap(a.page, url, { nearAccount: recv, returnAddress: user.ua, scen });
  const payTx = user.pay(deposit.address, PAY_ZATS);
  const lost = await waitState(a.page, (s) => s.stage === 'funding_sent' || s.stage === 'funding_confirming', 15 * 60_000, 'the funding');
  await a.ctx.close();
  c.facts.lost_at = lost.stage;
  log(`first browser closed at ${lost.stage}; restoring from ${file}`);
  const b = await newPage(scen, 'fresh');
  await b.page.goto(url);
  await b.page.getByRole('button', { name: /Restore a swap/ }).click({ timeout: 120_000 });
  await b.page.locator('input[type=file]').setInputFiles(file);
  const st = await waitState(b.page, (s) => s.stage === 'done' || s.stage === 'refunded' || s.stage === 'exit_retry', 15 * 60_000, 'done after restore');
  // The maker sweeps the ZEC once it read a; give it its k_Z confirmations.
  for (let i = 0; i < 120 && maker.swap()?.stage !== 'Swept'; i++) await sleep(500);
  await maker.stop();
  c.facts.timings = stageTimings(st, b.timings);
  c.facts.timings_first = a.timings.map((t) => ({ what: t.what, secs: t.ms / 1000 }));
  c.ok('the restored swap completed', st.stage === 'done', `${st.stage} ${JSON.stringify(st.lastError ?? '')}`);
  c.ok('the NEAR account received the price', (await wnearAtLeast(recv, PRICE)) === PRICE, await wnear(recv));
  const m = maker.swap();
  c.ok('the maker swept the one funding note with a (one request, one acceptance, one funding)', m && m.stage === 'Swept', m?.stage);
  const sends = st.txs.filter((t) => t.chain === 'zcash').map((t) => t.id);
  c.ok('the restored swap shows exactly one request, one acceptance and one funding', ['request', 'accept', 'funding'].every((id) => sends.filter((x) => x === id).length === 1), sends.join(','));
  const restoredLog = fs.readFileSync(path.join(EVID, scen, 'console-fresh.log'), 'utf8');
  c.ok('the engine reconciled the early record with the chain', /restored: the request was already sent/.test(restoredLog) && /restored: the joint address was already funded|restored: the acceptance was already sent/.test(restoredLog));
  c.ok('no second proof for request, acceptance or funding in the fresh browser', !b.timings.some((t) => ['proof_request', 'proof_accept', 'proof_fund'].includes(t.what)), b.timings.map((t) => t.what).join(','));
  await privacyAudit(c, [payTx, ...zcashTxids(st, maker)]);
  await b.ctx.close();
  return c.save();
}

async function scenarioE() {
  const scen = 'e_quote_below_minimum';
  const c = new Checks(scen, 'The maker’s firm quote is below the minimum the page set (directory rate 32, maker pays 30): the engine stops before the acceptance, pays no premium, and returns the ZEC to the return address');
  const maker = new Maker(scen);
  maker.start();
  const url = writeRun(scen, maker, '32');
  const user = new Wallet(path.join(WORK, scen, 'user'));
  await coinbase(user.ua);
  await coinbase(maker.ua);
  const recv = receiver('alicee');
  await settle();
  if (user.balance() <= 0) throw new Error('the user wallet did not receive its coinbase');
  const userBefore = user.balance();
  const makerBefore = await wnear(chains.maker);
  const { page, ctx, timings } = await newPage(scen, 'main');
  const { deposit } = await startSwap(page, url, { nearAccount: recv, returnAddress: user.ua, scen });
  const payTx = user.pay(deposit.address, PAY_ZATS);
  const st = await waitState(page, (s) => s.stage === 'refunded', 15 * 60_000, 'refunded');
  await page.getByTestId('refunded-title').waitFor({ timeout: 30_000 });
  await page.screenshot({ path: path.join(EVID, scen, 'refunded.png'), fullPage: true });
  await maker.stop();
  await settle();
  c.facts.timings = stageTimings(st, timings);
  c.ok('refund reason QUOTE_BELOW_MINIMUM', st.refund?.reason === 'QUOTE_BELOW_MINIMUM', st.refund?.reason);
  c.ok('stopped before the acceptance: no premium, no funding', !st.txs.some((t) => t.id === 'accept' || t.id === 'funding'));
  c.ok('the maker opened no escrow and paid nothing', (await wnear(chains.maker)) === makerBefore);
  c.ok('the refund was the payment less the request fee and its own fee', Number(st.refund?.amountZats) === PAY_ZATS - 2 * FEE, st.refund?.amountZats);
  c.ok('the user wallet lost exactly three fees (payment, request, refund)', userBefore - user.balance() === 3 * FEE, userBefore - user.balance());
  await privacyAudit(c, [payTx, ...zcashTxids(st, maker)]);
  await ctx.close();
  return c.save();
}

const SCENARIOS = { a: scenarioA, b: scenarioB, c: scenarioC, d: scenarioD, e: scenarioE };

// ---------------------------------------------------------------- main

function writeReport(current) {
  // Every scenario with evidence, so that a partial run keeps the others' last results.
  const results = {};
  for (const k of Object.values(SCEN_NAMES)) {
    const f = path.join(EVID, k, 'result.json');
    if (!fs.existsSync(f)) continue;
    const r = JSON.parse(fs.readFileSync(f, 'utf8'));
    results[k] = current[k] ?? { pass: r.checks.length > 0 && r.checks.every((c) => c.pass), secs: r.secs ?? '?' };
  }
  const lines = ['# Browser scenario results', '', `Generated by \`harness/browser/run.mjs\` on ${new Date().toISOString()}. The taker runs only in headless Chromium through the website (web/dist) with the WebAssembly engine (crates/zecret-wasm); the maker is the native \`zecret maker\`. Zebra 7.0.0-rc.0 regtest (NU7) and a NEAR 2.14.1 sandbox with the escrow, the live wrap.near code (stand-in for USDC) and the live intents.near code.`, '', '| Scenario | Result | Seconds | Run at |', '|---|---|---|---|'];
  for (const [k, v] of Object.entries(results)) {
    const ran = JSON.parse(fs.readFileSync(path.join(EVID, k, 'result.json'), 'utf8')).ran_at ?? '';
    lines.push(`| \`${k}\` | ${v.pass ? 'PASS' : 'FAIL'} | ${v.secs} | ${ran} |`);
  }
  lines.push('', '## Details');
  for (const k of Object.keys(results)) {
    const f = path.join(EVID, k, 'result.json');
    if (!fs.existsSync(f)) continue;
    const r = JSON.parse(fs.readFileSync(f, 'utf8'));
    lines.push('', `### ${k}`, '', r.title, '');
    for (const ck of r.checks) lines.push(`- ${ck.pass ? 'PASS' : 'FAIL'}: ${ck.check}${ck.detail && ck.detail !== 'true' ? ` (${ck.detail})` : ''}`);
    const t = r.facts.timings;
    if (t) {
      lines.push('', `Stages (seconds in each, total ${t.total_secs} s):`, '');
      lines.push(t.stages.map((s) => `${s.stage} ${s.secs ?? ''}`.trim()).join(' → '));
      lines.push('', `Engine timings in the browser: ${t.engine.map((e) => `${e.what} ${e.secs} s`).join(', ')}`);
    }
  }
  fs.writeFileSync(path.join(ROOT, 'harness/BROWSER_RESULTS.md'), lines.join('\n') + '\n');
}

async function main() {
  const args = process.argv.slice(2);
  const serve = args.includes('--serve');
  const sel = args.filter((a) => !a.startsWith('--'));
  const chosen = serve ? [] : sel.length ? sel : Object.keys(SCENARIOS);
  fs.mkdirSync(WORK, { recursive: true });
  if (!process.env.SKIP_BUILD) {
    log('building the wasm engine, the site and the test helper');
    for (const [cmd, a, cwd] of [
      ['sh', [path.join(ROOT, 'crates/zecret-wasm/build.sh')], ROOT],
      ['npx', ['vite', 'build', '--logLevel', 'error'], WEB],
      ['cargo', ['build', '-p', 'zecret-wasm', '--example', 'sandbox_account'], path.join(ROOT, 'crates')],
    ]) {
      const env = { ...process.env };
      delete env.LD_PRELOAD;
      const r = spawnSync(cmd, a, { cwd, stdio: 'inherit', env });
      if (r.status !== 0) throw new Error(`${cmd} ${a.join(' ')} failed`);
    }
  }
  await startChains();
  const nearCors = await fetch(chains.near, { method: 'OPTIONS', headers: { origin: SITE, 'access-control-request-method': 'POST', 'access-control-request-headers': 'content-type' } });
  log(`NEAR sandbox CORS preflight: ${nearCors.status} allow-origin=${nearCors.headers.get('access-control-allow-origin')}`);
  const proxy = corsProxy(PROXY_PORT, chains.zebra);
  const site = siteServer(SITE_PORT, [`http://127.0.0.1:${PROXY_PORT}`, chains.near, `http://127.0.0.1:${RELAY_PORT}`]);
  browser = await chromium.launch({ headless: true });
  const results = {};
  try {
    if (serve) {
      const maker = new Maker('local');
      maker.start();
      const url = writeRun('local', maker, '30');
      const user = new Wallet(path.join(WORK, 'local', 'user'));
      await coinbase(user.ua);
      await coinbase(maker.ua);
      const recv = receiver('alice');
      log(`site: ${url}\n  NEAR account for the tokens: ${recv}\n  pay from: zecret pay --dir ${user.dir} --to <deposit address> --zats <amount>\n  return address: ${user.ua}\n  stop with Ctrl-C`);
      await new Promise(() => {});
    }
    for (const k of chosen) {
      const fn = SCENARIOS[k];
      const t = Date.now();
      const name = fn.name;
      log(`=== scenario ${k} ===`);
      let pass = false;
      let scen = k;
      try {
        pass = await runScenario(k, fn);
      } catch (e) {
        log(`  ERROR ${e.stack || e}`);
      }
      scen = lastScen ?? k;
      results[scen] = { pass, secs: Math.round((Date.now() - t) / 1000), name };
      const rf = path.join(EVID, scen, 'result.json');
      if (fs.existsSync(rf)) fs.writeFileSync(rf, JSON.stringify({ ...JSON.parse(fs.readFileSync(rf, 'utf8')), secs: results[scen].secs, ran_at: new Date().toISOString() }, null, 2));
      spawnSync('pkill', ['-f', `${WORK}/${scen}/maker`]);
    }
  } finally {
    await browser.close();
    proxy.close();
    site.close();
    chainsProc.stdin.end();
    await new Promise((r) => chainsProc.on('exit', r));
  }
  log('SUMMARY: ' + Object.entries(results).map(([k, v]) => `${k} ${v.pass ? 'PASS' : 'FAIL'} (${v.secs}s)`).join(', '));
  writeReport(results);
  process.exit(Object.values(results).every((v) => v.pass) && Object.keys(results).length ? 0 : 1);
}

let lastScen;
const SCEN_NAMES = { a: 'a_full_swap', b: 'b_maker_quiet_refund', c: 'c_reload_twice', d: 'd_restore_fresh_profile', e: 'e_quote_below_minimum' };
async function runScenario(k, fn) {
  lastScen = SCEN_NAMES[k];
  fs.rmSync(path.join(EVID, lastScen), { recursive: true, force: true });
  fs.mkdirSync(path.join(EVID, lastScen), { recursive: true });
  return fn();
}

main().catch((e) => {
  console.error(e);
  try {
    chainsProc?.stdin.end();
  } catch {
    /* already gone */
  }
  process.exit(1);
});
