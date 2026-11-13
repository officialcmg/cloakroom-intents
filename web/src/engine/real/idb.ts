// The real engine's records in IndexedDB: one object store of UTF-8 text values. Keys are
// the core's records (`wallet`, `swap/<id>`) and the worker's own (`web/state`, `web/meta`).
// A write resolves only when its transaction has committed, so a caller that awaits it
// knows the records are durable before it does anything that depends on them.

const DB_NAME = 'zecret-engine';
const STORE = 'records';

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
  return {
    persistent: true,
    async all() {
      const tx = db.transaction(STORE, 'readonly');
      const os = tx.objectStore(STORE);
      const keys = os.getAllKeys();
      const values = os.getAll();
      await done(tx);
      return (keys.result as IDBValidKey[]).map((k, i) => [String(k), String(values.result[i])] as [string, string]);
    },
    async putAll(entries) {
      if (entries.length === 0) return;
      const tx = db.transaction(STORE, 'readwrite', { durability: 'strict' });
      const os = tx.objectStore(STORE);
      for (const [k, v] of entries) os.put(v, k);
      await done(tx);
    },
    async clear() {
      const tx = db.transaction(STORE, 'readwrite', { durability: 'strict' });
      tx.objectStore(STORE).clear();
      await done(tx);
    },
  };
}

/** When the browser refuses IndexedDB: the swap lasts as long as the tab. */
export function memoryRecords(): RecordStore {
  const m = new Map<string, string>();
  return {
    persistent: false,
    async all() {
      return [...m.entries()];
    },
    async putAll(entries) {
      for (const [k, v] of entries) m.set(k, v);
    },
    async clear() {
      m.clear();
    },
  };
}
