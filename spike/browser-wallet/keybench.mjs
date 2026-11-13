// Proving key cost across page loads, in headless Chromium: wasm instantiation, the key
// build, and how much of it is the Halo 2 parameters (which can be serialized).
//   node keybench.mjs [loads=2]
import { spawn } from 'node:child_process';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const HERE = path.dirname(fileURLToPath(import.meta.url));
const LOADS = Number(process.argv[2] || 2);
const srv = spawn(process.execPath, [path.join(HERE, 'serve.mjs'), '18382'], { stdio: ['ignore', 'pipe', 'inherit'] });
await new Promise(r => srv.stdout.on('data', d => String(d).includes('serving') && r()));
const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto('http://127.0.0.1:18382/');
const E = (cmd, ...args) => page.evaluate(([c, a]) => window.engine(c, ...a), [cmd, args]);
for (let i = 1; i <= LOADS; i++) {
  if (i > 1) await page.reload();
  const inst = (await E('ready')).value.instantiate_ms;
  const key = (await E('prepareKey')).value;
  const again = (await E('prepareKey')).value;
  console.log(`load ${i}: wasm instantiate ${inst.toFixed(0)} ms, proving key build ${(key / 1000).toFixed(1)} s, second call in the same worker ${again.toFixed(0)} ms`);
}
const pb = (await E('paramsBench')).value;
console.log(`Halo 2 params (k = 11): Params::new ${(pb.params_new_ms / 1000).toFixed(1)} s, Params::read from ${pb.params_bytes} bytes ${pb.params_read_ms.toFixed(0)} ms, round trip equal: ${pb.round_trip_equal}`);
await browser.close();
srv.kill();
