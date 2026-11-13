// Small, dependency-light encoders and decoders used by address validation.
// Hash functions come from @noble/hashes; everything else is written out here.
import { sha256 } from '@noble/hashes/sha2.js';

export const BTC_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
export const XRP_ALPHABET = 'rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz';

export function base58Decode(input: string, alphabet = BTC_ALPHABET): Uint8Array | null {
  if (input.length === 0) return null;
  const map = new Map<string, number>();
  for (let i = 0; i < alphabet.length; i++) map.set(alphabet[i], i);
  let value = 0n;
  for (const ch of input) {
    const digit = map.get(ch);
    if (digit === undefined) return null;
    value = value * 58n + BigInt(digit);
  }
  const bytes: number[] = [];
  while (value > 0n) {
    bytes.push(Number(value & 0xffn));
    value >>= 8n;
  }
  for (const ch of input) {
    if (ch !== alphabet[0]) break;
    bytes.push(0);
  }
  return Uint8Array.from(bytes.reverse());
}

export function base58Encode(data: Uint8Array, alphabet = BTC_ALPHABET): string {
  let value = 0n;
  for (const b of data) value = (value << 8n) | BigInt(b);
  let out = '';
  while (value > 0n) {
    out = alphabet[Number(value % 58n)] + out;
    value /= 58n;
  }
  for (const b of data) {
    if (b !== 0) break;
    out = alphabet[0] + out;
  }
  return out;
}

function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

/** Base58Check: returns the payload without the 4-byte checksum, or null if the checksum fails. */
export function base58CheckDecode(input: string, alphabet = BTC_ALPHABET): Uint8Array | null {
  const raw = base58Decode(input, alphabet);
  if (!raw || raw.length < 5) return null;
  const payload = raw.subarray(0, raw.length - 4);
  const check = raw.subarray(raw.length - 4);
  const digest = sha256(sha256(payload)).subarray(0, 4);
  return equalBytes(check, digest) ? payload : null;
}

export function base58CheckEncode(payload: Uint8Array, alphabet = BTC_ALPHABET): string {
  const digest = sha256(sha256(payload)).subarray(0, 4);
  const all = new Uint8Array(payload.length + 4);
  all.set(payload);
  all.set(digest, payload.length);
  return base58Encode(all, alphabet);
}

// ---------------------------------------------------------------- bech32 / bech32m (BIP 173, BIP 350)

const BECH32_CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
const BECH32_GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
export const BECH32_CONST = 1;
export const BECH32M_CONST = 0x2bc830a3;

function bech32Polymod(values: number[]): number {
  let chk = 1;
  for (const v of values) {
    const top = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((top >>> i) & 1) chk ^= BECH32_GEN[i];
  }
  return chk >>> 0;
}

function hrpExpand(hrp: string): number[] {
  const out: number[] = [];
  for (const c of hrp) out.push(c.charCodeAt(0) >> 5);
  out.push(0);
  for (const c of hrp) out.push(c.charCodeAt(0) & 31);
  return out;
}

export type Bech32Variant = 'bech32' | 'bech32m';

export interface Bech32Decoded {
  hrp: string;
  words: number[];
  variant: Bech32Variant;
}

/** Decodes bech32 or bech32m. `limit` is the maximum string length (90 in BIP 173; Zcash UAs and Cardano allow more). */
export function bech32Decode(input: string, limit = 90): Bech32Decoded | null {
  if (input.length > limit) return null;
  const lower = input.toLowerCase();
  if (input !== lower && input !== input.toUpperCase()) return null;
  const pos = lower.lastIndexOf('1');
  if (pos < 1 || pos + 7 > lower.length) return null;
  const hrp = lower.slice(0, pos);
  for (const c of hrp) {
    const code = c.charCodeAt(0);
    if (code < 33 || code > 126) return null;
  }
  const data: number[] = [];
  for (const c of lower.slice(pos + 1)) {
    const d = BECH32_CHARSET.indexOf(c);
    if (d === -1) return null;
    data.push(d);
  }
  const poly = bech32Polymod([...hrpExpand(hrp), ...data]);
  let variant: Bech32Variant;
  if (poly === BECH32_CONST) variant = 'bech32';
  else if (poly === BECH32M_CONST) variant = 'bech32m';
  else return null;
  return { hrp, words: data.slice(0, -6), variant };
}

export function bech32Encode(hrp: string, words: number[], variant: Bech32Variant): string {
  const values = [...hrpExpand(hrp), ...words, 0, 0, 0, 0, 0, 0];
  const mod = bech32Polymod(values) ^ (variant === 'bech32' ? BECH32_CONST : BECH32M_CONST);
  let out = hrp + '1';
  for (const w of words) out += BECH32_CHARSET[w];
  for (let i = 0; i < 6; i++) out += BECH32_CHARSET[(mod >>> (5 * (5 - i))) & 31];
  return out;
}

