// Boots the page: reads the maker directory, starts the engine in its worker, resumes any
// stored swap, and keeps a live copy of the swap by folding engine events.
import { useEffect, useSyncExternalStore } from 'react';
import { createEngine, type EngineClient, type EngineErrorData, type InitStatus, type SwapState } from '../engine';
import { reduceSwap } from '../engine/reducer';
import { directoryUrl, effectiveTerms, fetchDirectory, type MakerDirectory, type MakerTerms } from '../lib/directory';
import { chooseConfig, fetchNetworkConfig, siteCapZats, type ConfigChoice, type NetworkConfig } from '../lib/netconfig';
import { readSettings, type PageSettings } from './settings';
import { setPayToken } from './stageCopy';

export interface AppState {
  /** `other_tab`: another tab of this browser runs the engine; this tab waits (review 3, NV3-02). */
  phase: 'loading' | 'ready' | 'failed' | 'other_tab';
  failure?: { kind: 'directory' | 'engine' | 'config'; message: string };
  /** The real engine's network, when the page runs one (undefined in the demo). */
  network?: NetworkConfig;
  directory?: MakerDirectory;
  terms?: MakerTerms;
  init?: InitStatus;
  swap: SwapState | null;
  /** Problems the engine reported that are not tied to one call (for example no storage). */
  warnings: EngineErrorData[];
  /** Set when ?config= named a file other than the shipped ones (development builds only). */
  configOverride?: ConfigChoice;
  /** Set after the page restarted a crashed engine. */
  restartedAt?: number;
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
    this.releaseLock?.();
    this.releaseLock = undefined;
    this.booting = undefined;
    this.set({ phase: 'loading', failure: undefined });
    void this.boot();
  }

  private releaseLock?: () => void;
  private restarts: number[] = [];

  private async run(): Promise<void> {
    let directory: MakerDirectory;
    let terms: MakerTerms;
    let network: NetworkConfig | undefined;
    let choice: ConfigChoice | undefined;
    try {
      choice = chooseConfig();
      // The warning shows from the start, also when the file or its directory fails to load.
      this.set({ configOverride: choice?.source === 'override' ? choice : undefined });
      if (choice) network = await fetchNetworkConfig(choice.url);
    } catch (e) {
      this.set({ phase: 'failed', failure: { kind: 'config', message: (e as Error).message } });
      return;
    }
    try {
      // With a network configuration, its own directory: ?directory= is for the demo only (review 3, NV3-01).
      ({ directory, terms } = await fetchDirectory(network ? network.directoryUrl : directoryUrl()));
    } catch (e) {
      this.set({ phase: 'failed', failure: { kind: 'directory', message: (e as Error).message } });
      return;
    }
    // One payment range for the form, the notices and the engine (effectiveTerms).
    terms = effectiveTerms(terms, directory, siteCapZats(network));
    if (network) setPayToken(network.near.tokenSymbol === 'USDC' ? 'USDC' : network.near.tokenSymbol);
    this.set({ directory, terms, network });
    // One tab runs the engine of this browser at a time (review 3, NV3-02). A second tab
    // waits here and takes over by itself when the first one closes.
    const lockName = network ? 'zecret-engine' : 'zecret-engine-demo';
    let release = await takeLock(lockName, false);
    if (!release) {
      this.set({ phase: 'other_tab' });
      release = await takeLock(lockName, true);
      this.set({ phase: 'loading' });
    }
    this.releaseLock = release;
    await this.startEngine(network, directory, terms);
  }

  private async startEngine(network: NetworkConfig | undefined, directory: MakerDirectory, terms: MakerTerms): Promise<void> {
    const engine = createEngine(network ? 'real' : 'simulated');
    this.engine = engine;
    engine.subscribe((ev) => {
      if (this.engine !== engine) return;
      if (ev.type === 'init') this.set({ init: ev.init });
      else if (ev.type === 'error' && !ev.swapId) {
        if (ev.error.code === 'ENGINE_CRASHED') return void this.restart(network, directory, terms, ev.error.message);
        if (ev.error.code === 'OTHER_TAB') {
          engine.dispose();
          this.engine = undefined;
          return this.set({ phase: 'other_tab', failure: { kind: 'engine', message: ev.error.message } });
        }
        this.set({ warnings: [...this.state.warnings.filter((w) => w.message !== ev.error.message), ev.error] });
      } else if ((ev.type === 'stage' || ev.type === 'swap') && this.state.warnings.some((w) => w.code === 'STORAGE' && w.transient)) {
        // Every event is stored before it is sent: storage works again.
        this.set({ warnings: this.state.warnings.filter((w) => !(w.code === 'STORAGE' && w.transient)) });
      }
      const swap = reduceSwap(this.state.swap, ev);
      if (swap !== this.state.swap) this.set({ swap });
    });
    try {
      const init = await engine.init({ network: terms.network, directory, sim: network ? undefined : this.state.settings.sim, real: network });
      if (this.engine !== engine) return;
      this.set({ init });
      const swap = await engine.resume();
      if (this.engine !== engine) return;
      this.set({ swap, phase: 'ready' });
    } catch (e) {
      if (this.engine !== engine) return;
      this.set({ phase: 'failed', failure: { kind: 'engine', message: (e as Error).message } });
    }
  }

  /**
   * The WebAssembly engine trapped (review 3, H6): a trapped instance must not be called
   * again. The page ends its worker and starts a new one, which resumes from the stored
   * swap. At most three restarts in ten minutes; then the page shows the failure.
   */
  private restart(network: NetworkConfig | undefined, directory: MakerDirectory, terms: MakerTerms, message: string): void {
    this.engine?.dispose();
    this.engine = undefined;
    const t = Date.now();
    this.restarts = [...this.restarts.filter((r) => t - r < 600_000), t];
    if (this.restarts.length > 3) {
      this.set({ phase: 'failed', failure: { kind: 'engine', message: `The swap engine stopped again: ${message}` } });
      return;
    }
    this.set({ restartedAt: t });
    void this.startEngine(network, directory, terms);
  }
}

/**
 * Takes the Web Lock `name` for this tab until the returned function is called. Without
 * `wait`, resolves to undefined when another tab holds it. Browsers without Web Locks get a
 * no-op lock (the real engine's version check still refuses an older tab's writes).
 */
function takeLock(name: string, wait: boolean): Promise<(() => void) | undefined> {
  const locks = typeof navigator !== 'undefined' ? navigator.locks : undefined;
  if (!locks) return Promise.resolve(() => undefined);
  return new Promise((resolve) => {
    locks
      .request(name, { ifAvailable: !wait }, (lock) => {
        if (!lock) {
          resolve(undefined);
          return undefined;
        }
        return new Promise<void>((release) => resolve(() => release()));
      })
      .catch(() => resolve(() => undefined));
  });
}

export const appStore = new Store();
// Development builds only: lets screenshot scripts show states that are hard to cause (a
// failing disk, a crashed engine). Not in production builds.
if (import.meta.env.DEV) (globalThis as { zecretStore?: Store }).zecretStore = appStore;

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

export { usePref } from './prefs';
