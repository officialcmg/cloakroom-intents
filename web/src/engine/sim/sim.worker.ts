/// <reference lib="webworker" />
// Worker entry of the simulated engine.
import { serveEngine } from '../rpc';
import { SimEngine } from './simEngine';
import { openStore } from './store';

const scope = self as unknown as DedicatedWorkerGlobalScope;

// Calls that arrive while the store opens wait in a buffer, so none is lost.
const early: MessageEvent[] = [];
const buffer = (e: MessageEvent) => early.push(e);
scope.addEventListener('message', buffer);

openStore().then((store) => {
  serveEngine(scope, new SimEngine(store));
  scope.removeEventListener('message', buffer);
  for (const e of early) scope.dispatchEvent(new MessageEvent('message', { data: e.data }));
});
