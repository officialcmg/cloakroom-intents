// Zcash unified addresses (ZIP 316): bech32m without a length limit, F4Jumble, then
// typecode-length-value items. The swap engine sends refunds and leftovers to the Orchard
// receiver of a unified address, so a return address must contain one.
import { blake2b } from '@noble/hashes/blake2.js';
import { bech32Decode, bech32Encode, convertBits } from './encoding';

export type ZcashNetwork = 'mainnet' | 'testnet' | 'regtest';

const UA_HRPS: Record<ZcashNetwork, string[]> = {
  // Revision 0 ("u"), then revision 2 shielded-only ("zu") and transparent-including ("tu").
  mainnet: ['u', 'zu', 'tu'],
  testnet: ['utest', 'zutest', 'tutest'],
  regtest: ['uregtest', 'zuregtest', 'turegtest'],
};

const TYPECODE = { p2pkh: 0x00, p2sh: 0x01, sapling: 0x02, orchard: 0x03 } as const;

const ENCODER = new TextEncoder();

function personal(tag: string, i: number, j?: number): Uint8Array {
  const p = new Uint8Array(16);
  p.set(ENCODER.encode(tag));
  p[13] = i;
  if (j !== undefined) {
    p[14] = j & 0xff;
    p[15] = (j >> 8) & 0xff;
  }
  return p;
}

function hashH(i: number, u: Uint8Array, outLen: number): Uint8Array {
  return blake2b(u, { dkLen: outLen, personalization: personal('UA_F4Jumble_H', i, 0) });
}

function hashG(i: number, u: Uint8Array, outLen: number): Uint8Array {
  const out = new Uint8Array(outLen);
  const blocks = Math.ceil(outLen / 64);
  for (let j = 0; j < blocks; j++) {
    const h = blake2b(u, { dkLen: 64, personalization: personal('UA_F4Jumble_G', i, j) });
    out.set(h.subarray(0, Math.min(64, outLen - 64 * j)), 64 * j);
  }
  return out;
}

function xor(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length);
  for (let i = 0; i < a.length; i++) out[i] = a[i] ^ b[i];
  return out;
}

function split(m: Uint8Array): [number, number] {
  const left = Math.min(64, Math.floor(m.length / 2));
  return [left, m.length - left];
}

export function f4jumble(m: Uint8Array): Uint8Array {
  const [lL, lR] = split(m);
  const a = m.subarray(0, lL);
  const b = m.subarray(lL);
  const x = xor(b, hashG(0, a, lR));
  const y = xor(a, hashH(0, x, lL));
  const d = xor(x, hashG(1, y, lR));
  const c = xor(y, hashH(1, d, lL));
  const out = new Uint8Array(m.length);
  out.set(c);
  out.set(d, lL);
  return out;
}

export function f4jumbleInv(m: Uint8Array): Uint8Array {
  const [lL, lR] = split(m);
  const c = m.subarray(0, lL);
  const d = m.subarray(lL);
  const y = xor(c, hashH(1, d, lL));
  const x = xor(d, hashG(1, y, lR));
  const a = xor(y, hashH(0, x, lL));
  const b = xor(x, hashG(0, a, lR));
  const out = new Uint8Array(m.length);
  out.set(a);
  out.set(b, lL);
  return out;
}

function readCompactSize(buf: Uint8Array, pos: number): [number, number] | null {
  if (pos >= buf.length) return null;
  const first = buf[pos];
  if (first < 0xfd) return [first, pos + 1];
  if (first === 0xfd && pos + 3 <= buf.length) return [buf[pos + 1] | (buf[pos + 2] << 8), pos + 3];
  if (first === 0xfe && pos + 5 <= buf.length)
    return [(buf[pos + 1] | (buf[pos + 2] << 8) | (buf[pos + 3] << 16)) + buf[pos + 4] * 2 ** 24, pos + 5];
  return null;
}

export interface UaItem {
  typecode: number;
  data: Uint8Array;
}

export interface DecodedUa {
  network: ZcashNetwork;
  hrp: string;
  items: UaItem[];
}

