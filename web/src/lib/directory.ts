// The maker directory: a small public JSON file that says who the maker is, how to reach
// it, and its indicative rate. The firm rate comes later, inside the encrypted quote memo.
import { parseUnits, ZEC_DECIMALS, USDC_DECIMALS } from './units';
import type { ZcashNetwork } from './address/zcash';

export interface MakerDirectory {
  format: 'zecret-maker-directory';
  version: 1;
  network: ZcashNetwork;
  updatedAt: string;
  status: 'online' | 'paused';
  maker: {
    name: string;
    /** The maker's shielded unified address. Request and acceptance memos go here. */
    zcashAddress: string;
    /** Public key that encrypts the request memo (32 bytes, hex). */
    requestKey: string;
    /** NEAR account that opens escrows and pays the stipend. */
    nearAccount: string;
    /** NEAR account of the escrow contract. */
    escrowContract: string;
    /** The maker's NEAR key (`ed25519:...`) that signs the directory entry and the quotes. The real engine needs it. */
    nearPublicKey?: string;
    /** Version of the request key. The real engine needs it. */
    epoch?: number;
    /**
     * The maker's limits as `zecret directory` prints them (zatoshis). When present, the
     * signature covers them, and the real engine checks a swap against them before it
     * sends anything. Not read by the page.
     */
    signedLimits?: { min_zats: number; max_zats: number; premium_zats: number };
  };
  rate: {
    /** Indicative price: USDC the maker pays for 1 ZEC, as a decimal string. */
    usdcPerZec: string;
    /** How long the indicative rate is meant to hold. */
    validForSeconds: number;
  };
  limits: { minZec: string; maxZec: string };
  /** Paid to the maker with the acceptance memo: the larger of `bps` of the amount and `minZec`. */
  premium: { bps: number; minZec: string };
  /** Zcash network fee for one shielded send by the swap's temporary wallet. */
  zcashFeePerSendZec: string;
  /**
   * The maker's ed25519 signature (hex, 64 bytes) by `maker.nearPublicKey` over the
   * directory digest of (zcashAddress, epoch, requestKey, nearAccount), as `zecret
   * directory` prints it. Checked by the real engine, which also checks that the key is a
   * full-access key of `maker.nearAccount` on NEAR. Not read by the page.
   */
  signature?: string;
}

/** The directory in base units, ready for price math. */
export interface MakerTerms {
  network: ZcashNetwork;
  status: 'online' | 'paused';
  makerName: string;
  /** micro-USDC per 1 ZEC */
  rateMicroUsdcPerZec: bigint;
  rateValidForSeconds: number;
  minZats: bigint;
  maxZats: bigint;
  premiumBps: bigint;
  premiumMinZats: bigint;
  feePerSendZats: bigint;
  updatedAt: Date;
}

export class DirectoryError extends Error {}

function str(v: unknown, path: string): string {
  if (typeof v !== 'string' || v.length === 0) throw new DirectoryError(`${path} must be a non-empty string`);
  return v;
}

function num(v: unknown, path: string): number {
  if (typeof v !== 'number' || !Number.isFinite(v) || v < 0) throw new DirectoryError(`${path} must be a non-negative number`);
  return v;
}

function obj(v: unknown, path: string): Record<string, unknown> {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) throw new DirectoryError(`${path} must be an object`);
  return v as Record<string, unknown>;
}

function amount(v: unknown, path: string, decimals: number): bigint {
  const r = parseUnits(str(v, path), decimals);
  if (!r.ok) throw new DirectoryError(`${path} must be a decimal amount with at most ${decimals} decimals`);
  return r.value;
}

