// The network configuration of the real engine: which Zcash and NEAR endpoints it uses,
// the escrow it trusts, the token and the Verifier, and how the exit runs. It is a small
// public JSON file, read at page load. Without one the page runs the simulated engine
// (the demo).
//
// Where it comes from, first match wins:
//   ?demo=1                      the simulated engine, whatever else is set
//   ?config=<name or URL>        a name means /config/<name>.json
//   VITE_ZECRET_CONFIG           the same, at build time
// Shipped files: public/config/local.json (Zebra regtest through a CORS proxy and a NEAR
// sandbox) and public/config/testnet.json (Zcash testnet over gRPC-web, NEAR testnet).
import type { ZcashNetwork } from './address/zcash';

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
    rpc: string;
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
  /** yoctoNEAR: the smallest stipend the engine accepts (0.03 NEAR). */
  minStipend: string;
  quoteTimeoutSecs?: number;
  maxHorizonSecs?: number;
  exit:
    | { mode: 'oneclick'; oneClickUrl: string }
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

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown, p: string): string => {
  if (typeof v !== 'string' || !v) throw new NetConfigError(`${p} must be a non-empty string`);
  return v;
};

/** Validates a parsed configuration file. */
export function parseNetworkConfig(json: unknown): NetworkConfig {
  if (!isObj(json) || json.format !== 'zecret-config' || json.version !== 1) throw new NetConfigError('not a zecret-config version 1 file');
  const network = json.network;
  if (network !== 'mainnet' && network !== 'testnet' && network !== 'regtest') throw new NetConfigError('network must be mainnet, testnet or regtest');
  str(json.label, 'label');
  str(json.directoryUrl, 'directoryUrl');
  if (!isObj(json.zcash) || (!json.zcash.zebraRpc && !json.zcash.lightwalletd)) throw new NetConfigError('zcash needs zebraRpc or lightwalletd');
  if (json.zcash.lightwalletd && network === 'regtest') throw new NetConfigError('regtest uses zebraRpc');
  if (!isObj(json.near)) throw new NetConfigError('near is missing');
  for (const k of ['rpc', 'escrow', 'escrowCodeHash', 'token', 'tokenSymbol', 'verifier']) str(json.near[k], `near.${k}`);
  const dec = json.near.tokenDecimals;
  if (typeof dec !== 'number' || !Number.isInteger(dec) || dec < 6 || dec > 30) throw new NetConfigError('near.tokenDecimals must be an integer from 6 to 30');
  if (!isObj(json.timing)) throw new NetConfigError('timing is missing');
  for (const k of ['inclusion_secs', 'finality_secs', 'reorg_budget', 'fund_secs', 'k_z']) {
    if (typeof json.timing[k] !== 'number') throw new NetConfigError(`timing.${k} must be a number`);
  }
  if (!/^\d+$/.test(String(json.minStipend))) throw new NetConfigError('minStipend must be a whole number of yoctoNEAR');
  if (!isObj(json.exit)) throw new NetConfigError('exit is missing');
  if (json.exit.mode === 'oneclick') str(json.exit.oneClickUrl, 'exit.oneClickUrl');
  else if (json.exit.mode === 'direct') str(json.exit.relayUrl, 'exit.relayUrl');
  else throw new NetConfigError('exit.mode must be oneclick or direct');
  if (network === 'mainnet' && json.exit.mode !== 'oneclick') throw new NetConfigError('mainnet exits through 1Click');
  return json as unknown as NetworkConfig;
}

/** The configuration URL for this page load, or undefined for the demo. */
export function configUrl(search = typeof location === 'undefined' ? '' : location.search, buildDefault = import.meta.env.VITE_ZECRET_CONFIG): string | undefined {
  const q = new URLSearchParams(search);
  if (q.get('demo') === '1') return undefined;
  const v = q.get('config') || buildDefault || '';
  if (!v) return undefined;
  if (/^[a-z0-9-]+$/.test(v)) return `/config/${v}.json`;
  return v;
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

/** Fill `{txid}` or `{hash}` in an explorer template. */
export function explorerUrl(template: string | undefined, value: string): string | undefined {
  if (!template) return undefined;
  return template.replace('{txid}', value).replace('{hash}', value);
}