/** Regroups bits, as in BIP 173. Returns null on invalid padding when `pad` is false. */
export function convertBits(data: ArrayLike<number>, from: number, to: number, pad: boolean): number[] | null {
  let acc = 0;
  let bits = 0;
  const out: number[] = [];
  const maxv = (1 << to) - 1;
  for (let i = 0; i < data.length; i++) {
    const value = data[i];
    if (value < 0 || value >> from !== 0) return null;
    acc = (acc << from) | value;
    bits += from;
    while (bits >= to) {
      bits -= to;
      out.push((acc >> bits) & maxv);
    }
  }
  if (pad) {
    if (bits > 0) out.push((acc << (to - bits)) & maxv);
  } else if (bits >= from || ((acc << (to - bits)) & maxv) !== 0) {
    return null;
  }
  return out;
}

/** Decodes a SegWit address for the given human-readable part (BIP 173 and BIP 350). */
export function segwitDecode(hrp: string, address: string): { version: number; program: Uint8Array } | null {
  const dec = bech32Decode(address);
  if (!dec || dec.hrp !== hrp || dec.words.length < 1) return null;
  const version = dec.words[0];
  if (version > 16) return null;
  const program = convertBits(dec.words.slice(1), 5, 8, false);
  if (!program || program.length < 2 || program.length > 40) return null;
  if (version === 0 && program.length !== 20 && program.length !== 32) return null;
  if (version === 0 && dec.variant !== 'bech32') return null;
  if (version !== 0 && dec.variant !== 'bech32m') return null;
  return { version, program: Uint8Array.from(program) };
}

// ---------------------------------------------------------------- CashAddr (Bitcoin Cash)

const CASHADDR_GEN: bigint[] = [0x98f2bc8e61n, 0x79b76d99e2n, 0xf33e5fb3c4n, 0xae2eabe2a8n, 0x1e4f43e470n];

function cashPolymod(values: number[]): bigint {
  let c = 1n;
  for (const d of values) {
    const c0 = c >> 35n;
    c = ((c & 0x07ffffffffn) << 5n) ^ BigInt(d);
    for (let i = 0; i < 5; i++) if ((c0 >> BigInt(i)) & 1n) c ^= CASHADDR_GEN[i];
  }
  return c ^ 1n;
}

/** Decodes a CashAddr (with or without its prefix). Returns the version byte and hash. */
export function cashAddrDecode(input: string, prefix = 'bitcoincash'): { type: number; hash: Uint8Array } | null {
  let s = input;
  if (s !== s.toLowerCase() && s !== s.toUpperCase()) return null;
  s = s.toLowerCase();
  if (s.includes(':')) {
    const [p, rest] = s.split(':');
    if (p !== prefix || !rest) return null;
    s = rest;
  }
  const data: number[] = [];
  for (const c of s) {
    const d = BECH32_CHARSET.indexOf(c);
    if (d === -1) return null;
    data.push(d);
  }
  if (data.length < 8) return null;
  const prefixData = [...prefix].map((c) => c.charCodeAt(0) & 31);
  if (cashPolymod([...prefixData, 0, ...data]) !== 0n) return null;
  const payload = convertBits(data.slice(0, -8), 5, 8, false);
  if (!payload || payload.length < 1) return null;
  const version = payload[0];
  const hash = Uint8Array.from(payload.slice(1));
  const sizeBits = version & 0x07;
  const sizes = [20, 24, 28, 32, 40, 48, 56, 64];
  if (hash.length !== sizes[sizeBits]) return null;
  return { type: (version >> 3) & 0x0f, hash };
}

// ---------------------------------------------------------------- CRC16 (XMODEM), used by TON and Stellar

export function crc16xmodem(data: Uint8Array): number {
  let crc = 0;
  for (const byte of data) {
    crc ^= byte << 8;
    for (let i = 0; i < 8; i++) {
      crc = crc & 0x8000 ? (crc << 1) ^ 0x1021 : crc << 1;
      crc &= 0xffff;
    }
  }
  return crc;
}

// ---------------------------------------------------------------- base32 (RFC 4648, Stellar) and base64

const B32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

export function base32Decode(input: string): Uint8Array | null {
  const words: number[] = [];
  for (const c of input) {
    const v = B32.indexOf(c);
    if (v === -1) return null;
    words.push(v);
  }
  const out = convertBits(words, 5, 8, true);
  if (!out) return null;
  // Drop the trailing partial byte created by padding.
  return Uint8Array.from(out.slice(0, Math.floor((words.length * 5) / 8)));
}

export function base32Encode(data: Uint8Array): string {
  const words = convertBits(data, 8, 5, true) ?? [];
  return words.map((w) => B32[w]).join('');
}

export function base64Decode(input: string): Uint8Array | null {
  const std = input.replace(/-/g, '+').replace(/_/g, '/');
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(std) || std.length % 4 !== 0) return null;
  try {
    const bin = atob(std);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

export function hexToBytes(hex: string): Uint8Array | null {
  if (hex.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(hex)) return null;
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  let s = '';
  for (const b of bytes) s += b.toString(16).padStart(2, '0');
  return s;
}
