// Boots the page: reads the maker directory, starts the engine in its worker, resumes any
// stored swap, and keeps a live copy of the swap by folding engine events.
import { useEffect, useState, useSyncExternalStore } from 'react';
import { createEngine, type EngineClient, type EngineErrorData, type InitStatus, type SwapState } from '../engine';
import { reduceSwap } from '../engine/reducer';
import { directoryUrl, fetchDirectory, type MakerDirectory, type MakerTerms } from '../lib/directory';
import { configUrl, fetchNetworkConfig, type NetworkConfig } from '../lib/netconfig';
import { readSettings, type PageSettings } from './settings';
import { setPayToken } from './stageCopy';

export interface AppState {
  phase: 'loading' | 'ready' | 'failed';
  failure?: { kind: 'directory' | 'engine' | 'config'; message: string };
  /** The real engine's network, when the page runs one (undefined in the demo). */
  network?: NetworkConfig;
  directory?: MakerDirectory;
  terms?: MakerTerms;
  init?: InitStatus;
  swap: SwapState | null;
  /** Problems the engine reported that are not tied to one call (for example no storage). */
  warnings: EngineErrorData[];
  settings: PageSettings;
}

type Listener = () => void;

class Store {
  state: AppState = { phase: 'loading', swap: null, warnings: [], settings: readSettings() };
  engine?: EngineClient;
  private listeners = new Set<Listener>();
  private booting?: Promise<void>;

  subscribe = (l: Listener) => {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  };
  get = () => this.state;
  set(patch: Partial<AppState>) {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }

  boot(): Promise<void> {
    if (!this.booting) this.booting = this.run();
    return this.booting;
  }

  /** Starts over after a failure, for the "Try again" button. */
  retry(): void {
    this.engine?.dispose();
    this.engine = undefined;
    this.booting = undefined;
    this.set({ phase: 'loading', failure: undefined });
    void this.boot();
  }

  private async run(): Promise<void> {
    let directory: MakerDirectory;
    let terms: MakerTerms;
    let network: NetworkConfig | undefined;
    const cfgUrl = configUrl();
    if (cfgUrl) {
      try {
        network = await fetchNetworkConfig(cfgUrl);
      } catch (e) {
        this.set({ phase: 'failed', failure: { kind: 'config', message: (e as Error).message } });
        return;
      }
    }
    try {
      const fromQuery = new URLSearchParams(location.search).get('directory');
      ({ directory, terms } = await fetchDirectory(network && !fromQuery ? network.directoryUrl : directoryUrl()));
    } catch (e) {
      this.set({ phase: 'failed', failure: { kind: 'directory', message: (e as Error).message } });
      return;
    }
    if (network) setPayToken(network.near.tokenSymbol === 'USDC' ? 'USDC' : network.near.tokenSymbol);
    this.set({ directory, terms, network });
    const engine = createEngine(network ? 'real' : 'simulated');
    this.engine = engine;
    engine.subscribe((ev) => {
      if (ev.type === 'init') this.set({ init: ev.init });
      else if (ev.type === 'error' && !ev.swapId) this.set({ warnings: [...this.state.warnings, ev.error] });
      const swap = reduceSwap(this.state.swap, ev);
      if (swap !== this.state.swap) this.set({ swap });
    });
    try {
      const init = await engine.init({ network: terms.network, directory, sim: network ? undefined : this.state.settings.sim, real: network });
      this.set({ init });
      const swap = await engine.resume();
      this.set({ swap, phase: 'ready' });
    } catch (e) {
      this.set({ phase: 'failed', failure: { kind: 'engine', message: (e as Error).message } });
    }
  }
}

export const appStore = new Store();

export function useApp(): AppState {
  const state = useSyncExternalStore(appStore.subscribe, appStore.get);
  useEffect(() => {
    void appStore.boot();
  }, []);
  return state;
}

export function useEngine(): EngineClient | undefined {
  useApp();
  return appStore.engine;
}

/** Local UI preference that survives reloads (theme, last token). */
export function usePref<T extends string>(key: string, initial: T): [T, (v: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      return (localStorage.getItem(`zecret.${key}`) as T | null) ?? initial;
    } catch {
      return initial;
    }
  });
  const set = (v: T) => {
    setValue(v);
    try {
      localStorage.setItem(`zecret.${key}`, v);
    } catch {
      // Storage refused: the preference lasts for this visit only.
    }
  };
  return [value, set];
}
