#!/usr/bin/env node
// One swap through the hosted website on Zcash testnet and NEAR testnet (docs/HOSTING.md,
// docs/TESTNET.md "Hosted swap"). It opens the site in headless Chromium, fills the form,
// saves the recovery file, pays the shown address from a native testnet wallet with
// `zecret pay`, and waits until the page reaches a final stage.
//
//   source .tools/env.sh && export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers
//   node testnet/hosted/run.mjs <site URL> <payer wallet folder> <NEAR account> [ZEC amount]
//
// Writes testnet/hosted/result.json and small screenshots next to this file. The recovery
// file and the page console log hold the browser wallet's key; they go to
// .tools/testnet/hosted/ (not in git).
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '../..');
const require = createRequire(path.join(ROOT, 'web/package.json'));
const { chromium } = require('playwright');
const [site, payer, nearAccount, amount = '0.01'] = process.argv.slice(2);
if (!site || !payer || !nearAccount) {
  console.error('usage: run.mjs <site URL> <payer wallet folder> <NEAR account> [ZEC amount]');
  process.exit(2);
}
const PRIVATE = path.join(ROOT, '.tools/testnet/hosted');
fs.mkdirSync(PRIVATE, { recursive: true });
const bin = process.env.ZECRET_BIN || path.join(ROOT, 'crates/target/release/zecret');
const t0 = Date.now();
const log = (m) => console.log(`[hosted ${new Date().toISOString().slice(11, 19)} +${Math.round((Date.now() - t0) / 1000)}s] ${m}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// The payer is a one-shot command-line wallet. A public light-wallet server sometimes
// answers 502 for a moment. Read-only steps are run again; a send is run again only when
// it failed while reading the chain, before anything was broadcast.
const READ_STEP = /GetLatestBlock|GetBlockRange|GetTreeState|GetLightdInfo/;
function payerCli(args, { send = false } = {}) {
  for (let i = 1; ; i++) {
    try {
      return execFileSync(bin, args, { encoding: 'utf8' });
    } catch (e) {
      const err = String(e.stderr || e.message);
      if (i >= 5 || (send && !READ_STEP.test(err.split('\n')[0]))) throw e;
      log(`payer wallet: ${err.split('\n')[0].slice(0, 120)}; try ${i + 1} of 5 in 20 s`);
      execFileSync('sleep', ['20']);
    }
  }
}
const result = { site, amountZec: amount, nearAccount, startedAt: new Date().toISOString(), checks: [], facts: {} };
const check = (what, cond, detail = '') => {
  result.checks.push({ check: what, pass: !!cond, detail: String(detail) });
  log(`${cond ? 'PASS' : 'FAIL'}  ${what} ${detail}`);
};
const shot = (page, name) => page.screenshot({ path: path.join(HERE, `${name}.jpg`), type: 'jpeg', quality: 60 });

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

async function waitState(page, pred, timeoutMs, what) {
  const end = Date.now() + timeoutMs;
  let last;
  let lastStage;
  while (Date.now() < end) {
    try {
      last = await webState(page);
    } catch {
      last = undefined;
    }
    if (last && last.stage !== lastStage) {
      lastStage = last.stage;
      log(`stage ${last.stage}`);
    }
    if (last && pred(last)) return last;
    await sleep(1000);
  }
  throw new Error(`timed out waiting for ${what} (stage ${last?.stage}, error ${JSON.stringify(last?.lastError)})`);
}

const payerUa = JSON.parse(payerCli(['balance', '--dir', payer]));
result.facts.payerBefore = payerUa;
log(`payer ${payerUa.ua.slice(0, 20)}… holds ${payerUa.zats} zats at height ${payerUa.height}`);

const browser = await chromium.launch({ headless: true });
// ZECRET_PROXY_BYPASS (for example 127.0.0.1,localhost): hosts the browser reaches without the
// proxy, for a site and a relay served on this computer (docs/TESTNET.md, "Light payout on testnet").
const proxy = process.env.HTTPS_PROXY ? { server: process.env.HTTPS_PROXY, ...(process.env.ZECRET_PROXY_BYPASS ? { bypass: process.env.ZECRET_PROXY_BYPASS } : {}) } : undefined;
const ctx = await browser.newContext({ acceptDownloads: true, viewport: { width: 1100, height: 820 }, proxy });
const page = await ctx.newPage();
const consoleLog = fs.createWriteStream(path.join(PRIVATE, 'console.log'), { flags: 'a' });
const csp = [];
const timings = [];
page.on('console', (m) => {
  const t = m.text();
  consoleLog.write(`${new Date().toISOString()} ${t}\n`);
  if (/Content Security Policy|Refused to/.test(t)) csp.push(t);
  const tm = /^\[zecret-timing\] (\S+) (\d+)/.exec(t);
  if (tm) timings.push({ what: tm[1], secs: Math.round(Number(tm[2]) / 100) / 10 });
});
page.on('pageerror', (e) => consoleLog.write(`PAGEERROR ${e.message}\n`));
page.on('dialog', (d) => d.accept());

let ok = false;
try {
  await page.goto(new URL('/swap', site).href);
  await page.getByLabel('You pay').fill(amount, { timeout: 180_000 });
  const banner = await page.getByTestId('network-banner').innerText().catch(() => '');
  check('the site opened with the real engine on testnet by default (network banner)', /testnet/i.test(banner), banner.replace(/\s+/g, ' ').slice(0, 120));
  await page.getByTestId('near-account').fill(nearAccount);
  await page.getByRole('button', { name: /Add a Zcash return address/ }).click();
  await page.getByLabel(/Zcash return address/).fill(payerUa.ua);
  await page.getByTestId('price-line').getByText('You receive at least').waitFor({ timeout: 120_000 });
  result.facts.preview = await page.getByTestId('price-line').innerText();
  await shot(page, '1-form');
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await page.getByRole('heading', { name: 'Save your recovery file' }).waitFor({ timeout: 240_000 });
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Save recovery file' }).click();
  await (await dl).saveAs(path.join(PRIVATE, 'recovery.json'));
  check('the recovery file was saved', fs.statSync(path.join(PRIVATE, 'recovery.json')).size > 0);
  await page.getByRole('button', { name: 'Continue to payment' }).click();
  const st0 = await waitState(page, (s) => s.stage === 'awaiting_deposit', 60_000, 'the payment screen');
  await shot(page, '2-pay');
  const zats = Number(st0.deposit.expectedZats);
  check('the page shows a testnet deposit address and amount', st0.deposit.address.startsWith('utest1') && zats > 0, `${zats} zats`);
  result.facts.deposit = { expectedZats: zats };
  // ZECRET_PAY=faucet: no wallet pays. The Fauzec testnet faucet pays the page's address
  // (a fixed 1 TAZ; the page sends the surplus to the return address at the end).
  let payTx;
  if (process.env.ZECRET_PAY === 'faucet') {
    const out = execFileSync('curl', ['-sS', '-X', 'POST', 'https://fauzec.com/api/v1/claim', '-H', 'content-type: application/json', '-d', JSON.stringify({ network: 'testnet', address: st0.deposit.address })], { encoding: 'utf8' });
    result.facts.faucet = JSON.parse(out);
    payTx = `faucet:${out.slice(0, 300)}`;
  } else {
    payTx = payerCli(['pay', '--dir', payer, '--to', st0.deposit.address, '--zats', String(zats)], { send: true }).trim().split('\n').pop();
  }
  result.facts.payTx = payTx;
  log(`paid ${zats} zats in ${payTx}`);
  const st = await waitState(page, (s) => ['done', 'refunded', 'exit_retry', 'failed'].includes(s.stage), 45 * 60_000, 'a final stage');
  await page.getByTestId('done-title').waitFor({ timeout: 30_000 }).catch(() => undefined);
  await shot(page, '3-done');
  result.facts.finalStage = st.stage;
  result.facts.history = st.history.map((h, i) => ({ stage: h.stage, at: new Date(h.at).toISOString(), secs: i + 1 < st.history.length ? Math.round((st.history[i + 1].at - h.at) / 100) / 10 : null }));
  result.facts.txs = st.txs;
  result.facts.escrow = st.escrow;
  result.facts.payout = st.payout;
  result.facts.exit = st.exit && { phase: st.exit.phase, deliveredAmount: st.exit.deliveredAmount, intentHash: st.exit.intentHash, direct: st.exit.direct };
  result.facts.quote = st.quote;
  result.facts.engineTimings = timings;
  result.facts.doneTitle = await page.getByTestId('done-title').innerText().catch(() => '');
  check('the page reached done', st.stage === 'done', `${st.stage} ${JSON.stringify(st.lastError ?? '')}`);
  check('no Content-Security-Policy violation', csp.length === 0, csp.slice(0, 3).join(' | '));
  ok = st.stage === 'done' && csp.length === 0;
} catch (e) {
  check('the flow ran to the end', false, e.message);
  await shot(page, 'error').catch(() => undefined);
  try {
    const s = await webState(page);
    result.facts.lastState = s && { stage: s.stage, lastError: s.lastError, history: s.history, txs: s.txs, escrow: s.escrow };
  } catch {
    /* page gone */
  }
} finally {
  result.finishedAt = new Date().toISOString();
  result.totalSecs = Math.round((Date.now() - t0) / 1000);
  fs.writeFileSync(path.join(HERE, 'result.json'), JSON.stringify(result, null, 2));
  await browser.close();
}
process.exit(ok ? 0 : 1);
