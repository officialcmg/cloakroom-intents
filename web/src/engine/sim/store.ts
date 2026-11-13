// A small key-value store on IndexedDB, with an in-memory fallback when the browser
// refuses storage (some private modes).

export interface KvStore {
  get<T>(store: StoreName, key: string): Promise<T | undefined>;
  put(store: StoreName, key: string, value: unknown): Promise<void>;
  delete(store: StoreName, key: string): Promise<void>;
  readonly persistent: boolean;
}

/** `local` holds this browser's keys; `chain` stands in for what the networks know. */
export type StoreName = 'local' | 'chain';

const DB_NAME = 'zecret-sim';

function req<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

export async function openStore(): Promise<KvStore> {
  try {
    if (typeof indexedDB === 'undefined') throw new Error('no IndexedDB');
    const open = indexedDB.open(DB_NAME, 1);
    open.onupgradeneeded = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains('local')) db.createObjectStore('local');
      if (!db.objectStoreNames.contains('chain')) db.createObjectStore('chain');
    };
    const db = await req(open);
    const tx = (store: StoreName, mode: IDBTransactionMode) => db.transaction(store, mode).objectStore(store);
    return {
      persistent: true,
      async get<T>(store: StoreName, key: string) {
        return (await req(tx(store, 'readonly').get(key))) as T | undefined;
      },
      async put(store, key, value) {
        await req(tx(store, 'readwrite').put(value, key));
      },
      async delete(store, key) {
        await req(tx(store, 'readwrite').delete(key));
      },
    };
  } catch {
    return memoryStore();
  }
}

export function memoryStore(): KvStore {
  const maps: Record<StoreName, Map<string, unknown>> = { local: new Map(), chain: new Map() };
  return {
    persistent: false,
    async get<T>(store: StoreName, key: string) {
      const v = maps[store].get(key);
      return v === undefined ? undefined : (structuredClone(v) as T);
    },
    async put(store, key, value) {
      maps[store].set(key, structuredClone(value));
    },
    async delete(store, key) {
      maps[store].delete(key);
    },
  };
}
