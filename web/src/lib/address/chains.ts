// Destination chains that the website offers, how their addresses look, and where their
// transactions can be looked up. A chain that is not in this table is not offered, because
// the page cannot check addresses for it.
import { keccak_256 } from '@noble/hashes/sha3.js';
import {
  base32Decode,
  base58CheckDecode,
  base58Decode,
  base64Decode,
  bech32Decode,
  bytesToHex,
  cashAddrDecode,
  convertBits,
  crc16xmodem,
  segwitDecode,
  XRP_ALPHABET,
} from './encoding';

export type AddressCheck = { ok: true; address: string; note?: string } | { ok: false; reason: string };

type Family =
  | 'evm'
  | 'btc'
  | 'ltc'
  | 'doge'
  | 'bch'
  | 'dash'
  | 'solana'
  | 'near'
  | 'move'
  | 'sui'
  | 'starknet'
  | 'ton'
  | 'tron'
  | 'stellar'
  | 'xrp'
  | 'cardano'
  | 'aleo';

export interface ChainInfo {
  id: string;
  name: string;
  family: Family;
  /** Shown under the address field. */
  hint: string;
  example: string;
  /** Explorer link for a transaction hash, if a well-known explorer exists. */
  txUrl?: (hash: string) => string;
}

const ENC = new TextEncoder();

// ---------------------------------------------------------------- family checks

function checkEvm(a: string): AddressCheck {
  if (!/^0x[0-9a-fA-F]{40}$/.test(a)) return { ok: false, reason: 'An address here is 0x followed by 40 hex characters.' };
  const body = a.slice(2);
  if (/^0+$/.test(body)) return { ok: false, reason: 'This is the zero address. Funds sent there are lost.' };
  const mixed = body !== body.toLowerCase() && body !== body.toUpperCase();
  if (mixed) {
    const hash = bytesToHex(keccak_256(ENC.encode(body.toLowerCase())));
    for (let i = 0; i < 40; i++) {
      const c = body[i];
      if (/[a-f]/i.test(c)) {
        const upper = parseInt(hash[i], 16) >= 8;
        if ((c === c.toUpperCase()) !== upper)
          return { ok: false, reason: 'The capital letters do not match the checksum. Check for a typo.' };
      }
    }
  }
  return { ok: true, address: a };
}

function checkBase58Versions(a: string, versions: number[]): boolean {
  const payload = base58CheckDecode(a);
  return !!payload && payload.length === 21 && versions.includes(payload[0]);
}

function checkBtc(a: string): AddressCheck {
  if (/^bc1/i.test(a)) {
    return segwitDecode('bc', a) ? { ok: true, address: a.toLowerCase() } : { ok: false, reason: 'This bc1 address has a typo or a wrong checksum.' };
  }
  if (/^(tb1|[mn2])/.test(a)) return { ok: false, reason: 'This is a Bitcoin testnet address. Use a mainnet address.' };
  return checkBase58Versions(a, [0x00, 0x05])
    ? { ok: true, address: a }
    : { ok: false, reason: 'A Bitcoin address starts with 1, 3 or bc1, and its checksum must match.' };
}

function checkLtc(a: string): AddressCheck {
  if (/^ltc1/i.test(a)) {
    return segwitDecode('ltc', a) ? { ok: true, address: a.toLowerCase() } : { ok: false, reason: 'This ltc1 address has a typo or a wrong checksum.' };
  }
  return checkBase58Versions(a, [0x30, 0x32, 0x05])
    ? { ok: true, address: a }
    : { ok: false, reason: 'A Litecoin address starts with L, M, 3 or ltc1, and its checksum must match.' };
}

function checkDoge(a: string): AddressCheck {
  return checkBase58Versions(a, [0x1e, 0x16])
    ? { ok: true, address: a }
    : { ok: false, reason: 'A Dogecoin address starts with D, 9 or A, and its checksum must match.' };
}

function checkDash(a: string): AddressCheck {
  return checkBase58Versions(a, [0x4c, 0x10])
    ? { ok: true, address: a }
    : { ok: false, reason: 'A Dash address starts with X or 7, and its checksum must match.' };
}

function checkBch(a: string): AddressCheck {
  if (/^[13]/.test(a) && checkBase58Versions(a, [0x00, 0x05])) {
    return { ok: true, address: a, note: 'This is a legacy address format. It works, but a bitcoincash: address is less likely to be mixed up with Bitcoin.' };
  }
  const dec = cashAddrDecode(a);
  if (!dec || (dec.type !== 0 && dec.type !== 1))
    return { ok: false, reason: 'A Bitcoin Cash address starts with bitcoincash:q or bitcoincash:p, and its checksum must match.' };
  const full = a.toLowerCase().startsWith('bitcoincash:') ? a.toLowerCase() : `bitcoincash:${a.toLowerCase()}`;
  return { ok: true, address: full };
}