/** Validates a parsed JSON value and returns the directory and its terms in base units. */
export function parseDirectory(json: unknown): { directory: MakerDirectory; terms: MakerTerms } {
  const d = obj(json, 'directory');
  if (d.format !== 'zecret-maker-directory') throw new DirectoryError('format must be "zecret-maker-directory"');
  if (d.version !== 1) throw new DirectoryError('version must be 1');
  const network = d.network;
  if (network !== 'mainnet' && network !== 'testnet' && network !== 'regtest') throw new DirectoryError('network must be mainnet, testnet or regtest');
  const status = d.status ?? 'online';
  if (status !== 'online' && status !== 'paused') throw new DirectoryError('status must be online or paused');
  const updatedAt = new Date(str(d.updatedAt, 'updatedAt'));
  if (Number.isNaN(updatedAt.getTime())) throw new DirectoryError('updatedAt must be an ISO date');
  const maker = obj(d.maker, 'maker');
  const name = str(maker.name, 'maker.name');
  str(maker.zcashAddress, 'maker.zcashAddress');
  if (!/^[0-9a-f]{64}$/.test(str(maker.requestKey, 'maker.requestKey'))) throw new DirectoryError('maker.requestKey must be 32 bytes of lowercase hex');
  str(maker.nearAccount, 'maker.nearAccount');
  str(maker.escrowContract, 'maker.escrowContract');
  const rate = obj(d.rate, 'rate');
  const rateMicro = amount(rate.usdcPerZec, 'rate.usdcPerZec', USDC_DECIMALS);
  if (rateMicro <= 0n) throw new DirectoryError('rate.usdcPerZec must be positive');
  const validFor = num(rate.validForSeconds, 'rate.validForSeconds');
  const limits = obj(d.limits, 'limits');
  const minZats = amount(limits.minZec, 'limits.minZec', ZEC_DECIMALS);
  const maxZats = amount(limits.maxZec, 'limits.maxZec', ZEC_DECIMALS);
  if (maxZats < minZats) throw new DirectoryError('limits.maxZec must not be below limits.minZec');
  const premium = obj(d.premium, 'premium');
  const bps = num(premium.bps, 'premium.bps');
  if (!Number.isInteger(bps) || bps > 1000) throw new DirectoryError('premium.bps must be an integer from 0 to 1000');
  const premiumMin = amount(premium.minZec, 'premium.minZec', ZEC_DECIMALS);
  const fee = amount(d.zcashFeePerSendZec, 'zcashFeePerSendZec', ZEC_DECIMALS);
  if (d.signature !== undefined && typeof d.signature !== 'string') throw new DirectoryError('signature must be a string');
  if (maker.nearPublicKey !== undefined && (typeof maker.nearPublicKey !== 'string' || !maker.nearPublicKey.startsWith('ed25519:'))) throw new DirectoryError('maker.nearPublicKey must be an ed25519: key');
  if (maker.epoch !== undefined && (typeof maker.epoch !== 'number' || !Number.isInteger(maker.epoch) || maker.epoch < 0)) throw new DirectoryError('maker.epoch must be a non-negative integer');
  return {
    directory: json as MakerDirectory,
    terms: {
      network,
      status,
      makerName: name,
      rateMicroUsdcPerZec: rateMicro,
      rateValidForSeconds: validFor,
      minZats,
      maxZats,
      premiumBps: BigInt(bps),
      premiumMinZats: premiumMin,
      feePerSendZats: fee,
      updatedAt,
    },
  };
}

export const DEFAULT_DIRECTORY_URL = '/maker-directory.json';

/** The directory URL: `?directory=` on the page, else VITE_DIRECTORY_URL, else the bundled sample. */
export function directoryUrl(search = typeof location === 'undefined' ? '' : location.search): string {
  const fromQuery = new URLSearchParams(search).get('directory');
  return fromQuery || import.meta.env.VITE_DIRECTORY_URL || DEFAULT_DIRECTORY_URL;
}

export async function fetchDirectory(url = directoryUrl(), signal?: AbortSignal) {
  const res = await fetch(url, { signal, cache: 'no-store' });
  if (!res.ok) throw new DirectoryError(`The maker directory answered ${res.status}`);
  return parseDirectory(await res.json());
}