/** Decodes a unified address. Returns null if any check fails. */
export function decodeUnifiedAddress(address: string): DecodedUa | null {
  const dec = bech32Decode(address.trim(), 1000);
  if (!dec || dec.variant !== 'bech32m') return null;
  const network = (Object.keys(UA_HRPS) as ZcashNetwork[]).find((n) => UA_HRPS[n].includes(dec.hrp));
  if (!network) return null;
  const bytes = convertBits(dec.words, 5, 8, false);
  if (!bytes || bytes.length < 48) return null;
  const raw = f4jumbleInv(Uint8Array.from(bytes));
  const padding = raw.subarray(raw.length - 16);
  const expected = new Uint8Array(16);
  expected.set(ENCODER.encode(dec.hrp));
  for (let i = 0; i < 16; i++) if (padding[i] !== expected[i]) return null;
  const body = raw.subarray(0, raw.length - 16);
  const items: UaItem[] = [];
  let pos = 0;
  while (pos < body.length) {
    const tc = readCompactSize(body, pos);
    if (!tc) return null;
    const len = readCompactSize(body, tc[1]);
    if (!len) return null;
    const end = len[1] + len[0];
    if (end > body.length) return null;
    items.push({ typecode: tc[0], data: body.slice(len[1], end) });
    pos = end;
  }
  return { network, hrp: dec.hrp, items };
}

/** Encodes a revision 0 unified address from receivers (typecode < 0xfd, length < 0xfd). */
export function encodeUnifiedAddress(network: ZcashNetwork, items: UaItem[]): string {
  const hrp = UA_HRPS[network][0];
  const parts: number[] = [];
  for (const it of [...items].sort((a, b) => a.typecode - b.typecode)) {
    parts.push(it.typecode, it.data.length, ...it.data);
  }
  const pad = new Uint8Array(16);
  pad.set(ENCODER.encode(hrp));
  const raw = Uint8Array.from([...parts, ...pad]);
  const words = convertBits(f4jumble(raw), 8, 5, true) ?? [];
  return bech32Encode(hrp, words, 'bech32m');
}

export type ZcashAddressCheck =
  | { ok: true; address: string }
  | { ok: false; reason: string };

const NETWORK_WORD: Record<ZcashNetwork, string> = { mainnet: 'mainnet', testnet: 'testnet', regtest: 'regtest' };
const UA_PREFIX: Record<ZcashNetwork, string> = { mainnet: 'u1', testnet: 'utest1', regtest: 'uregtest1' };

/** Checks a Zcash return address. It must be a unified address with an Orchard receiver on `network`. */
export function validateZcashReturnAddress(input: string, network: ZcashNetwork): ZcashAddressCheck {
  const address = input.trim();
  if (address === '') return { ok: false, reason: 'Enter a Zcash address.' };
  const lower = address.toLowerCase();
  if (/^(t1|t3|tm|t2)/i.test(address))
    return { ok: false, reason: `This is a transparent address. Use a shielded unified address (it starts with ${UA_PREFIX[network]}).` };
  if (lower.startsWith('zs1') || lower.startsWith('ztestsapling') || lower.startsWith('zregtestsapling'))
    return { ok: false, reason: `This is a Sapling address. Use a unified address (it starts with ${UA_PREFIX[network]}); most wallets show one.` };
  const ua = decodeUnifiedAddress(address);
  if (!ua) return { ok: false, reason: 'This is not a valid Zcash unified address. Copy it again from your wallet.' };
  if (ua.network !== network)
    return { ok: false, reason: `This address is for Zcash ${NETWORK_WORD[ua.network]}. This swap runs on ${NETWORK_WORD[network]}.` };
  const orchard = ua.items.find((i) => i.typecode === TYPECODE.orchard);
  if (!orchard || orchard.data.length !== 43)
    return { ok: false, reason: 'This address has no Orchard part, so the app cannot send to it privately. Use a newer unified address.' };
  return { ok: true, address };
}

export const ZCASH_TYPECODES = TYPECODE;
