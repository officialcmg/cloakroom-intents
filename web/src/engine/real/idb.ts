// The real engine's records in IndexedDB: one object store of UTF-8 text values. Keys are
// the core's records (`wallet`, `swap/<id>`) and the worker's own (`web/state`, `web/meta`).
// A write resolves only when its transaction has committed, so a caller that awaits it
// knows the records are durable before it does anything that depends on them.
//
// Version guard (security review 3, NV3-02): `web/version` counts writes. A store reads it
// when it opens; every write and clear checks, in the same transaction, that the stored
// count is still the one this store last saw, and adds one. A tab whose copy is older (the
// other tab wrote since) gets StaleWriteError and writes nothing.

const DB_NAME = 'zecret-engine';
const STORE = 'records';
export const VERSION_KEY = 'web/version';

/** Another store (another tab) wrote since this one last read or wrote. Nothing was written. */
export class StaleWriteError extends Error {
  constructor(stored: number, known: number) {
    super(`another tab changed this swap (version ${stored}, this tab has ${known})`);
    this.name = 'StaleWriteError';
  }
}

export interface RecordStore {
  readonly persistent: boolean;
  all(): Promise<[string, string][]>;
  /** Writes all entries in one transaction; resolves when it has committed. */
  putAll(entries: [string, string][]): Promise<void>;
  clear(): Promise<void>;
}

function done(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error ?? new Error('IndexedDB transaction failed'));
    tx.onabort = () => reject(tx.error ?? new Error('IndexedDB transaction aborted'));
  });
}

export async function openRecords(name = DB_NAME): Promise<RecordStore> {
  if (typeof indexedDB === 'undefined') return memoryRecords();
  let db: IDBDatabase;
  try {
    db = await new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open(name, 1);
      r.onupgradeneeded = () => {
        if (!r.result.objectStoreNames.contains(STORE)) r.result.createObjectStore(STORE);
      };
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
  } catch {
    return memoryRecords();
  }
  const readVersion = async () => {
    const tx = db.transaction(STORE, 'readonly');
    const r = tx.objectStore(STORE).get(VERSION_KEY);
    await done(tx);
    return Number(r.result ?? 0);
  };
  let known = await readVersion();
  /** Runs `write` in one transaction after the version check; adds one to the version. */
  const guarded = async (write: (os: IDBObjectStore) => void) => {
    const tx = db.transaction(STORE, 'readwrite', { durability: 'strict' });
    const os = tx.objectStore(STORE);
    let stale: StaleWriteError | undefined;
    const r = os.get(VERSION_KEY);
    r.onsuccess = () => {
      const stored = Number(r.result ?? 0);
      if (stored !== known) {
        stale = new StaleWriteError(stored, known);
        tx.abort();
        return;
      }
      write(os);
      os.put(String(known + 1), VERSION_KEY);
    };
    try {
      await done(tx);
    } catch (e) {
      throw stale ?? e;
    }
    known += 1;
  };
  return {
    persistent: true,
    async all() {
      const tx = db.transaction(STORE, 'readonly');
      const os = tx.objectStore(STORE);
      const keys = os.getAllKeys();
      const values = os.getAll();
      await done(tx);
      const out = (keys.result as IDBValidKey[]).map((k, i) => [String(k), String(values.result[i])] as [string, string]);
      known = Number(out.find(([k]) => k === VERSION_KEY)?.[1] ?? 0);
      return out.filter(([k]) => k !== VERSION_KEY);
    },
    async putAll(entries) {
      if (entries.length === 0) return;
      await guarded((os) => {
        for (const [k, v] of entries) os.put(v, k);
      });
    },
    async clear() {
      await guarded((os) => os.clear());
    },
  };
}

/**
 * When the browser refuses IndexedDB: the swap lasts as long as the tab. `shared` lets two
 * stores share one map (tests of the version guard).
 */
export function memoryRecords(shared?: Map<string, string>): RecordStore {
  const m = shared ?? new Map<string, string>();
  let known = Number(m.get(VERSION_KEY) ?? 0);
  const guarded = (write: () => void) => {
    const stored = Number(m.get(VERSION_KEY) ?? 0);
    if (stored !== known) throw new StaleWriteError(stored, known);
    write();
    known += 1;
    m.set(VERSION_KEY, String(known));
  };
  return {
    persistent: false,
    async all() {
      known = Number(m.get(VERSION_KEY) ?? 0);
      return [...m.entries()].filter(([k]) => k !== VERSION_KEY);
    },
    async putAll(entries) {
      if (entries.length === 0) return;
      guarded(() => {
        for (const [k, v] of entries) m.set(k, v);
      });
    },
    async clear() {
      guarded(() => m.clear());
    },
  };
}
