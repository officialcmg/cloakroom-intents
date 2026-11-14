// The network configuration of the real engine: which Zcash and NEAR endpoints it uses,
// the escrow it trusts, the token and the Verifier, and how the exit runs. It is a small
// public JSON file, read at page load. Without one the page runs the simulated engine
// (the demo).
//
// Where it comes from, first match wins:
//   ?demo=1                      the simulated engine, whatever else is set
//   ?config=<name>               a shipped configuration: /config/<name>.json, for the
//                                names in KNOWN_CONFIGS only
//   ?config=<other name or URL>  only in a development build (`vite`), or a build made
//                                with VITE_ZECRET_ALLOW_CONFIG_URL=1; the page then shows
//                                a warning that names the file (security review 3, NV3-01)
//   VITE_ZECRET_CONFIG           a name or URL, at build time
// Shipped files: public/config/local.json (Zebra regtest through a CORS proxy and a NEAR
// sandbox) and public/config/testnet.json (Zcash testnet over gRPC-web, NEAR testnet).
//
// A mainnet configuration must use https: endpoints, and its escrow, escrow code hash,
// token and Verifier must equal MAINNET_ANCHORS, which are compiled into the bundle. The
// shipped public/config/mainnet.json says "@build" for the escrow and its code hash: they
// are always the bundle's.
//
// Mainnet is off unless the build sets VITE_ZECRET_ALLOW_MAINNET=1 (docs/MAINNET.md). Without
// it, ?config=mainnet, VITE_ZECRET_CONFIG=mainnet and any file with "network": "mainnet" are
// refused, so the testnet site cannot be pointed at mainnet by a link.
import type { ZcashNetwork } from './address/zcash';

/** The configurations shipped in public/config/ that a link may name in every build. */
export const KNOWN_CONFIGS: readonly string[] = ['local', 'testnet'];

/** The shipped mainnet configuration; a name that works only in a mainnet build. */
export const MAINNET_CONFIG = 'mainnet';

/** Whether this build may run on mainnet: only a build made with VITE_ZECRET_ALLOW_MAINNET=1. */
export const MAINNET_ALLOWED: boolean = import.meta.env.VITE_ZECRET_ALLOW_MAINNET === '1';

/** The NEAR Intents 1Click API, called from the browser with no key. */
export const ONECLICK_DIRECT_URL = 'https://1click.chaindefuser.com';

/** In the shipped mainnet file: take this value from the bundle's MAINNET_ANCHORS. */
export const FROM_BUILD = '@build';

/** The configuration names a link may use in this build. */
export function knownConfigs(allowMainnet = MAINNET_ALLOWED): readonly string[] {
  return allowMainnet ? [...KNOWN_CONFIGS, MAINNET_CONFIG] : KNOWN_CONFIGS;
}

/** A build made for mainnet: mainnet allowed and VITE_ZECRET_CONFIG=mainnet. The landing page uses it. */
export const MAINNET_BUILD: boolean = MAINNET_ALLOWED && import.meta.env.VITE_ZECRET_CONFIG === MAINNET_CONFIG;

/**
 * Whether `?config=` may name a file other than KNOWN_CONFIGS: in a development build, or
 * a build made with VITE_ZECRET_ALLOW_CONFIG_URL=1 (the browser scenario runner).
 */
export const CONFIG_OVERRIDE_ALLOWED: boolean = import.meta.env.DEV === true || import.meta.env.VITE_ZECRET_ALLOW_CONFIG_URL === '1';

export interface MainnetAnchors {
  escrow: string;
  escrowCodeHash: string;
  token: string;
  verifier: string;
}

/**
 * The mainnet contracts this bundle trusts. Token and Verifier are Circle's USDC on NEAR
 * and intents.near. No mainnet escrow is deployed yet: the escrow and its code hash come
 * from VITE_MAINNET_ESCROW and VITE_MAINNET_ESCROW_CODE_HASH at build time, and without
 * them the page refuses every mainnet configuration.
 */