function checkSolana(a: string): AddressCheck {
  const bytes = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(a) ? base58Decode(a) : null;
  return bytes && bytes.length === 32
    ? { ok: true, address: a }
    : { ok: false, reason: 'An address here is 32 to 44 base58 characters (no 0, O, I or l).' };
}

const NEAR_NAMED = /^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/;

function checkNear(a: string): AddressCheck {
  if (/^[0-9a-f]{64}$/.test(a)) return { ok: true, address: a };
  if (/^0x[0-9a-f]{40}$/.test(a)) return { ok: true, address: a };
  if (a.length >= 2 && a.length <= 64 && NEAR_NAMED.test(a)) {
    if (!a.includes('.')) return { ok: true, address: a, note: 'Top-level NEAR names are rare. Check that this is the full account name, for example name.near.' };
    return { ok: true, address: a };
  }
  return { ok: false, reason: 'A NEAR account is a name such as alice.near, or 64 lowercase hex characters.' };
}

function checkHex64(a: string, chain: string): AddressCheck {
  return /^0x[0-9a-fA-F]{64}$/.test(a)
    ? { ok: true, address: a.toLowerCase() }
    : { ok: false, reason: `A ${chain} address is 0x followed by 64 hex characters.` };
}

const STARK_PRIME = 2n ** 251n + 17n * 2n ** 192n + 1n;

function checkStarknet(a: string): AddressCheck {
  if (!/^0x[0-9a-fA-F]{1,64}$/.test(a)) return { ok: false, reason: 'A Starknet address is 0x followed by up to 64 hex characters.' };
  const v = BigInt(a);
  if (v === 0n || v >= STARK_PRIME) return { ok: false, reason: 'This value is outside the range of Starknet addresses.' };
  return { ok: true, address: a };
}

function checkTon(a: string): AddressCheck {
  const raw = /^(-1|0):[0-9a-fA-F]{64}$/.exec(a);
  if (raw) return { ok: true, address: a };
  if (a.length !== 48) return { ok: false, reason: 'A TON address is 48 characters, for example starting with UQ or EQ.' };
  const bytes = base64Decode(a);
  if (!bytes || bytes.length !== 36) return { ok: false, reason: 'This TON address has characters that do not belong in it.' };
  const crc = crc16xmodem(bytes.subarray(0, 34));
  if (((bytes[34] << 8) | bytes[35]) !== crc) return { ok: false, reason: 'This TON address has a typo or a wrong checksum.' };
  const tag = bytes[0];
  if (tag & 0x80) return { ok: false, reason: 'This is a TON testnet address. Use a mainnet address.' };
  if ((tag & 0x7f) !== 0x11 && (tag & 0x7f) !== 0x51) return { ok: false, reason: 'This TON address has an unknown format.' };
  if (bytes[1] !== 0x00 && bytes[1] !== 0xff) return { ok: false, reason: 'This TON address is for an unknown workchain.' };
  return { ok: true, address: a };
}

function checkTron(a: string): AddressCheck {
  const payload = base58CheckDecode(a);
  return payload && payload.length === 21 && payload[0] === 0x41
    ? { ok: true, address: a }
    : { ok: false, reason: 'A Tron address starts with T, is 34 characters long, and its checksum must match.' };
}

function checkStellar(a: string): AddressCheck {
  if (a.startsWith('M')) return { ok: false, reason: 'Muxed (M…) Stellar addresses are not supported. Use the G… address.' };
  if (!/^G[A-Z2-7]{55}$/.test(a)) return { ok: false, reason: 'A Stellar address starts with G and is 56 characters long.' };
  const bytes = base32Decode(a);
  if (!bytes || bytes.length !== 35 || bytes[0] !== 6 << 3) return { ok: false, reason: 'This Stellar address is not valid.' };
  const crc = crc16xmodem(bytes.subarray(0, 33));
  if ((bytes[33] | (bytes[34] << 8)) !== crc) return { ok: false, reason: 'This Stellar address has a typo or a wrong checksum.' };
  return { ok: true, address: a };
}

function checkXrp(a: string): AddressCheck {
  if (a.startsWith('X')) return { ok: false, reason: 'X-addresses are not supported. Use the classic address that starts with r.' };
  const payload = a.startsWith('r') ? base58CheckDecode(a, XRP_ALPHABET) : null;
  return payload && payload.length === 21 && payload[0] === 0
    ? { ok: true, address: a, note: 'Do not use an exchange deposit address that needs a destination tag. Tags are not supported here.' }
    : { ok: false, reason: 'An XRP address starts with r, and its checksum must match.' };
}

