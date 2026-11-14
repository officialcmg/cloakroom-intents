// Renders the film frame by frame with headless Chromium.
//   node render.mjs frames <outDir> [from] [to] [workers]   JPEG frames, film seconds [from, to)
//   node render.mjs stills <outDir> t1,t2,...              PNG stills at those times
// ENDCARD=A|B|C|D picks an end-card design for this render only.
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire('/home/user/zecret/web/package.json');
process.env.PLAYWRIGHT_BROWSERS_PATH ||= '/opt/pw-browsers';
const { chromium } = require('playwright');
const [mode, out, a, b, w] = process.argv.slice(2);
fs.mkdirSync(out, { recursive: true });
const FPS = 30;

async function open() {
  const browser = await chromium.launch({ args: ['--font-render-hinting=none', '--disable-lcd-text', '--allow-file-access-from-files'] });
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  page.on('pageerror', (e) => console.error('PAGEERROR', e.message));
  page.on('console', (m) => { if (m.type() === 'error') console.error('CONSOLE', m.text()); });
  await page.goto('file://' + path.join(HERE, 'src/film.html') + (process.env.ENDCARD ? `?endcard=${process.env.ENDCARD}` : ''));
  await page.evaluate(() => window.filmReady);
  return { browser, page };
}

if (mode === 'stills') {
  const { browser, page } = await open();
  for (const t of a.split(',').map(Number)) {
    await page.evaluate((t) => window.renderFrame(t), t);
    await page.screenshot({ path: path.join(out, `still_${t.toFixed(2)}.png`) });
  }
  await browser.close();
} else if (mode === 'frames') {
  const from = Number(a ?? 0), workers = Number(w ?? 1);
  const { browser, page } = await open();
  const dur = await page.evaluate(() => window.FILM.DURATION);
  const to = Number(b ?? dur);
  const f0 = Math.round(from * FPS), f1 = Math.round(to * FPS);
  if (workers > 1) {
    await browser.close();
    const per = Math.ceil((f1 - f0) / workers);
    await Promise.all([...Array(workers)].map((_, i) => new Promise((res, rej) => {
      const s = (f0 + i * per) / FPS, e = Math.min(f1, f0 + (i + 1) * per) / FPS;
      const p = spawn(process.execPath, [fileURLToPath(import.meta.url), 'frames', out, String(s), String(e), '1'], { stdio: 'inherit' });
      p.on('exit', (c) => (c === 0 ? res() : rej(new Error('worker ' + i))));
    })));
  } else {
    for (let f = f0; f < f1; f++) {
      await page.evaluate((t) => window.renderFrame(t), f / FPS);
      await page.screenshot({ path: path.join(out, `f${String(f).padStart(5, '0')}.jpg`), type: 'jpeg', quality: 95 });
      if (f % 150 === 0) console.log('frame', f);
    }
    await browser.close();
  }
}