export const MAINNET_ANCHORS: MainnetAnchors = {
  escrow: import.meta.env.VITE_MAINNET_ESCROW ?? '',
  escrowCodeHash: import.meta.env.VITE_MAINNET_ESCROW_CODE_HASH ?? '',
  token: '17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1',
  verifier: 'intents.near',
};

export interface Timing {
  inclusion_secs: number;
  finality_secs: number;
  reorg_budget: number;
  margin_secs?: number;
  fund_secs: number;
  k_z: number;
}

export interface NetworkConfig {
  format: 'zecret-config';
  version: 1;
  network: ZcashNetwork;
  /** Shown to the user: what this network is. */
  label: string;
  /** The maker directory for this network. */
  directoryUrl: string;
  zcash: {
    /** Zebra JSON-RPC (regtest, through a CORS proxy). */
    zebraRpc?: string;
    /** lightwalletd gRPC-web base URL (testnet, mainnet). */
    lightwalletd?: string;
    /** Regtest only: NU7 activation height. */
    nu7Height?: number;
    /** Transaction explorer, `{txid}` is replaced. */
    explorer?: string;
  };
  near: {
    /**
     * The first NEAR RPC endpoint. The file may give one URL or an ordered list; the
     * engine uses the first that answers and moves to the next on a rate limit, a server
     * error or a timeout. A second endpoint is a fallback, not a second opinion.
     */
    rpc: string;
    /** Every NEAR RPC endpoint, in order (set by `parseNetworkConfig`; `rpc` is the first). */
    rpcs?: string[];
    escrow: string;
    /** base58 sha256 of the reviewed escrow wasm, as view_account reports it. */
    escrowCodeHash: string;
    /** The NEP-141 token the maker pays (USDC on mainnet, a stand-in elsewhere). */
    token: string;
    tokenSymbol: string;
    /** At least 6. The page counts in micro-USDC; the engine scales by 10^(tokenDecimals - 6). */
    tokenDecimals: number;
    /** The Verifier (intents.near on mainnet, a copy of its code elsewhere). */
    verifier: string;
    /** Transaction explorer, `{hash}` is replaced. */
    explorer?: string;
  };
  timing: Timing;
  /** yoctoNEAR: the smallest stipend the engine accepts (0.03 NEAR: enough for commit, claim and withdraw at today's gas price). */
  minStipend: string;
  quoteTimeoutSecs?: number;
  maxHorizonSecs?: number;
  /**
   * Base URL of the 1Click API for the price preview and the 1Click exit: an http(s) URL,
   * or a path on this site such as /api/oneclick (the proxy that adds the partner key,
   * docs/ONECLICK_PROXY.md). Unset in the file: ONECLICK_DIRECT_URL. `parseNetworkConfig`
   * always sets it. Older files named it exit.oneClickUrl; that name still works.
   */
  oneClickUrl: string;
  exit:
    | { mode: 'oneclick' }
    | {
        /** No 1Click for this Verifier: the engine signs an ft_withdraw of the token to a NEAR account the user names. */
        mode: 'direct';
        /** The maker's relay that sends the signed intent with execute_intents (zecret maker --relay). */
        relayUrl: string;
      };
  /** Plain sentences on what is real on this network, shown on the page. */
  honesty?: string[];
}

export class NetConfigError extends Error {}

/** Every NEAR RPC endpoint of a configuration, in order. */
export function nearRpcs(c: Pick<NetworkConfig, 'near'>): string[] {
  return c.near.rpcs && c.near.rpcs.length > 0 ? c.near.rpcs : [c.near.rpc];
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown, p: string): string => {
  if (typeof v !== 'string' || !v) throw new NetConfigError(`${p} must be a non-empty string`);
  return v;
};

const isHttps = (u: unknown) => {
  try {
    return typeof u === 'string' && new URL(u).protocol === 'https:';
  } catch {
    return false;
  }
};
const isHttp = (u: unknown) => {
  try {
    return typeof u === 'string' && ['http:', 'https:'].includes(new URL(u).protocol);
  } catch {
    return false;
  }
};

