// The page: talks to the wallet worker by messages and shows what it reports.
const worker = new Worker('./worker.js', { type: 'module' });
let next = 1;
const pending = new Map();
worker.onmessage = (e) => {
  const p = pending.get(e.data.id);
  pending.delete(e.data.id);
  if (e.data.ok) p.res({ value: e.data.value, ms: e.data.ms });
  else p.rej(new Error(e.data.error));
};
worker.onerror = (e) => log('worker error: ' + e.message);

function call(cmd, ...args) {
  const id = next++;
  return new Promise((res, rej) => {
    pending.set(id, { res, rej });
    worker.postMessage({ id, cmd, args });
  });
}

function log(s) {
  const el = document.getElementById('log');
  el.textContent += s + '\n';
  console.log('PAGE ' + s);
}
function show(id, s) { document.getElementById(id).textContent = s; }

// Wrapped so every call is also shown on the page.
async function engine(cmd, ...args) {
  const r = await call(cmd, ...args);
  log(`${cmd}: ${JSON.stringify(r.value).slice(0, 300)} (${(r.ms / 1000).toFixed(2)} s)`);
  if (cmd === 'create') show('addr', r.value.address);
  if (cmd === 'balance') show('bal', r.value + ' zats');
  if (cmd === 'notes') {
    const n = r.value.filter(x => !x.spent_in).concat(r.value.filter(x => x.spent_in));
    show('memo', n.length ? n.map(x => `${x.zats} zats at height ${x.height}${x.spent_in ? ' (spent)' : ''}: ${x.memo.slice(0, 120)}`).join('\n') : '-');
  }
  return r;
}
window.engine = engine;

// Manual use: the query string selects the backend, e.g. ?rpc=http://127.0.0.1:18300
const q = new URLSearchParams(location.search);
document.getElementById('go').onclick = async () => {
  try {
    await engine('create', { backend: 'zebra', url: q.get('rpc') || 'http://127.0.0.1:18300', network: 'regtest', nu7: 2, restore: false });
    await engine('sync'); await engine('balance'); await engine('notes');
  } catch (e) { log('error: ' + e.message); }
};
document.getElementById('send').onclick = async () => {
  try {
    const memo = Array.from(new TextEncoder().encode(document.getElementById('m').value)).slice(0, 512);
    await engine('pay', document.getElementById('to').value, Number(document.getElementById('z').value), memo);
  } catch (e) { log('error: ' + e.message); }
};
document.getElementById('refresh').onclick = async () => {
  try { await engine('sync'); await engine('balance'); await engine('notes'); } catch (e) { log('error: ' + e.message); }
};
