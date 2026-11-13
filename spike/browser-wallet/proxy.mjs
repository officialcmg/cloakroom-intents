// CORS proxy for local development: the browser cannot call Zebra's JSON-RPC directly,
// because Zebra sends no CORS headers. This forwards POST bodies to Zebra and adds them.
//   node proxy.mjs [listenPort=18300] [target=http://127.0.0.1:18232]
import http from 'node:http';

const port = Number(process.argv[2] || 18300);
const target = process.argv[3] || 'http://127.0.0.1:18232';
const cors = {
  'access-control-allow-origin': '*',
  'access-control-allow-methods': 'POST, OPTIONS',
  'access-control-allow-headers': 'content-type',
  'access-control-max-age': '600',
};

http.createServer((req, res) => {
  if (req.method === 'OPTIONS') { res.writeHead(204, cors); return res.end(); }
  if (req.method !== 'POST') { res.writeHead(405, cors); return res.end(); }
  const chunks = [];
  req.on('data', c => chunks.push(c));
  req.on('end', async () => {
    try {
      const r = await fetch(target, { method: 'POST', headers: { 'content-type': 'application/json' }, body: Buffer.concat(chunks) });
      const body = Buffer.from(await r.arrayBuffer());
      res.writeHead(r.status, { ...cors, 'content-type': r.headers.get('content-type') || 'application/json' });
      res.end(body);
    } catch (e) {
      res.writeHead(502, cors);
      res.end(JSON.stringify({ error: String(e) }));
    }
  });
}).listen(port, '127.0.0.1', () => console.log(`CORS proxy on http://127.0.0.1:${port} -> ${target}`));
