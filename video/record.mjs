#!/usr/bin/env node
// Records one real swap on the hosted website (Zcash testnet and NEAR testnet) for the
// launch film. Adapted from testnet/hosted/record-demo.mjs. Chromium runs headed and full
// screen in a private X display (Xvfb, 2560x1600), at 1280x800 CSS pixels with device
// scale 2, so the film can zoom into the footage and keep the type sharp. ffmpeg captures
// that display at 30 frames per second with wall-clock timestamps. cut_footage.py turns
// the recording into the footage frames of the film.
//
//   export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers
//   node video/record.mjs <site URL> <payer wallet folder> <NEAR account> <raw dir> [ZEC amount]
//
// <raw dir> receives raw.mkv (H.264, frame times are Unix seconds), marks.json (stage
// changes and script events, Unix seconds) and facts.json (transactions and stage
// history). Keep it outside the repository: it is large. Needs Xvfb and ffmpeg. The recovery file and the page console log hold
// the browser wallet's key; they go to .tools/testnet/hosted/ (not in git) and are never
// opened by this script.
import { execFile, execFileSync, spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const require = createRequire(path.join(ROOT, 'web/package.json'));
const { chromium } = require('playwright');
const [site, payer, nearAccount, rawDir, amount = '0.01'] = process.argv.slice(2);
if (!site || !payer || !nearAccount || !rawDir) {
  console.error('usage: record-demo.mjs <site URL> <payer wallet folder> <NEAR account> <raw dir> [ZEC amount]');
  process.exit(2);
}
const PRIVATE = path.join(rawDir, 'private'); // recovery file and console log: never opened, never in git
fs.mkdirSync(rawDir, { recursive: true });
fs.mkdirSync(PRIVATE, { recursive: true, mode: 0o700 });
const bin = process.env.ZECRET_BIN || path.join(ROOT, '.tools/soak/zecret');
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const t0 = Date.now();
const now = () => Date.now() / 1000;
const log = (m) => console.log(`[demo ${new Date().toISOString().slice(11, 19)} +${Math.round((Date.now() - t0) / 1000)}s] ${m}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const marks = [];
const mark = (name, extra = {}) => {
  marks.push({ name, t: now(), ...extra });
  log(`mark ${name} ${Object.keys(extra).length ? JSON.stringify(extra) : ''}`);
};
const facts = { site, amountZec: amount, nearAccount, startedAt: new Date().toISOString() };

// The cursor: a dot that follows mouse events, with a ring on press. Playwright draws none.
// Styles are set through the CSSOM: the site's Content-Security-Policy blocks <style> tags.
const CURSOR = `(() => {
  const set = (el, css) => { for (const [k, v] of Object.entries(css)) el.style.setProperty(k, v); };
  const add = () => {
    if (document.getElementById('__demo_cursor')) return;
    const c = document.createElement('div');
    c.id = '__demo_cursor';
    const p = window.__demoCursorPos || [640, 400];
    set(c, { position: 'fixed', left: p[0] + 'px', top: p[1] + 'px', width: '22px', height: '22px', margin: '-11px 0 0 -11px',
      'border-radius': '50%', background: 'rgba(12,24,20,.85)', border: '3px solid #fff', 'box-sizing': 'border-box',
      'box-shadow': '0 1px 6px rgba(0,0,0,.4)', 'z-index': '2147483647', 'pointer-events': 'none', transition: 'transform .12s ease-out' });
    const r = document.createElement('div');
    set(r, { position: 'absolute', left: '50%', top: '50%', width: '46px', height: '46px', margin: '-23px 0 0 -23px', 'border-radius': '50%',
      border: '3px solid rgba(23,115,90,0)', 'box-sizing': 'border-box', transition: 'border-color .25s, transform .25s', transform: 'scale(.6)' });
    c.appendChild(r);
    document.documentElement.appendChild(c);
  };
  const press = (down) => {
    const c = document.getElementById('__demo_cursor');
    if (!c) return;
    c.style.setProperty('transform', down ? 'scale(.75)' : 'scale(1)');
    c.firstChild.style.setProperty('border-color', down ? 'rgba(23,115,90,.7)' : 'rgba(23,115,90,0)');
    c.firstChild.style.setProperty('transform', down ? 'scale(1.3)' : 'scale(.6)');
  };
  document.addEventListener('mousemove', (e) => {
    window.__demoCursorPos = [e.clientX, e.clientY];
    const c = document.getElementById('__demo_cursor');
    if (c) { c.style.setProperty('left', e.clientX + 'px'); c.style.setProperty('top', e.clientY + 'px'); }
  }, true);
  document.addEventListener('mousedown', () => press(true), true);
  document.addEventListener('mouseup', () => setTimeout(() => press(false), 140), true);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', add); else add();
  new MutationObserver(add).observe(document, { childList: true, subtree: true });
})();`;

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

const payerUa = JSON.parse(execFileSync(bin, ['balance', '--dir', payer], { encoding: 'utf8' }));
facts.payerBefore = { zats: payerUa.zats, height: payerUa.height };
log(`payer holds ${payerUa.zats} zats at height ${payerUa.height}`);

// A private X display with no window manager; the browser goes full screen in it.
const DISPLAY = process.env.DEMO_DISPLAY || ':97';
const xvfb = spawn('Xvfb', [DISPLAY, '-screen', '0', '2560x1600x24', '-nolisten', 'tcp'], { stdio: 'ignore' });
await sleep(1500);
const browser = await chromium.launch({
  headless: false,
  env: { ...process.env, DISPLAY },
  args: ['--window-position=0,0', '--window-size=1280,800', '--force-device-scale-factor=2', '--hide-scrollbars', '--disable-infobars'],
  ignoreDefaultArgs: ['--enable-automation'],
});
const proxy = process.env.HTTPS_PROXY ? { server: process.env.HTTPS_PROXY } : undefined;
const ctx = await browser.newContext({ acceptDownloads: true, viewport: null, colorScheme: 'light', proxy });
await ctx.addInitScript(CURSOR);
const page = await ctx.newPage();
const cdp = await ctx.newCDPSession(page);
const { windowId } = await cdp.send('Browser.getWindowForTarget');
await cdp.send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'fullscreen' } });
const consoleLog = fs.createWriteStream(path.join(PRIVATE, `console-demo-${stamp}.log`), { flags: 'a' });
const timings = [];
page.on('console', (m) => {
  const t = m.text();
  consoleLog.write(`${new Date().toISOString()} ${t}\n`);
  const tm = /^\[zecret-timing\] (\S+) (\d+)/.exec(t);
  if (tm) timings.push({ what: tm[1], secs: Math.round(Number(tm[2]) / 100) / 10 });
});
page.on('pageerror', (e) => consoleLog.write(`PAGEERROR ${e.message}\n`));
page.on('dialog', (d) => d.accept());

// Pointer and scrolling helpers, paced like a person.
let mouse = [640, 400];
const ease = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
async function moveMouse(x, y, ms = 650) {
  const [x0, y0] = mouse;
  const n = Math.max(8, Math.round(ms / 16));
  for (let i = 1; i <= n; i++) {
    const e = ease(i / n);
    await page.mouse.move(x0 + (x - x0) * e, y0 + (y - y0) * e);
    await sleep(ms / n);
  }
  mouse = [x, y];
}
async function scrollTo(y, ms = 1200) {
  await page.evaluate(
    ([y, ms]) =>
      new Promise((res) => {
        const max = document.documentElement.scrollHeight - innerHeight;
        const to = Math.max(0, Math.min(max, y));
        const y0 = scrollY;
        const t0 = performance.now();
        const ease = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
        const step = (t) => {
          const p = Math.min(1, (t - t0) / ms);
          window.scrollTo({ top: y0 + (to - y0) * ease(p), behavior: 'instant' });
          if (p < 1) requestAnimationFrame(step);
          else res();
        };
        requestAnimationFrame(step);
      }),
    [y, ms],
  );
}
const docTop = (loc) => loc.evaluate((e) => e.getBoundingClientRect().top + scrollY);
// Bring an element into a comfortable band of the viewport, smoothly.
async function reveal(loc, at = 260, ms = 900) {
  await loc.waitFor({ state: 'visible', timeout: 240_000 });
  const box = await loc.boundingBox();
  if (box && box.y >= 110 && box.y + box.height <= 560) return;
  await scrollTo((await docTop(loc)) - at, ms);
}
async function pointAt(loc, ms = 650) {
  const box = await loc.boundingBox();
  await moveMouse(box.x + box.width / 2, box.y + box.height / 2, ms);
}
async function clickOn(loc, ms = 650) {
  await reveal(loc);
  await pointAt(loc, ms);
  await sleep(180);
  const box = await loc.boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
}

// Where the parts of the page are, in CSS pixels of the viewport, whenever they move. The
// film points its callouts and its camera at these boxes.
const boxLog = [];
let stopBoxes = false;
async function pollBoxes() {
  let last = '';
  while (!stopBoxes) {
    try {
      const b = await page.evaluate(() => {
        const r = (el) => { if (!el) return null; const q = el.getBoundingClientRect(); return q.width ? [Math.round(q.x), Math.round(q.y), Math.round(q.width), Math.round(q.height)] : null; };
        const one = (sel) => r(document.querySelector(sel));
        const out = { scrollY: Math.round(scrollY) };
        for (const [k, sel] of Object.entries({ price: '[data-testid=price-line]', qr: '.qr-plate', deposit: '[data-testid=deposit-address]', now: 'section.now', nowTitle: '[data-testid=now-title]', stepsCard: '.steps-card', summary: '.progress-top', done: '[data-testid=done-title]', banner: '.site-banner', payAmount: 'input[aria-label="You pay"], #amount', form: 'form' })) {
          const v = one(sel); if (v) out[k] = v;
        }
        const steps = [...document.querySelectorAll('ol.steps > li')].map((li) => ({ box: r(li), dot: r(li.querySelector('.step-dot')), cls: li.className, title: (li.querySelector('.step-title')?.firstChild?.textContent || '').trim() }));
        if (steps.length) out.steps = steps;
        const ti = document.querySelector('[data-testid=now-title]'); if (ti) out.nowText = ti.textContent.trim();
        const dt = document.querySelector('[data-testid=done-title]'); if (dt) out.doneText = dt.textContent.trim();
        return out;
      });
      const j = JSON.stringify(b);
      if (j !== last) { last = j; boxLog.push({ t: now(), ...b }); }
    } catch { /* navigating */ }
    await sleep(250);
  }
}

let ok = false;
let ff;
let stopPoll = false;
let lastState;
const stages = [];
async function pollStages() {
  let lastStage;
  while (!stopPoll) {
    try {
      const s = await webState(page);
      if (s) {
        lastState = s;
        if (s.stage !== lastStage) {
          lastStage = s.stage;
          stages.push({ stage: s.stage, seenAt: now() });
          mark(`stage:${s.stage}`);
        }
      }
    } catch {
      /* page busy or navigating */
    }
    await sleep(400);
  }
}

try {
  // 1. Landing page.
  await page.goto(site, { waitUntil: 'networkidle' });
  const view = await page.evaluate(() => [innerWidth, innerHeight, devicePixelRatio]);
  facts.viewport = view;
  if (view[0] !== 1280 || view[1] !== 800 || view[2] !== 2) throw new Error(`viewport ${view}`);
  await sleep(3500); // the full-screen notice fades before the capture starts
  ff = spawn('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-f', 'x11grab', '-draw_mouse', '0', '-framerate', '30', '-video_size', '2560x1600', '-use_wallclock_as_timestamps', '1', '-i', DISPLAY, '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '12', '-pix_fmt', 'yuv444p', '-copyts', '-fps_mode', 'passthrough', path.join(rawDir, 'raw.mkv')], { stdio: ['pipe', 'inherit', 'inherit'] });
  await sleep(1000);
  await page.mouse.move(...mouse);
  await sleep(600);
  mark('landing');
  await moveMouse(900, 470, 1200);
  await sleep(2200);
  mark('scroll_start');
  mark('scroll_top');
  mark('landing_end');
  const boxesDone = pollBoxes();

  // 2. The form.
  const heroStart = page.getByRole('link', { name: /Start a swap/ }).nth(1);
  await clickOn(heroStart, 800);
  mark('open_swap');
  const pay = page.getByLabel('You pay');
  await pay.waitFor({ state: 'visible', timeout: 180_000 });
  await sleep(400);
  mark('form_ready');
  const banner = await page.getByTestId('network-banner').innerText().catch(() => '');
  facts.banner = banner.replace(/\s+/g, ' ').trim();
  await scrollTo((await docTop(page.getByText('You pay', { exact: true }).first())) - 150, 1000);
  await clickOn(pay);
  await page.keyboard.type(amount, { delay: 160 });
  await sleep(500);
  const near = page.getByTestId('near-account');
  await clickOn(near);
  await page.keyboard.type(nearAccount, { delay: 55 });
  await sleep(400);
  const addReturn = page.getByRole('button', { name: /Add a Zcash return address/ });
  await clickOn(addReturn);
  await sleep(300);
  const ret = page.getByLabel(/Zcash return address/);
  await clickOn(ret);
  mark('paste_return');
  await ret.fill(payerUa.ua); // pasted, as a person would paste a 213-character address
  await sleep(500);
  const price = page.getByTestId('price-line');
  await price.getByText('You receive at least').waitFor({ timeout: 120_000 });
  mark('price_wait_end');
  await reveal(price, 300);
  facts.preview = (await price.innerText()).replace(/\s+/g, ' ').trim();
  await pointAt(price, 700);
  mark('price_shown');
  await sleep(3000);

  // 3. Recovery file and the payment screen.
  await clickOn(page.getByRole('button', { name: 'Continue', exact: true }));
  mark('continue');
  await page.getByRole('heading', { name: 'Save your recovery file' }).waitFor({ timeout: 240_000 });
  mark('recovery_shown');
  await scrollTo((await docTop(page.getByRole('heading', { name: 'Save your recovery file' }))) - 230, 900);
  await sleep(1200);
  const dl = page.waitForEvent('download');
  await clickOn(page.getByRole('button', { name: 'Save recovery file' }));
  const recoveryPath = path.join(PRIVATE, `recovery-demo-${stamp}.json`);
  await (await dl).saveAs(recoveryPath);
  facts.recoverySaved = fs.statSync(recoveryPath).size > 0; // the size only; the file is never read
  mark('recovery_saved');
  await sleep(1200);
  await clickOn(page.getByRole('button', { name: 'Continue to payment' }));
  mark('continue_to_payment');
  const poll = pollStages();
  const deadline = Date.now() + 120_000;
  while (!(lastState && lastState.stage === 'awaiting_deposit' && lastState.deposit) && Date.now() < deadline) await sleep(300);
  if (!lastState?.deposit) throw new Error('the payment screen did not appear');
  await page.getByTestId('deposit-address').waitFor({ timeout: 60_000 });
  if ((await page.evaluate(() => scrollY)) > 0) await scrollTo(0, 800);
  await moveMouse(445, 487, 900); // over the QR code
  mark('pay_screen');
  const zats = Number(lastState.deposit.expectedZats);
  facts.deposit = { address: lastState.deposit.address, expectedZats: zats };
  await sleep(3500);
  // DEMO_DRY=1: a rehearsal that stops before any money moves.
  if (process.env.DEMO_DRY === '1') throw new Error('dry run: stopped before the payment');

  // 4. Pay from the testnet wallet, off screen. Async, so the script keeps polling the page.
  mark('pay_start');
  const { stdout } = await promisify(execFile)(bin, ['pay', '--dir', payer, '--to', lastState.deposit.address, '--zats', String(zats)], { encoding: 'utf8' });
  const payTx = stdout.trim().split('\n').pop();
  facts.payTx = payTx;
  mark('pay_sent', { tx: payTx });
  await moveMouse(1180, 640, 900);

  // 5. Progress. Keep the "Now" card and the steps in view.
  let placed = false;
  const end = Date.now() + 45 * 60_000;
  while (Date.now() < end) {
    if (!placed && (await page.getByTestId('now-title').count())) {
      await sleep(600);
      await scrollTo((await docTop(page.locator('.progress-top'))) - 28, 1000);
      placed = true;
      mark('progress_placed');
    } else if (placed) {
      // Keep the current step in view as the list grows.
      const cur = page.locator('ol.steps > li.is-current').first();
      if (await cur.count()) {
        const bx = await cur.boundingBox().catch(() => null);
        if (bx && bx.y + bx.height > 760) {
          await scrollTo((await page.evaluate(() => scrollY)) + (bx.y + bx.height - 640), 900);
          mark('progress_scrolled');
        }
      }
    }
    if (lastState && ['done', 'refunded', 'exit_retry', 'failed'].includes(lastState.stage)) break;
    await sleep(500);
  }
  const st = lastState;
  facts.finalStage = st.stage;

  // 6. The finished screen.
  await page.getByTestId('done-title').waitFor({ timeout: 30_000 }).catch(() => undefined);
  await sleep(300);
  if ((await page.evaluate(() => scrollY)) > 0) await scrollTo(0, 900);
  mark('final_screen');
  const title = page.getByTestId(st.stage === 'done' ? 'done-title' : 'refunded-title');
  facts.doneTitle = (await title.count()) ? await title.innerText() : '';
  await sleep(4500);
  const revealed = page.getByRole('heading', { name: 'What this swap revealed' });
  if (await revealed.count()) {
    await moveMouse(1100, 520, 800);
    await scrollTo((await docTop(revealed)) - 120, 1600);
    mark('revealed_shown');
    await sleep(4500);
  }
  mark('end');
  ok = st.stage === 'done';

  facts.history = st.history.map((h, i) => ({ stage: h.stage, at: new Date(h.at).toISOString(), atEpoch: h.at / 1000, secs: i + 1 < st.history.length ? Math.round((st.history[i + 1].at - h.at) / 100) / 10 : null }));
  facts.txs = st.txs;
  facts.escrow = st.escrow;
  facts.payout = st.payout;
  facts.exit = st.exit && { phase: st.exit.phase, deliveredAmount: st.exit.deliveredAmount, intentHash: st.exit.intentHash, direct: st.exit.direct };
  facts.quote = st.quote;
  facts.lastError = st.lastError ?? null;
  stopPoll = true;
  await poll;
} catch (e) {
  facts.error = e.message;
  log(`ERROR ${e.message}`);
  mark('error', { message: e.message });
  await sleep(3000);
  try {
    const s = await webState(page);
    facts.lastState = s && { stage: s.stage, lastError: s.lastError, history: s.history, txs: s.txs, escrow: s.escrow };
  } catch {
    /* page gone */
  }
} finally {
  stopPoll = true;
  if (ff) {
    ff.stdin.write('q');
    await new Promise((r) => ff.on('exit', r));
  }
  stopBoxes = true;
  fs.writeFileSync(path.join(rawDir, 'boxes.json'), JSON.stringify(boxLog));
  facts.stagesSeen = stages;
  facts.engineTimings = timings;
  facts.finishedAt = new Date().toISOString();
  fs.writeFileSync(path.join(rawDir, 'marks.json'), JSON.stringify(marks, null, 1));
  fs.writeFileSync(path.join(rawDir, 'facts.json'), JSON.stringify(facts, null, 2));
  await browser.close();
  xvfb.kill();
  log(`final stage ${facts.finalStage}`);
}
process.exit(ok ? 0 : 1);
