import { describe, expect, it } from 'vitest';
import { validateDestinationAddress, CHAINS } from '../../src/lib/address/chains';
import { base58CheckEncode, bech32Encode, convertBits, base58Decode, base58Encode, crc16xmodem, base32Encode } from '../../src/lib/address/encoding';
import {
  decodeUnifiedAddress,
  encodeUnifiedAddress,
  f4jumble,
  f4jumbleInv,
  validateZcashReturnAddress,
  ZCASH_TYPECODES,
} from '../../src/lib/address/zcash';

const ok = (chain: string, a: string) => expect(validateDestinationAddress(chain, a), `${chain} ${a}`).toMatchObject({ ok: true });
const bad = (chain: string, a: string) => expect(validateDestinationAddress(chain, a), `${chain} ${a}`).toMatchObject({ ok: false });

function withVersion(version: number): string {
  const payload = new Uint8Array(21);
  payload[0] = version;
  for (let i = 1; i < 21; i++) payload[i] = i * 7;
  return base58CheckEncode(payload);
}

describe('EVM addresses', () => {
  it('accepts lower-case, upper-case and correctly checksummed addresses', () => {
    ok('eth', '0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed');
    ok('base', '0xfb6916095ca1df60bb79ce92ce3ea74c37c5d359');
    ok('arb', '0xFB6916095CA1DF60BB79CE92CE3EA74C37C5D359');
    ok('hypercore', '0x85F17Cf997934a597031b2E18a9aB6ebD4B9f6a4');
  });
  it('rejects a broken checksum, wrong length and the zero address', () => {
    bad('eth', '0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAeD');
    bad('eth', '0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeA');
    bad('eth', '0x0000000000000000000000000000000000000000');
    bad('eth', '5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed');
  });
});

