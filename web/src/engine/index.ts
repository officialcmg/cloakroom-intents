// The one place that picks the engine. To switch to the real engine, point the marked
// line at the real engine's worker entry. Nothing else on the page changes: the worker
// says which engine it is in its InitStatus.
import { connectEngine, type EngineClient } from './rpc';

export function createEngine(): EngineClient {
  const worker = new Worker(new URL('./sim/sim.worker.ts', import.meta.url), { type: 'module' }); // <- engine worker
  return connectEngine(worker, () => worker.terminate());
}

export * from './api';
export type { EngineClient } from './rpc';