/**
 * Validates a parsed configuration file. `anchors`: the mainnet contracts the bundle trusts.
 * `allowMainnet`: whether this build runs on mainnet at all (VITE_ZECRET_ALLOW_MAINNET).
 */
export function parseNetworkConfig(json: unknown, anchors: MainnetAnchors = MAINNET_ANCHORS, allowMainnet = MAINNET_ALLOWED): NetworkConfig {
  if (isObj(json) && json.network === 'mainnet') {
    if (!allowMainnet) throw new NetConfigError('This build of the page does not run on mainnet.');
    if (isObj(json.near)) {
      // "@build": the escrow and its code hash of this bundle. A copy, so the caller's object is not changed.
      const near = { ...json.near };
      if ((near.escrow === FROM_BUILD || near.escrowCodeHash === FROM_BUILD) && (!anchors.escrow || !anchors.escrowCodeHash))
        throw new NetConfigError('this build has no mainnet escrow built in, so it runs no mainnet swap');
      if (near.escrow === FROM_BUILD) near.escrow = anchors.escrow;
      if (near.escrowCodeHash === FROM_BUILD) near.escrowCodeHash = anchors.escrowCodeHash;
      json = { ...json, near };
    }
  }
  return parseFile(json, anchors);
}

function parseFile(json: unknown, anchors: MainnetAnchors): NetworkConfig {
  if (!isObj(json) || json.format !== 'zecret-config' || json.version !== 1) throw new NetConfigError('not a zecret-config version 1 file');
  const network = json.network;
  if (network !== 'mainnet' && network !== 'testnet' && network !== 'regtest') throw new NetConfigError('network must be mainnet, testnet or regtest');
  str(json.label, 'label');
  str(json.directoryUrl, 'directoryUrl');
  if (!isObj(json.zcash) || (!json.zcash.zebraRpc && !json.zcash.lightwalletd)) throw new NetConfigError('zcash needs zebraRpc or lightwalletd');
  if (json.zcash.lightwalletd && network === 'regtest') throw new NetConfigError('regtest uses zebraRpc');
  if (!isObj(json.near)) throw new NetConfigError('near is missing');
  const rpcs: unknown[] = Array.isArray(json.near.rpc) ? json.near.rpc : [json.near.rpc];
  if (rpcs.length === 0) throw new NetConfigError('near.rpc must name at least one URL');
  rpcs.forEach((u, i) => str(u, `near.rpc[${i}]`));
  if (new Set(rpcs).size !== rpcs.length) throw new NetConfigError('near.rpc names a URL twice');
  for (const k of ['escrow', 'escrowCodeHash', 'token', 'tokenSymbol', 'verifier']) str(json.near[k], `near.${k}`);
  const dec = json.near.tokenDecimals;
  if (typeof dec !== 'number' || !Number.isInteger(dec) || dec < 6 || dec > 30) throw new NetConfigError('near.tokenDecimals must be an integer from 6 to 30');
  if (!isObj(json.timing)) throw new NetConfigError('timing is missing');
  for (const k of ['inclusion_secs', 'finality_secs', 'reorg_budget', 'fund_secs', 'k_z']) {
    if (typeof json.timing[k] !== 'number') throw new NetConfigError(`timing.${k} must be a number`);
  }
  if (!/^\d+$/.test(String(json.minStipend))) throw new NetConfigError('minStipend must be a whole number of yoctoNEAR');
  if (!isObj(json.exit)) throw new NetConfigError('exit is missing');
  if (json.exit.mode === 'direct') str(json.exit.relayUrl, 'exit.relayUrl');
  else if (json.exit.mode !== 'oneclick') throw new NetConfigError('exit.mode must be oneclick or direct');
  if (network === 'mainnet' && json.exit.mode !== 'oneclick') throw new NetConfigError('mainnet exits through 1Click');
  // Explorer links become links on the page: https only (review 3, H3).
  for (const [k, v] of [['zcash.explorer', json.zcash.explorer], ['near.explorer', json.near.explorer]] as const) {
    if (v !== undefined && !isHttps(v)) throw new NetConfigError(`${k} must be an https: URL`);
  }
  rpcs.forEach((u, i) => {
    if (!isHttp(u)) throw new NetConfigError(`near.rpc[${i}] must be an http: or https: URL`);
  });
  for (const [k, v] of [['zcash.zebraRpc', json.zcash.zebraRpc], ['zcash.lightwalletd', json.zcash.lightwalletd], ['exit.relayUrl', json.exit.relayUrl]] as const) {
    if (v !== undefined && !isHttp(v)) throw new NetConfigError(`${k} must be an http: or https: URL`);
  }
  const oneClickUrl = parseOneClickUrl(json.oneClickUrl, json.exit.oneClickUrl);
  const exit = { ...json.exit };
  delete exit.oneClickUrl;
  // A copy with the endpoint list in `rpcs` and the first one in `rpc`.
  const parsed = { ...json, oneClickUrl, exit, near: { ...json.near, rpc: rpcs[0], rpcs } } as unknown as NetworkConfig;
  if (network === 'mainnet') checkMainnet(parsed, anchors);
  return parsed;
}

