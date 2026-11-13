// Carries the Engine interface over any postMessage port: a Worker on the page side, the
// worker's global scope on the other side, or a MessageChannel in tests.
import {
  EngineError,
  type Engine,
  type EngineEvent,
  type EngineErrorData,
  type EngineMethod,
  type SimControls,
  type SimMethod,
} from './api';

export interface PortLike {
  postMessage(message: unknown): void;
  addEventListener(type: 'message', listener: (event: MessageEvent) => void): void;
  removeEventListener(type: 'message', listener: (event: MessageEvent) => void): void;
}

type Method = EngineMethod | SimMethod;

export type WireMessage =
  | { kind: 'call'; id: number; method: Method; args: unknown[] }
  | { kind: 'result'; id: number; ok: true; value: unknown }
  | { kind: 'result'; id: number; ok: false; error: EngineErrorData }
  | { kind: 'event'; event: EngineEvent };

/** What a worker implements. The real engine's worker implements the same object. */
export type EngineBackend = Omit<Engine, 'subscribe' | 'dispose'> & {
  /** Called once by `serveEngine`; the backend calls `emit` for every event. */
  attach(emit: (event: EngineEvent) => void): void;
  sim?: SimControls;
};

const ENGINE_METHODS: readonly EngineMethod[] = [
  'init',
  'resume',
  'getState',
  'createSwap',
  'setReturnAddress',
  'exportRecovery',
  'importRecovery',
  'exit',
  'forget',
];

function toErrorData(e: unknown): EngineErrorData {
  if (e instanceof EngineError) return e.toData();
  if (e && typeof e === 'object' && 'code' in e && 'message' in e) return e as EngineErrorData;
  return { code: 'INTERNAL', message: e instanceof Error ? e.message : String(e) };
}

/** Worker side: answers calls from the page and forwards events. */
export function serveEngine(port: PortLike, backend: EngineBackend): void {
  backend.attach((event) => port.postMessage({ kind: 'event', event } satisfies WireMessage));
  port.addEventListener('message', (ev: MessageEvent) => {
    const msg = ev.data as WireMessage;
    if (!msg || msg.kind !== 'call') return;
    const reply = (m: WireMessage) => port.postMessage(m);
    const run = async () => {
      let fn: ((...a: unknown[]) => Promise<unknown>) | undefined;
      if (msg.method.startsWith('sim.')) {
        const name = msg.method.slice(4) as keyof SimControls;
        const sim = backend.sim;
        fn = sim && typeof sim[name] === 'function' ? (sim[name] as (...a: unknown[]) => Promise<unknown>).bind(sim) : undefined;
      } else if ((ENGINE_METHODS as readonly string[]).includes(msg.method)) {
        const m = backend[msg.method as EngineMethod] as (...a: unknown[]) => Promise<unknown>;
        fn = m.bind(backend);
      }
      if (!fn) throw new EngineError('INVALID_PARAMS', `Unknown method ${msg.method}`);
      return fn(...msg.args);
    };
    run().then(
      (value) => reply({ kind: 'result', id: msg.id, ok: true, value }),
      (e) => reply({ kind: 'result', id: msg.id, ok: false, error: toErrorData(e) }),
    );
  });
}

export interface EngineClient extends Engine {
  /** Only the simulated engine answers these; the real one rejects them. */
  sim: SimControls;
}

/** Page side: an Engine whose calls go over the port. */
export function connectEngine(port: PortLike, onDispose?: () => void): EngineClient {
  let nextId = 1;
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  const listeners = new Set<(e: EngineEvent) => void>();

  const onMessage = (ev: MessageEvent) => {
    const msg = ev.data as WireMessage;
    if (!msg) return;
    if (msg.kind === 'event') {
      for (const l of [...listeners]) l(msg.event);
    } else if (msg.kind === 'result') {
      const p = pending.get(msg.id);
      if (!p) return;
      pending.delete(msg.id);
      if (msg.ok) p.resolve(msg.value);
      else p.reject(EngineError.from(msg.error));
    }
  };
  port.addEventListener('message', onMessage);

  const call = <T>(method: Method, ...args: unknown[]): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      port.postMessage({ kind: 'call', id, method, args } satisfies WireMessage);
    });

  const client: EngineClient = {
    init: (config) => call('init', config),
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    resume: () => call('resume'),
    getState: () => call('getState'),
    createSwap: (params) => call('createSwap', params),
    setReturnAddress: (address) => call('setReturnAddress', address),
    exportRecovery: () => call('exportRecovery'),
    importRecovery: (text) => call('importRecovery', text),
    exit: (request) => call('exit', request),
    forget: () => call('forget'),
    dispose() {
      port.removeEventListener('message', onMessage);
      for (const p of pending.values()) p.reject(new EngineError('INTERNAL', 'The engine was closed.'));
      pending.clear();
      listeners.clear();
      onDispose?.();
    },
    sim: {
      pay: (amountZats) => call('sim.pay', amountZats),
      skip: () => call('sim.skip'),
      clearLocal: () => call('sim.clearLocal'),
    },
  };
  return client;
}