describe('Bitcoin-family addresses', () => {
  it('accepts Bitcoin legacy, P2SH, SegWit v0 and Taproot', () => {
    ok('btc', '1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa');
    ok('btc', '3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy');
    // One character off (Y for Q): the checksum catches it.
    bad('btc', '3J98t1WpEZ73CNmYviecrnyiWrnqRhWNLy');
    ok('btc', 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4');
    ok('btc', 'BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4');
    // A widely copied example with a bad checksum; the 1Click API rejects it too.
    bad('btc', 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080');
    ok('btc', 'bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0');
  });
  it('rejects typos, testnet, and v0 with a bech32m checksum', () => {
    bad('btc', '1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb');
    bad('btc', 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t5');
    bad('btc', 'tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx');
    // BIP 350 invalid vector: v0 program with a bech32m checksum.
    bad('btc', 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kemeawh');
  });
  it('checks version bytes for Litecoin, Dogecoin and Dash', () => {
    ok('ltc', withVersion(0x30));
    ok('ltc', withVersion(0x32));
    bad('ltc', withVersion(0x00));
    ok('doge', withVersion(0x1e));
    bad('doge', withVersion(0x30));
    ok('dash', withVersion(0x4c));
    bad('dash', withVersion(0x1e));
    const ltcProgram = convertBits(new Uint8Array(20).fill(9), 8, 5, true)!;
    ok('ltc', bech32Encode('ltc', [0, ...ltcProgram], 'bech32'));
  });
  it('accepts CashAddr with and without prefix, and adds the prefix', () => {
    const r = validateDestinationAddress('bch', 'qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a');
    expect(r).toEqual({ ok: true, address: 'bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a' });
    ok('bch', 'bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a');
    bad('bch', 'bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6b');
  });
});

describe('other chains', () => {
  it('Solana and Fogo: 32-byte base58 keys', () => {
    ok('sol', '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM');
    ok('fogo', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');
    bad('sol', '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWW0');
    bad('sol', 'abc');
  });
  it('NEAR named and implicit accounts', () => {
    ok('near', 'alice.near');
    ok('near', 'sub.alice.near');
    ok('near', 'a'.repeat(64).replace(/a/g, 'f'));
    bad('near', 'Alice.near');
    bad('near', 'alice..near');
    bad('near', 'a');
  });
  it('Move chains, Sui and Starknet', () => {
    ok('aptos', '0x357b0b74bc833e95a115ad22604854d6b0fca151cecd94111770e5d6ffc9dc2b');
    ok('movement', '0x357b0b74bc833e95a115ad22604854d6b0fca151cecd94111770e5d6ffc9dc2b');
    ok('sui', '0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7');
    bad('sui', '0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900');
    ok('starknet', '0x05ce53b9b68fb8e9ecab9283a96d97948914733fd6ed8d9a53a276a419497841');
    bad('starknet', '0x' + 'f'.repeat(64));
  });
  it('TON user-friendly and raw forms, with checksum', () => {
    const body = new Uint8Array(34);
    body[0] = 0x51;
    body[1] = 0x00;
    body.fill(0xab, 2);
    const crc = crc16xmodem(body);
    const full = Uint8Array.from([...body, crc >> 8, crc & 0xff]);
    const b64 = btoa(String.fromCharCode(...full)).replace(/\+/g, '-').replace(/\//g, '_');
    ok('ton', b64);
    ok('ton', '0:' + 'ab'.repeat(32));
    const broken = b64.slice(0, -1) + (b64.endsWith('A') ? 'B' : 'A');
    bad('ton', broken);
    const testnet = Uint8Array.from(full);
    testnet[0] = 0xd1;
    const crc2 = crc16xmodem(testnet.subarray(0, 34));
    testnet[34] = crc2 >> 8;
    testnet[35] = crc2 & 0xff;
    bad('ton', btoa(String.fromCharCode(...testnet)));
  });
  it('Tron, Stellar and XRP', () => {
    ok('tron', 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t');
    bad('tron', 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6s');
    ok('stellar', 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN');
    bad('stellar', 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVM');
    ok('xrp', 'rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh');
    bad('xrp', 'rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTi');
    bad('xrp', 'X7AcgcsBL6XDcUb289X4mJ8djcdyKaB5hJDWMArnXr61cqZ');
  });
  it('Cardano Shelley and Aleo', () => {
    ok('cardano', 'addr1qx2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3n0d3vllmyqwsx5wktcd8cc3sq835lu7drv2xwl2wywfgse35a3x');
    bad('cardano', 'addr1qx2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3n0d3vllmyqwsx5wktcd8cc3sq835lu7drv2xwl2wywfgse35a3y');
    bad('cardano', 'addr_test1qz2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3n0d3vllmyqwsx5wktcd8cc3sq835lu7drv2xwl2wywfgs68faae');
    const aleo = bech32Encode('aleo', convertBits(new Uint8Array(32).fill(3), 8, 5, true)!, 'bech32m');
    expect(aleo).toHaveLength(63);
    ok('aleo', aleo);
    bad('aleo', bech32Encode('aleo', convertBits(new Uint8Array(32).fill(3), 8, 5, true)!, 'bech32'));
  });
  it('catches a Zcash address pasted as the destination, and unknown chains', () => {
    bad('eth', 'u1sc85xacmfvsy3l2ny320a4ppzmrssfamyevy67nmdv6dgd30jnjxn5mujeld6r4tuhkm0a4qqy6r58ede086kezc4q9n0xduhq0fym7x');
    bad('qtc', '0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed');
    bad('eth', ' ');
  });
  it('every chain has a name and a hint', () => {
    expect(CHAINS.length).toBeGreaterThanOrEqual(30);
    for (const c of CHAINS) expect(c.name && c.hint).toBeTruthy();
  });
});

describe('encodings', () => {
  it('base58 round-trips with leading zeros', () => {
    const data = Uint8Array.from([0, 0, 1, 2, 3, 255]);
    expect(base58Decode(base58Encode(data))).toEqual(data);
  });
  it('base32 encodes a Stellar key back to the same text', () => {
    expect(base32Encode(new Uint8Array([0x30]))).toBe('GA');
  });
});

// Unified addresses made with zcash_address 0.14.0-pre.1 (the crate the engine uses),
// with receivers filled with constant bytes. See web/README.md, "Test vectors".
const VECTORS = {
  mainOrchard: 'u1sc85xacmfvsy3l2ny320a4ppzmrssfamyevy67nmdv6dgd30jnjxn5mujeld6r4tuhkm0a4qqy6r58ede086kezc4q9n0xduhq0fym7x',
  mainOrchardSaplingP2pkh:
    'u15yem4wfeufstrzy0asj9jt6sh8xh4exvlk7nt90nlzu7vfd35rawz97y9pfmh2q0vldd2ccvpyyasjz2qquwvt79xd27fwsxzvlx7zz7yufl04fn5um3s84z0cc0l9dj40yqnnl8a5g7nv4gdmcyw85a7sdguxux9fm5tfgqq50sumf4agqe7m6ptk8dwmrj5lawfxf7zfu0qqpw0l6',
  mainSaplingOnly:
    'u12nkykgxp6skd3uxcm5yzvaj90zcetknlu66xr5qxarhsdxszpdm9huy6mt0jzx4q843vyqwt2puz3td4t8jj0md7apgwwnnnjyqv675xmm6d0452m5vyu6sp4vstlsw8avdmvmk8z64',
  testOrchardSapling:
    'utest13x4neahaknjvdwe5j4h5le2zhfh502nava4jf5g43z9n8ynk9g5whlvsztnn9lm88eq83zzpvsjst0vnfda32lywxv9uz8qgk8wtgr84d6da980fg0wrj2en9hnywv4khl35008f263gztnlqa3zf0he97k0ahttlmmhm7e7ycmn45nm',
  regtestOrchard: 'uregtest19rzv07lcsrm4rg36s5gc4880n9ft55zp5m2xnfll6t8lwpq2t57tqufkhn2tzqxqlghhwuxagnecpaalzmtpattu9scec0jd25j5s6rc',
  mainR2Orchard: 'zu155k3uz4l9rn78j8yz9gzlgflj7eu9l0n2pf2pvyrhmmdk872v9wac2war3xcfatdhqrsyv87tkvqen545h07zw0pj87j32wzjuhlvknl',
  testR2Orchard: 'zutest12xg4u5kh59xmc28qyhe87cdu97csny8evkp8vyxwgksaq7gesex2sy66dhhpgulvfa9vlj8f2yukw7lgrylsurasrahqklrehqr0220d',
};

describe('Zcash unified addresses', () => {
  it('F4Jumble inverts itself for short and long inputs', () => {
    for (const len of [48, 64, 100, 200, 300]) {
      const m = Uint8Array.from({ length: len }, (_, i) => (i * 31) & 0xff);
      expect(f4jumbleInv(f4jumble(m))).toEqual(m);
    }
  });
  it('decodes vectors from zcash_address and finds the receivers', () => {
    const ua = decodeUnifiedAddress(VECTORS.mainOrchardSaplingP2pkh)!;
    expect(ua.network).toBe('mainnet');
    expect(ua.items.map((i) => i.typecode)).toEqual([0x00, 0x02, 0x03]);
    expect(ua.items[2].data).toEqual(new Uint8Array(43).fill(7));
    expect(decodeUnifiedAddress(VECTORS.testOrchardSapling)!.network).toBe('testnet');
    expect(decodeUnifiedAddress(VECTORS.regtestOrchard)!.network).toBe('regtest');
    expect(decodeUnifiedAddress(VECTORS.mainR2Orchard)!.items[0].data).toEqual(new Uint8Array(43).fill(7));
  });
  it('encodes byte-for-byte the same address as zcash_address', () => {
    expect(encodeUnifiedAddress('mainnet', [{ typecode: ZCASH_TYPECODES.orchard, data: new Uint8Array(43).fill(7) }])).toBe(VECTORS.mainOrchard);
    expect(encodeUnifiedAddress('regtest', [{ typecode: ZCASH_TYPECODES.orchard, data: new Uint8Array(43).fill(7) }])).toBe(VECTORS.regtestOrchard);
  });
  it('validates a return address: network, Orchard receiver, and typos', () => {
    expect(validateZcashReturnAddress(VECTORS.mainOrchard, 'mainnet')).toEqual({ ok: true, address: VECTORS.mainOrchard });
    expect(validateZcashReturnAddress(VECTORS.mainR2Orchard, 'mainnet').ok).toBe(true);
    expect(validateZcashReturnAddress(VECTORS.testR2Orchard, 'testnet').ok).toBe(true);
    expect(validateZcashReturnAddress(VECTORS.testOrchardSapling, 'testnet').ok).toBe(true);
    expect(validateZcashReturnAddress(VECTORS.mainOrchard, 'testnet')).toMatchObject({ ok: false, reason: expect.stringMatching(/mainnet/) });
    expect(validateZcashReturnAddress(VECTORS.mainSaplingOnly, 'mainnet')).toMatchObject({ ok: false, reason: expect.stringMatching(/Orchard/) });
    const typo = VECTORS.mainOrchard.slice(0, 20) + (VECTORS.mainOrchard[20] === 'q' ? 'p' : 'q') + VECTORS.mainOrchard.slice(21);
    expect(validateZcashReturnAddress(typo, 'mainnet').ok).toBe(false);
    expect(validateZcashReturnAddress('t1Rv4exT7bqhZqi2j7xz8bUHDMxwosrjADU', 'mainnet')).toMatchObject({ ok: false, reason: expect.stringMatching(/transparent/) });
    expect(validateZcashReturnAddress('zs1z7rejlpsa98s2rrrfkwmaxu53e4ue0ulcrw0h4x5g8jl04tak0d3mm47vdtahatqrlkngh9sly', 'mainnet')).toMatchObject({ ok: false, reason: expect.stringMatching(/Sapling/) });
    expect(validateZcashReturnAddress('', 'mainnet').ok).toBe(false);
  });
});

describe('quote sample addresses', () => {
  it('every chain has a sample that passes its own check', async () => {
    const { quoteSampleAddress } = await import('../../src/lib/address/chains');
    for (const c of CHAINS) {
      const sample = quoteSampleAddress(c.id)!;
      expect(validateDestinationAddress(c.id, sample), c.id).toMatchObject({ ok: true });
    }
  });
});
