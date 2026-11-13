// Read-only check of the lightwalletd (gRPC-web) backend against Zcash testnet, from the
// wallet worker in headless Chromium. Spends nothing.
//   node testnet.mjs [url=https://zcash-testnet.chainsafe.dev] [blocks=100]
import { spawn } from 'node:child_process';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const HERE = path.dirname(fileURLToPath(import.meta.url));
const URL_ = process.argv[2] || 'https://zcash-testnet.chainsafe.dev';
const N = Number(process.argv[3] || 100);
const WEB_PORT = 18381;

const srv = spawn(process.execPath, [path.join(HERE, 'serve.mjs'), String(WEB_PORT)], { stdio: ['ignore', 'pipe', 'inherit'] });
await new Promise(r => srv.stdout.on('data', d => String(d).includes('serving') && r()));
const proxy = process.env.HTTPS_PROXY || process.env.https_proxy;
const browser = await chromium.launch(proxy ? { proxy: { server: proxy, bypass: '127.0.0.1' } } : {});
// The container's egress proxy re-signs TLS; Chromium does not know its CA.
const ctx = await browser.newContext({ ignoreHTTPSErrors: !!proxy });
const page = await ctx.newPage();
page.on('console', m => { if (process.env.VERBOSE || /panic|error/i.test(m.text())) console.log('   browser: ' + m.text()); });
await page.goto(`http://127.0.0.1:${WEB_PORT}/`);
const E = (cmd, ...args) => page.evaluate(([c, a]) => window.engine(c, ...a), [cmd, args]);
let code = 0;
try {
  await E('ready');
  const p = (await E('lwdProbe', URL_, 10)).value;
  console.log('probe (raw lightwalletd calls from the worker):');
  console.log(JSON.stringify(p, null, 2));
  const birthday = p.tip.height - N + 1;
  await E('create', { backend: 'lwd', url: URL_, network: 'test', birthday, store: false });
  const t = Date.now();
  const h = (await E('sync')).value;
  const ms = Date.now() - t;
  const info = (await E('scanInfo')).value;
  console.log(`wallet sync (the wallet's own sync_lwd code): blocks ${birthday}..${h} (${h - birthday + 1} blocks) in ${(ms / 1000).toFixed(1)} s; ${JSON.stringify(info)}`);
  console.log(`address ${(await E('create', { backend: 'lwd', url: URL_, network: 'test', birthday, store: false })).value.address.slice(0, 20)}... (testnet encoding)`);
  if (h !== p.tip.height && h < p.tip.height) throw new Error('sync did not reach the tip');
  console.log('PASS');
} catch (e) { console.log(String(e && e.stack || e)); console.log('FAIL'); code = 1; }
await browser.close();
srv.kill();
process.exit(code);
