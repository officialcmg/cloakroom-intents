// Static server for web/ (the page, the worker and the wasm package).
//   node serve.mjs [port=8080]
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), 'web');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };
const port = Number(process.argv[2] || 8080);

http.createServer((req, res) => {
  const rel = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const f = path.join(root, rel === '/' ? 'index.html' : rel);
  if (!f.startsWith(root)) { res.writeHead(403); return res.end(); }
  fs.readFile(f, (e, d) => {
    if (e) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { 'content-type': types[path.extname(f)] || 'application/octet-stream', 'cache-control': 'no-store' });
    res.end(d);
  });
}).listen(port, '127.0.0.1', () => console.log(`serving ${root} on http://127.0.0.1:${port}`));
