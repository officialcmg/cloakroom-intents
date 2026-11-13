// The one place that picks the engine. The page runs the real engine (WebAssembly, in its
// own worker) when it has a network configuration (src/lib/netconfig.ts), and the
// simulated engine otherwise (the demo). Nothing else on the page depends on which one
// runs: the worker says which engine it is in its InitStatus.
import { connectEngine, type EngineClient } from './rpc';

export type EngineChoice = 'real' | 'simulated';

export function createEngine(choice: EngineChoice = 'simulated'): EngineClient {
  const worker =
    choice === 'real'
      ? new Worker(new URL('./real/real.worker.ts', import.meta.url), { type: 'module' })
      : new Worker(new URL('./sim/sim.worker.ts', import.meta.url), { type: 'module' });
  return connectEngine(worker, () => worker.terminate());
}

export * from './api';
export type { EngineClient } from './rpc';