function checkCardano(a: string): AddressCheck {
  if (/^(Ae2|DdzFF)/.test(a)) return { ok: false, reason: 'This is an old Byron address. Use an address that starts with addr1.' };
  const dec = bech32Decode(a, 200);
  if (!dec || dec.variant !== 'bech32' || dec.hrp !== 'addr') return { ok: false, reason: 'A Cardano address starts with addr1, and its checksum must match.' };
  const bytes = convertBits(dec.words, 5, 8, false);
  if (!bytes || bytes.length < 29) return { ok: false, reason: 'This Cardano address is too short.' };
  if ((bytes[0] & 0x0f) !== 1) return { ok: false, reason: 'This is a Cardano testnet address. Use a mainnet address.' };
  return { ok: true, address: a };
}

function checkAleo(a: string): AddressCheck {
  const dec = bech32Decode(a, 100);
  if (!dec || dec.hrp !== 'aleo' || dec.variant !== 'bech32m') return { ok: false, reason: 'An Aleo address starts with aleo1 and is 63 characters long.' };
  const bytes = convertBits(dec.words, 5, 8, false);
  return bytes && bytes.length === 32 ? { ok: true, address: a } : { ok: false, reason: 'This Aleo address has the wrong length.' };
}

const CHECKS: Record<Family, (a: string) => AddressCheck> = {
  evm: checkEvm,
  btc: checkBtc,
  ltc: checkLtc,
  doge: checkDoge,
  bch: checkBch,
  dash: checkDash,
  solana: checkSolana,
  near: checkNear,
  move: (a) => checkHex64(a, 'Move-chain'),
  sui: (a) => checkHex64(a, 'Sui'),
  starknet: checkStarknet,
  ton: checkTon,
  tron: checkTron,
  stellar: checkStellar,
  xrp: checkXrp,
  cardano: checkCardano,
  aleo: checkAleo,
};

// ---------------------------------------------------------------- the chain table

const EVM_HINT = 'An address that starts with 0x, from any Ethereum-style wallet.';
const EVM_EXAMPLE = '0x85F17Cf997934a597031b2E18a9aB6ebD4B9f6a4';
const scan = (base: string) => (h: string) => `${base}/tx/${h}`;

function evm(id: string, name: string, explorer?: string): ChainInfo {
  return { id, name, family: 'evm', hint: EVM_HINT, example: EVM_EXAMPLE, txUrl: explorer ? scan(explorer) : undefined };
}

export const CHAINS: ChainInfo[] = [
  evm('eth', 'Ethereum', 'https://etherscan.io'),
  evm('base', 'Base', 'https://basescan.org'),
  evm('arb', 'Arbitrum', 'https://arbiscan.io'),
  evm('op', 'Optimism', 'https://optimistic.etherscan.io'),
  evm('pol', 'Polygon', 'https://polygonscan.com'),
  evm('bsc', 'BNB Chain', 'https://bscscan.com'),
  evm('avax', 'Avalanche', 'https://snowtrace.io'),
  evm('gnosis', 'Gnosis', 'https://gnosisscan.io'),
  evm('scroll', 'Scroll', 'https://scrollscan.com'),
  evm('abs', 'Abstract', 'https://abscan.org'),
  evm('bera', 'Berachain', 'https://berascan.com'),
  evm('monad', 'Monad'),
  evm('xlayer', 'X Layer'),
  evm('plasma', 'Plasma'),
  evm('hood', 'Robinhood Chain'),
  evm('adi', 'ADI'),
  evm('hypercore', 'Hyperliquid'),
  { id: 'btc', name: 'Bitcoin', family: 'btc', hint: 'A Bitcoin address that starts with bc1, 1 or 3.', example: 'bc1q…', txUrl: (h) => `https://mempool.space/tx/${h}` },
  { id: 'ltc', name: 'Litecoin', family: 'ltc', hint: 'A Litecoin address that starts with ltc1, L, M or 3.', example: 'ltc1q…' },
  { id: 'doge', name: 'Dogecoin', family: 'doge', hint: 'A Dogecoin address that starts with D.', example: 'D…' },
  { id: 'bch', name: 'Bitcoin Cash', family: 'bch', hint: 'A Bitcoin Cash address that starts with bitcoincash:q.', example: 'bitcoincash:q…' },
  { id: 'dash', name: 'Dash', family: 'dash', hint: 'A Dash address that starts with X.', example: 'X…' },
  { id: 'sol', name: 'Solana', family: 'solana', hint: 'A Solana wallet address (base58, 32 to 44 characters).', example: '9WzD…AWWM', txUrl: (h) => `https://solscan.io/tx/${h}` },
  { id: 'fogo', name: 'Fogo', family: 'solana', hint: 'A Fogo wallet address (base58, 32 to 44 characters).', example: '9WzD…AWWM' },
  { id: 'near', name: 'NEAR', family: 'near', hint: 'A NEAR account such as alice.near, or a 64-character implicit account.', example: 'alice.near', txUrl: (h) => `https://nearblocks.io/txns/${h}` },
  { id: 'aptos', name: 'Aptos', family: 'move', hint: 'An Aptos address: 0x followed by 64 hex characters.', example: '0x…', txUrl: (h) => `https://explorer.aptoslabs.com/txn/${h}?network=mainnet` },
  { id: 'movement', name: 'Movement', family: 'move', hint: 'A Movement address: 0x followed by 64 hex characters.', example: '0x…' },
  { id: 'sui', name: 'Sui', family: 'sui', hint: 'A Sui address: 0x followed by 64 hex characters.', example: '0x…', txUrl: (h) => `https://suivision.xyz/txblock/${h}` },
  { id: 'starknet', name: 'Starknet', family: 'starknet', hint: 'A Starknet address that starts with 0x.', example: '0x…', txUrl: (h) => `https://voyager.online/tx/${h}` },
  { id: 'ton', name: 'TON', family: 'ton', hint: 'A TON address, 48 characters, often starting with UQ or EQ.', example: 'UQ…', txUrl: (h) => `https://tonviewer.com/transaction/${h}` },
  { id: 'tron', name: 'Tron', family: 'tron', hint: 'A Tron address that starts with T.', example: 'T…', txUrl: (h) => `https://tronscan.org/#/transaction/${h}` },
  { id: 'stellar', name: 'Stellar', family: 'stellar', hint: 'A Stellar address that starts with G.', example: 'G…', txUrl: (h) => `https://stellar.expert/explorer/public/tx/${h}` },
  { id: 'xrp', name: 'XRP Ledger', family: 'xrp', hint: 'An XRP address that starts with r. Destination tags are not supported.', example: 'r…', txUrl: (h) => `https://livenet.xrpl.org/transactions/${h}` },
  { id: 'cardano', name: 'Cardano', family: 'cardano', hint: 'A Cardano address that starts with addr1.', example: 'addr1…', txUrl: (h) => `https://cardanoscan.io/transaction/${h}` },
  { id: 'aleo', name: 'Aleo', family: 'aleo', hint: 'An Aleo address that starts with aleo1.', example: 'aleo1…' },
];

