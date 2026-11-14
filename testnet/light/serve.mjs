#!/usr/bin/env node
// Serves a local production build of the website (web/dist-light, built with
// VITE_ZECRET_CONFIG=/config/testnet-light.json) with the headers of web/vercel.json, for
// the browser swap against the light-payout escrow on testnet (docs/TESTNET.md, "Light
// payout on testnet"). The page's connect-src also allows the local maker's relay.
//
//   node testnet/light/serve.mjs <port> <relay origin, e.g. http://127.0.0.1:3411>
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const WEB = path.join(ROOT, 'web');
const DIST = path.join(WEB, 'dist-light');
const [port = '18480', relay = 'http://127.0.0.1:3411'] = process.argv.slice(2);
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.png': 'image/png' };

function headers(p) {
  const v = JSON.parse(fs.readFileSync(path.join(WEB, 'vercel.json'), 'utf8'));
  const out = {};
  for (const rule of v.headers) {
    const re = new RegExp(`^${rule.source.replace(/\(\.\*\)/g, '.*')}$`);
    if (!re.test(p)) continue;
    for (const h of rule.headers) out[h.key.toLowerCase()] = h.value;
  }
  if (out['content-security-policy']) out['content-security-policy'] = out['content-security-policy'].replace('connect-src', `connect-src ${relay}`);
  return out;
}

http
  .createServer((req, res) => {
    const p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    let f = path.join(DIST, p === '/' ? 'index.html' : p);
    if (!f.startsWith(DIST)) return res.writeHead(403).end();
    if (!fs.existsSync(f) || fs.statSync(f).isDirectory()) f = path.join(DIST, 'index.html');
    res.writeHead(200, { 'content-type': TYPES[path.extname(f)] || 'application/octet-stream', 'cache-control': 'no-store', ...headers(p) });
    fs.createReadStream(f).pipe(res);
  })
  .listen(Number(port), '127.0.0.1', () => console.log(`serving ${DIST} on http://127.0.0.1:${port}`));
