#!/usr/bin/env node
// A partial check of the real engine against Zcash testnet and NEAR testnet, with the
// production headers of web/vercel.json unchanged (no local hosts added to the CSP).
// No testnet ZEC is spent: the check stops at the payment screen.
//
//   source .tools/env.sh && export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers
//   node harness/browser/testnet-smoke.mjs <maker folder>
//
// <maker folder> holds a maker config.json for testnet (crates/zecret-node/config/
// testnet-maker.json) and the maker's NEAR key as near_key.json; the script makes its
// wallet and request key if missing and publishes its signed directory entry to the page.
// It then opens the site with ?config=testnet in headless Chromium (through HTTPS_PROXY if
// set) and checks: the engine loads, verifies the maker's directory entry against NEAR
// testnet, builds the proving keys, reads the Zcash testnet tip over gRPC-web, creates a
// swap and shows a testnet deposit address; no CSP violation.
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '../..');
const WEB = path.join(ROOT, 'web');
const require = createRequire(path.join(WEB, 'package.json'));
const { chromium } = require('playwright');
const PORT = 18381;
const makerDir = process.argv[2];
if (!makerDir) {
  console.error('usage: testnet-smoke.mjs <maker folder>');
  process.exit(2);
}
const bin = path.join(ROOT, 'crates/target/debug/zecret');
const log = (m) => console.log(`[testnet ${new Date().toISOString().slice(11, 19)}] ${m}`);

if (!fs.existsSync(path.join(makerDir, 'wallet.json'))) log(`maker wallet ${execFileSync(bin, ['init', '--dir', makerDir], { encoding: 'utf8' }).trim()}`);
const directory = execFileSync('node', [path.join(HERE, 'make-directory.mjs'), makerDir, '1', 'testnet'], { encoding: 'utf8' });

const v = JSON.parse(fs.readFileSync(path.join(WEB, 'vercel.json'), 'utf8'));
const headers = Object.fromEntries(v.headers.find((h) => h.source === '/(.*)').headers.map((h) => [h.key.toLowerCase(), h.value]));
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
const dist = path.join(WEB, 'dist');
const server = http
  .createServer((req, res) => {
    const p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (p === '/config/testnet-directory.json') {
      res.writeHead(200, { 'content-type': 'application/json', ...headers });
      return res.end(directory);
    }
    let f = path.join(dist, p === '/' ? 'index.html' : p);
    if (!f.startsWith(dist) || !fs.existsSync(f) || fs.statSync(f).isDirectory()) f = path.join(dist, 'index.html');
    res.writeHead(200, { 'content-type': TYPES[path.extname(f)] || 'application/octet-stream', ...headers });
    fs.createReadStream(f).pipe(res);
  })
  .listen(PORT, '127.0.0.1');

const browser = await chromium.launch({ headless: true });
const proxy = process.env.HTTPS_PROXY ? { server: process.env.HTTPS_PROXY, bypass: '127.0.0.1,localhost' } : undefined;
const ctx = await browser.newContext({ acceptDownloads: true, ignoreHTTPSErrors: true, proxy });
const page = await ctx.newPage();
const csp = [];
const lines = [];
page.on('console', (m) => {
  const t = m.text();
  lines.push(t);
  if (/Content Security Policy|Refused to/.test(t)) csp.push(t);
  if (/zecret-(timing|engine)/.test(t)) log(t);
});
const results = {};
let ok = true;
const check = (what, cond, detail = '') => {
  ok &&= !!cond;
  results[what] = !!cond;
  log(`${cond ? 'PASS' : 'FAIL'}  ${what} ${detail}`);
};
try {
  await page.goto(`http://127.0.0.1:${PORT}/?config=testnet`);
  await page.getByLabel('You pay').fill('0.01', { timeout: 120_000 });
  check('the page loaded with the real engine on testnet (network banner)', await page.getByTestId('network-banner').isVisible(), await page.getByTestId('network-banner').innerText());
  await page.getByTestId('near-account').fill('zecret-3f109eea1e.testnet');
  await page.getByTestId('price-line').getByText('You receive at least').waitFor({ timeout: 90_000 });
  check('the live mainnet 1Click preview price shows', true, await page.getByTestId('price-line').innerText());
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await page.getByRole('heading', { name: 'Save your recovery file' }).waitFor({ timeout: 180_000 });
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Save recovery file' }).click();
  const file = await (await dl).path();
  const rec = JSON.parse(fs.readFileSync(file, 'utf8'));
  check('the recovery file is for testnet and holds the wallet key and the swap record', rec.network === 'testnet' && rec.records.wallet && Object.keys(rec.records).some((k) => k.startsWith('swap/')));
  await page.getByRole('button', { name: 'Continue to payment' }).click();
  const link = await page.locator('a[href^="zcash:"]').first().getAttribute('href', { timeout: 30_000 });
  check('the payment screen asks for testnet ZEC at a utest address', /^zcash:utest1/.test(link ?? ''), link?.slice(0, 60));
  // Let the engine sync the new wallet over gRPC-web a few times.
  // Let the engine scan the new wallet over gRPC-web for a minute (one tick every 2 s).
  await page.waitForTimeout(60_000);
  const ticks = lines.filter((l) => l.includes('[zecret-engine] waiting for a deposit')).length;
  const transient = lines.filter((l) => /transient: Zcash sync/.test(l)).length;
  results.transient_errors = transient;
  check('the engine scanned Zcash testnet over gRPC-web and waits for the deposit', ticks > 0, `deposit log lines ${ticks}, transient sync errors retried ${transient}`);
  check('no Content-Security-Policy violation with the production headers', csp.length === 0, csp.join(' | '));
  await page.screenshot({ path: path.join(ROOT, 'harness/evidence/browser/testnet-pay.png'), fullPage: true });
} catch (e) {
  check('the flow ran to the payment screen', false, e.message);
} finally {
  fs.mkdirSync(path.join(ROOT, 'harness/evidence/browser'), { recursive: true });
  fs.writeFileSync(path.join(ROOT, 'harness/evidence/browser/testnet-smoke.json'), JSON.stringify({ at: new Date().toISOString(), results, console: lines }, null, 2));
  await browser.close();
  server.close();
}
process.exit(ok ? 0 : 1);
