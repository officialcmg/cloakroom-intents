/// <reference lib="webworker" />
// Worker entry of the real engine (crates/zecret-wasm behind the Engine interface).
import { serveEngine } from '../rpc';
import { openRecords } from './idb';
import { RealEngine } from './realEngine';

const scope = self as unknown as DedicatedWorkerGlobalScope;

// Calls that arrive while the store opens wait in a buffer, so none is lost.
const early: MessageEvent[] = [];
const buffer = (e: MessageEvent) => early.push(e);
scope.addEventListener('message', buffer);

openRecords().then((store) => {
  serveEngine(scope, new RealEngine(store, import.meta.env.BASE_URL));
  scope.removeEventListener('message', buffer);
  for (const e of early) scope.dispatchEvent(new MessageEvent('message', { data: e.data }));
});