/** A path on this site: one leading slash, not two, no backslash. */
const isSitePath = (u: string) => u.startsWith('/') && !u.startsWith('//') && !u.includes('\\');

/** `oneClickUrl`, or the older exit.oneClickUrl; both may be given only with the same value. */
function parseOneClickUrl(top: unknown, legacy: unknown): string {
  if (top !== undefined && legacy !== undefined && top !== legacy) throw new NetConfigError('oneClickUrl and exit.oneClickUrl name different URLs');
  const v = top ?? legacy;
  if (v === undefined) return ONECLICK_DIRECT_URL;
  const u = str(v, 'oneClickUrl');
  if (!isSitePath(u) && !isHttp(u)) throw new NetConfigError('oneClickUrl must be an http: or https: URL or a path on this site');
  if (/[?#]/.test(u)) throw new NetConfigError('oneClickUrl must not have a query or a fragment');
  const base = u.replace(/\/+$/, '');
  if (!base) throw new NetConfigError('oneClickUrl must name a path, not only /');
  return base;
}

/**
 * The absolute 1Click base URL for this page: the configuration's oneClickUrl (a path is
 * taken on this page's origin), or ONECLICK_DIRECT_URL without a configuration (the demo).
 * The price preview and the engine's exit both use it.
 */
export function oneClickBase(c?: Pick<NetworkConfig, 'oneClickUrl'>, origin: string = typeof location === 'undefined' ? '' : location.origin): string {
  const u = c?.oneClickUrl || ONECLICK_DIRECT_URL;
  return (isSitePath(u) ? `${origin}${u}` : u).replace(/\/+$/, '');
}

/** Mainnet: https everywhere, and exactly the contracts compiled into the bundle (review 3, NV3-01). */
function checkMainnet(c: NetworkConfig, anchors: MainnetAnchors): void {
  if (c.zcash.zebraRpc) throw new NetConfigError('mainnet uses lightwalletd, not zebraRpc');
  const urls: [string, unknown][] = [
    ...nearRpcs(c).map((u, i): [string, unknown] => [`near.rpc[${i}]`, u]),
    ['zcash.lightwalletd', c.zcash.lightwalletd],
  ];
  for (const [k, v] of urls) if (!isHttps(v)) throw new NetConfigError(`on mainnet ${k} must be an https: URL`);
  // The proxy on this site is a path; anywhere else 1Click must be https.
  if (!isSitePath(c.oneClickUrl) && !isHttps(c.oneClickUrl)) throw new NetConfigError('on mainnet oneClickUrl must be a path on this site or an https: URL');
  // A directory on the same site is a path; anywhere else it must be https.
  if (!c.directoryUrl.startsWith('/') || c.directoryUrl.startsWith('//')) {
    if (!isHttps(c.directoryUrl)) throw new NetConfigError('on mainnet directoryUrl must be a path on this site or an https: URL');
  }
  if (!anchors.escrow || !anchors.escrowCodeHash) throw new NetConfigError('this build has no mainnet escrow built in, so it runs no mainnet swap');
  if (anchors.escrow === FROM_BUILD || anchors.escrowCodeHash === FROM_BUILD) throw new NetConfigError('the mainnet escrow of this build is not set');
  for (const k of ['escrow', 'escrowCodeHash', 'token', 'verifier'] as const) {
    if (c.near[k] !== anchors[k]) throw new NetConfigError(`on mainnet near.${k} must be ${anchors[k]}, the value built into this page; the file says ${c.near[k]}`);
  }
}

/** Where this page load's configuration comes from. */
export interface ConfigChoice {
  url: string;
  /** `link`: a shipped name from ?config=; `build`: VITE_ZECRET_CONFIG; `override`: any other file (development only). */
  source: 'link' | 'build' | 'override';
  /** What ?config= or the build named. */
  name: string;
}

const nameUrl = (v: string) => (/^[a-z0-9-]+$/.test(v) ? `/config/${v}.json` : v);

/**
 * The configuration for this page load, or undefined for the demo. Throws NetConfigError
 * when ?config= names a file this build does not accept.
 */
export function chooseConfig(
  search = typeof location === 'undefined' ? '' : location.search,
  buildDefault = import.meta.env.VITE_ZECRET_CONFIG,
  allowOverride = CONFIG_OVERRIDE_ALLOWED,
  allowMainnet = MAINNET_ALLOWED,
): ConfigChoice | undefined {
  const q = new URLSearchParams(search);
  if (q.get('demo') === '1') return undefined;
  const v = q.get('config');
  const known = knownConfigs(allowMainnet);
  if (v) {
    if (known.includes(v)) return { url: `/config/${v}.json`, source: 'link', name: v };
    if (v === MAINNET_CONFIG) throw new NetConfigError('This build of the page does not run on mainnet.');
    if (!allowOverride) throw new NetConfigError(`This page loads only its own network settings (${known.join(', ')}); it does not load "${v.slice(0, 80)}" from a link.`);
    return { url: nameUrl(v), source: 'override', name: v };
  }
  if (buildDefault === MAINNET_CONFIG && !allowMainnet) throw new NetConfigError('This build of the page does not run on mainnet.');
  if (buildDefault) return { url: nameUrl(buildDefault), source: 'build', name: buildDefault };
  return undefined;
}

/** The configuration URL for this page load, or undefined for the demo (see chooseConfig). */
export function configUrl(search?: string, buildDefault?: string, allowOverride?: boolean, allowMainnet?: boolean): string | undefined {
  return chooseConfig(search, buildDefault, allowOverride, allowMainnet)?.url;
}

export async function fetchNetworkConfig(url: string, signal?: AbortSignal): Promise<NetworkConfig> {
  const res = await fetch(url, { signal, cache: 'no-store' });
  if (!res.ok) throw new NetConfigError(`The network configuration ${url} answered ${res.status}`);
  return parseNetworkConfig(await res.json());
}

/** The name the engine's Zcash code uses for a network. */
export function coreNetwork(n: ZcashNetwork): 'regtest' | 'test' | 'main' {
  return n === 'mainnet' ? 'main' : n === 'testnet' ? 'test' : 'regtest';
}

/** Fill `{txid}` or `{hash}` in an explorer template. Only https links (review 3, H3). */
export function explorerUrl(template: string | undefined, value: string): string | undefined {
  if (!template) return undefined;
  return safeLink(template.replace('{txid}', encodeURIComponent(value)).replace('{hash}', encodeURIComponent(value)));
}

/** `url` when it is an absolute https: URL, else undefined. For links that come from a server or a file. */
export function safeLink(url: unknown): string | undefined {
  return isHttps(url) ? (url as string) : undefined;
}
