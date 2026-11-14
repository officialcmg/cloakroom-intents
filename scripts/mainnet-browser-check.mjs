// Read-only browser check of the mainnet endpoints, from a page in headless Chromium.
// Spends nothing. It serves web/dist (or DIST) on a local port, opens it, and from the
// page:
//   1. calls GetLightdInfo on the Zcash mainnet light-wallet server with fetch (gRPC-web)
//      and prints the server's chain, branch id, height and the CORS answer;
//   2. loads the browser engine (/engine/zecret_wasm.js) and asks it for the Zcash mainnet
//      tip with the website's mainnet settings (wallet_for with network "main");
//   3. reads the final block of the NEAR mainnet RPC, and asks 1Click for its token list.
// It prints PASS or FAIL per step and exits 1 if one failed.
//
//   node scripts/mainnet-browser-check.mjs            (after `npm run build` in web/)
//
// It needs Playwright from web/node_modules (npm ci in web/).
import { createServer } from 'node:http';
import { readFileSync, existsSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WEB = path.join(HERE, '..', 'web');
const require = createRequire(path.join(WEB, 'package.json'));
const { chromium } = require('playwright');
const DIST = process.env.DIST || path.join(WEB, 'dist');
const cfg = JSON.parse(readFileSync(path.join(WEB, 'public/config/mainnet.json'), 'utf8'));
const LWD = cfg.zcash.lightwalletd;
// near.rpc is one URL or an ordered list (incident 2026-11-14).
const RPCS = [cfg.near.rpc].flat();
// oneClickUrl may be a path on the site (the proxy, docs/ONECLICK_PROXY.md). This local
// server has no proxy, so the check then asks 1Click itself, as the page does without one.
const ONECLICK_SET = cfg.oneClickUrl ?? cfg.exit.oneClickUrl ?? 'https://1click.chaindefuser.com';
const ONECLICK = ONECLICK_SET.startsWith('/') ? 'https://1click.chaindefuser.com' : ONECLICK_SET;
if (ONECLICK !== ONECLICK_SET) console.log(`note: the configuration names ${ONECLICK_SET} on the site; this check asks ${ONECLICK} directly (check the proxy with the curl in docs/ONECLICK_PROXY.md)`);
if (!existsSync(path.join(DIST, 'engine/zecret_wasm.js'))) {
  console.log(`FAIL no engine in ${DIST}/engine: build the engine and the site first`);
  process.exit(1);
}

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json', '.css': 'text/css' };
// The engine's transport is a synchronous XMLHttpRequest, which browsers allow only in a
// worker (as the website runs it): this worker loads the engine and asks for the tip.
const WORKER = `
import init, { zcashTip } from '/engine/zecret_wasm.js';
onmessage = async (e) => {
  try { await init(); postMessage({ ok: true, tip: zcashTip(e.data) }); }
  catch (err) { postMessage({ ok: false, error: String(err && err.message || err) }); }
};`;
const server = createServer((req, res) => {
  if (req.url === '/check-worker.js') {
    res.writeHead(200, { 'content-type': 'text/javascript' });
    return res.end(WORKER);
  }
  const p = path.join(DIST, decodeURIComponent(new URL(req.url, 'http://x').pathname));
  if (!p.startsWith(DIST) || !existsSync(p) || statSync(p).isDirectory()) {
    res.writeHead(200, { 'content-type': 'text/html' });
    return res.end('<!doctype html><title>check</title>');
  }
  res.writeHead(200, { 'content-type': TYPES[path.extname(p)] ?? 'application/octet-stream' });
  res.end(readFileSync(p));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${server.address().port}`;

const proxy = process.env.HTTPS_PROXY || process.env.https_proxy;
const browser = await chromium.launch(proxy ? { proxy: { server: proxy, bypass: '127.0.0.1' } } : {});
// A build machine whose egress proxy re-signs TLS: Chromium does not know that CA.
const ctx = await browser.newContext({ ignoreHTTPSErrors: !!proxy });
const page = await ctx.newPage();
await page.goto(`${origin}/check.html`);
let failed = 0;
const line = (ok, what) => {
  console.log(`${ok ? 'PASS' : 'FAIL'} ${what}`);
  if (!ok) failed++;
};

// 1. gRPC-web from the page.
const info = await page.evaluate(async (lwd) => {
  const r = await fetch(`${lwd}/cash.z.wallet.sdk.rpc.CompactTxStreamer/GetLightdInfo`, {
    method: 'POST',
    headers: { 'content-type': 'application/grpc-web+proto', 'x-grpc-web': '1' },
    body: new Uint8Array([0, 0, 0, 0, 0]),
  });
  const b = new Uint8Array(await r.arrayBuffer());
  const text = new TextDecoder('latin1').decode(b);
  return { status: r.status, acao: r.headers.get('access-control-allow-origin'), main: text.includes('main'), branch: /77190ad9/.test(text), grpcOk: /grpc-status:\s*0/.test(text), bytes: b.length };
}, LWD);
line(info.status === 200 && info.grpcOk && info.main, `${LWD} GetLightdInfo from page origin ${origin}: HTTP ${info.status}, chain main ${info.main}, ${info.bytes} bytes, grpc-status 0 ${info.grpcOk}`);
line(info.acao === origin || info.acao === '*', `CORS: access-control-allow-origin ${info.acao}`);
line(info.branch, 'the server reports consensus branch id 77190ad9 (NU7)');

// 2. The engine reads the mainnet tip.
const tip = await page.evaluate(async (cfg) => {
  const core = {
    chains: { lightwalletd: cfg.zcash.lightwalletd, birthday: 1, network: 'main', near_rpc: [cfg.near.rpc].flat(), escrow: 'escrow.check.near', token: cfg.near.token, intents: cfg.near.verifier },
    timing: cfg.timing,
    maker: { ua: 'u', epoch: 1, rfq_pub: '00', near_account: 'm', near_pk: 'ed25519:1', sig: '00' },
    escrow_code_hash: 'x',
    max_premium: 0,
    min_stipend: cfg.minStipend,
  };
  const w = new Worker('/check-worker.js', { type: 'module' });
  const r = await new Promise((resolve) => {
    w.onmessage = (e) => resolve(e.data);
    w.onerror = (e) => resolve({ ok: false, error: e.message });
    w.postMessage(JSON.stringify(core));
  });
  w.terminate();
  return r;
}, cfg);
line(tip.ok && tip.tip > 3_543_405, `engine zcashTip on mainnet: ${tip.ok ? tip.tip : tip.error}`);

// 3. NEAR RPC and 1Click from the page.
for (const RPC of RPCS) {
  const near = await page.evaluate(async (rpc) => {
    const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'block', params: { finality: 'final' } }) });
    const j = await r.json();
    return { status: r.status, height: j.result?.header?.height, chain: j.result ? 'ok' : JSON.stringify(j.error) };
  }, RPC);
  line(near.status === 200 && near.height > 0, `${RPC} final block ${near.height} from the page`);
}
const oc = await page.evaluate(async (u) => {
  const r = await fetch(`${u}/v0/tokens`);
  const j = await r.json();
  return { status: r.status, n: Array.isArray(j) ? j.length : 0, acao: r.headers.get('access-control-allow-origin') };
}, ONECLICK);
line(oc.status === 200 && oc.n > 0, `${ONECLICK}/v0/tokens from the page: ${oc.n} assets (the cross-origin request was allowed)`);

await browser.close();
server.close();
console.log(failed ? `${failed} BROWSER CHECK(S) FAILED` : 'ALL BROWSER CHECKS PASSED');
process.exit(failed ? 1 : 0);