/**
 * A valid public address per family, used only as the recipient of dry price quotes made
 * before the user has entered an address. 1Click accepted each of these on 2026-11-13.
 */
const QUOTE_SAMPLE: Record<Family, string> = {
  evm: '0x85F17Cf997934a597031b2E18a9aB6ebD4B9f6a4',
  btc: '1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa',
  ltc: 'LKsFs2FQnubNFuTp2Ar9tU5JrpvrgsivEC',
  doge: 'DH5yaieqoZN36fDVciNyRueRGvGLR3mr7L',
  bch: 'bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a',
  dash: 'XpESxaUmonkq8RaLLp46Brx2K39ggQe226',
  solana: '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM',
  near: 'alice.near',
  move: '0x357b0b74bc833e95a115ad22604854d6b0fca151cecd94111770e5d6ffc9dc2b',
  sui: '0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7',
  starknet: '0x05ce53b9b68fb8e9ecab9283a96d97948914733fd6ed8d9a53a276a419497841',
  ton: 'EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs',
  tron: 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t',
  stellar: 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN',
  xrp: 'rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh',
  cardano: 'addr1qx2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3n0d3vllmyqwsx5wktcd8cc3sq835lu7drv2xwl2wywfgse35a3x',
  aleo: 'aleo1kelm7k8786anyygg788ntlgkx4uqkmkpj7k5ugfuqchd8rnf858sun3qcr',
};

export function quoteSampleAddress(chainId: string): string | undefined {
  const c = BY_ID.get(chainId);
  return c ? QUOTE_SAMPLE[c.family] : undefined;
}

const BY_ID = new Map(CHAINS.map((c) => [c.id, c]));

export function chainInfo(id: string): ChainInfo | undefined {
  return BY_ID.get(id);
}

export function validateDestinationAddress(chainId: string, input: string): AddressCheck {
  const chain = BY_ID.get(chainId);
  if (!chain) return { ok: false, reason: 'This chain is not supported.' };
  const a = input.trim();
  if (a === '') return { ok: false, reason: 'Enter the address that should receive the tokens.' };
  if (/\s/.test(a)) return { ok: false, reason: 'The address contains a space. Copy it again.' };
  if (/^t[13][1-9A-HJ-NP-Za-km-z]{33}$/.test(a) || /^(u|zu|tu|utest|zutest|zs)1[02-9ac-hj-np-z]{60,}$/.test(a))
    return { ok: false, reason: `This looks like a Zcash address. Enter your ${chain.name} address.` };
  return CHECKS[chain.family](a);
}
