//! Swap messages, carried in 512-byte shielded memos (ZIP 302 type 0xFF).
//!
//! The three messages of Cloakroom (request, quote, acceptance) with the same framing and
//! encryption. Content changes for NEAR: the request names the taker's fresh ed25519 key
//! (its NEAR and Intents account is the hex of that key) instead of an EVM payout address
//! and asks for an amount of zatoshis; the quote names the escrow and maker NEAR accounts,
//! the NEAR stipend and no relayer fee, and is signed with the maker's ed25519 NEAR key.
//!
//! Payloads are end-to-end encrypted on top of Zcash note encryption, so that someone who
//! later obtains a party's incoming viewing key (an auditor, or a breach) cannot read past
//! RFQs. The RFQ is encrypted to the maker's current epoch key; Quote and Accept are
//! encrypted under an ephemeral Diffie-Hellman between the two parties' per-swap keys, which
//! are erased when the swap ends. Keys: BLAKE2b-256 over (label, shared point, both public
//! keys); cipher: ChaCha20-Poly1305 with a zero nonce (every key is used once).

use anyhow::{anyhow, bail, Result};
use blake2b_simd::Params;
use chacha20poly1305::{aead::{Aead, KeyInit}, ChaCha20Poly1305, Key, Nonce};
use group::{Group, GroupEncoding};
use pasta_curves::pallas;

pub const MAGIC: &[u8; 4] = b"ZCRT";
pub const VERSION: u8 = 1;
const T_RFQ: u8 = 1;
const T_QUOTE: u8 = 2;
const T_ACCEPT: u8 = 3;
const T_FUND: u8 = 4;
const HDR: usize = 7;

/// Longest NEAR account id.
pub const MAX_ACCOUNT_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rfq {
    pub rfq_id: [u8; 16],
    /// Zatoshis the taker sells (the value of the joint note, without its fee).
    pub zats: u64,
    pub a_pub: [u8; 32],
    pub pok_a: [u8; 64],
    pub commit_s_t: [u8; 32],
    /// The taker's fresh ed25519 public key. It signs `commit_claim`, sends `claim`, and its
    /// hex is the NEAR and Intents account that receives the payout.
    pub taker_pk: [u8; 32],
    pub reply_addr: [u8; 43],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quote {
    /// Token base units for the whole request.
    pub price: u128,
    /// Zatoshis the taker pays the maker with Accept (non-refundable).
    pub premium: u64,
    /// yoctoNEAR the maker sends to the taker's account right after `open`.
    pub stipend: u128,
    pub b_pub: [u8; 32],
    pub pok_b: [u8; 64],
    pub s_m: [u8; 32],
    pub t0_secs: u32,     // t0 = open time + t0_secs
    pub t1_secs: u32,     // t1 = t0 + t1_secs
    pub t2_secs: u32,     // t2 = t1 + t2_secs (punish window opens)
    pub reveal_secs: u32, // reveal window after commit_claim
    pub escrow: String,
    pub maker: String,
    /// The maker's ed25519 signature over `sig_digest()`: transferable evidence of the quote,
    /// and proof that it comes from the NEAR key the taker expects.
    pub sig: [u8; 64],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accept {
    pub s_t: [u8; 32],
    pub quote_hash: [u8; 32],
}

/// What travels in a memo. Only routing data is in the clear.
#[derive(Clone, Debug)]
pub enum Envelope {
    Rfq { epoch: u32, eph: [u8; 32], ct: Vec<u8> },
    Quote { rfq_id: [u8; 16], eph: [u8; 32], ct: Vec<u8> },
    Accept { rfq_id: [u8; 16], ct: Vec<u8> },
    Fund { rfq_id: [u8; 16] },
}

fn header(t: u8) -> Vec<u8> {
    let mut v = vec![0xFF];
    v.extend_from_slice(MAGIC);
    v.push(VERSION);
    v.push(t);
    v
}

fn to_memo(v: Vec<u8>) -> [u8; 512] {
    assert!(v.len() <= 512, "memo too long");
    let mut m = [0u8; 512];
    m[..v.len()].copy_from_slice(&v);
    m
}

impl Envelope {
    pub fn encode(&self) -> [u8; 512] {
        let mut v;
        match self {
            Envelope::Rfq { epoch, eph, ct } => {
                v = header(T_RFQ);
                v.extend_from_slice(&epoch.to_le_bytes());
                v.extend_from_slice(eph);
                v.extend_from_slice(&(ct.len() as u16).to_le_bytes());
                v.extend_from_slice(ct);
            }
            Envelope::Quote { rfq_id, eph, ct } => {
                v = header(T_QUOTE);
                v.extend_from_slice(rfq_id);
                v.extend_from_slice(eph);
                v.extend_from_slice(&(ct.len() as u16).to_le_bytes());
                v.extend_from_slice(ct);
            }
            Envelope::Accept { rfq_id, ct } => {
                v = header(T_ACCEPT);
                v.extend_from_slice(rfq_id);
                v.extend_from_slice(&(ct.len() as u16).to_le_bytes());
                v.extend_from_slice(ct);
            }
            Envelope::Fund { rfq_id } => {
                v = header(T_FUND);
                v.extend_from_slice(rfq_id);
            }
        }
        to_memo(v)
    }

    /// Ok(None) for memos that are not swap messages.
    pub fn decode(m: &[u8; 512]) -> Result<Option<Envelope>> {
        if m[0] != 0xFF || &m[1..5] != MAGIC {
            return Ok(None);
        }
        if m[5] != VERSION {
            bail!("unsupported message version {}", m[5]);
        }
        let b = &m[HDR..];
        let take_ct = |off: usize| -> Result<Vec<u8>> {
            let n = u16::from_le_bytes([b[off], b[off + 1]]) as usize;
            if off + 2 + n > b.len() {
                bail!("truncated message");
            }
            Ok(b[off + 2..off + 2 + n].to_vec())
        };
        Ok(Some(match m[6] {
            T_RFQ => Envelope::Rfq { epoch: u32::from_le_bytes(b[..4].try_into()?), eph: b[4..36].try_into()?, ct: take_ct(36)? },
            T_QUOTE => Envelope::Quote { rfq_id: b[..16].try_into()?, eph: b[16..48].try_into()?, ct: take_ct(48)? },
            T_ACCEPT => Envelope::Accept { rfq_id: b[..16].try_into()?, ct: take_ct(16)? },
            T_FUND => Envelope::Fund { rfq_id: b[..16].try_into()? },
            t => bail!("unknown message type {t}"),
        }))
    }

    pub fn wire_len(&self) -> usize {
        let e = self.encode();
        512 - e.iter().rev().take_while(|b| **b == 0).count()
    }
}

// ---------------------------------------------------------------- payload codecs

fn put_str(v: &mut Vec<u8>, s: &str) {
    assert!(s.len() <= MAX_ACCOUNT_LEN, "account id too long");
    v.push(s.len() as u8);
    v.extend_from_slice(s.as_bytes());
}

/// A cursor over a byte slice that fails instead of panicking.
struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.0.len() < n {
            bail!("message too short");
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into()?)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_le_bytes(self.arr()?))
    }
    fn string(&mut self) -> Result<String> {
        let n = self.take(1)?[0] as usize;
        if n > MAX_ACCOUNT_LEN {
            bail!("account id too long");
        }
        Ok(String::from_utf8(self.take(n)?.to_vec())?)
    }
    fn end(&self) -> Result<()> {
        if !self.0.is_empty() {
            bail!("trailing bytes in message");
        }
        Ok(())
    }
}

impl Rfq {
    pub const LEN: usize = 16 + 8 + 32 + 64 + 32 + 32 + 43;

    pub fn to_bytes(&self) -> Vec<u8> {
        [&self.rfq_id[..], &self.zats.to_le_bytes(), &self.a_pub, &self.pok_a, &self.commit_s_t, &self.taker_pk, &self.reply_addr].concat()
    }
    pub fn from_bytes(b: &[u8]) -> Result<Rfq> {
        if b.len() != Self::LEN {
            bail!("bad RFQ length");
        }
        let mut r = Rd(b);
        let q = Rfq {
            rfq_id: r.arr()?,
            zats: r.u64()?,
            a_pub: r.arr()?,
            pok_a: r.arr()?,
            commit_s_t: r.arr()?,
            taker_pk: r.arr()?,
            reply_addr: r.arr()?,
        };
        r.end()?;
        Ok(q)
    }
}

impl Quote {
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.unsigned_bytes(), self.sig.to_vec()].concat()
    }
    /// What the maker signs: domain-separated, bound to the request id.
    pub fn sig_digest(&self, rfq_id: &[u8; 16]) -> [u8; 32] {
        crate::keys::h32(b"quote-sig-near", &[rfq_id, &self.unsigned_bytes()])
    }
    pub fn unsigned_bytes(&self) -> Vec<u8> {
        let mut v = [
            &self.price.to_le_bytes()[..],
            &self.premium.to_le_bytes(),
            &self.stipend.to_le_bytes(),
            &self.b_pub,
            &self.pok_b,
            &self.s_m,
            &self.t0_secs.to_le_bytes(),
            &self.t1_secs.to_le_bytes(),
            &self.t2_secs.to_le_bytes(),
            &self.reveal_secs.to_le_bytes(),
        ]
        .concat();
        put_str(&mut v, &self.escrow);
        put_str(&mut v, &self.maker);
        v
    }
    pub fn from_bytes(b: &[u8]) -> Result<Quote> {
        let mut r = Rd(b);
        let q = Quote {
            price: r.u128()?,
            premium: r.u64()?,
            stipend: r.u128()?,
            b_pub: r.arr()?,
            pok_b: r.arr()?,
            s_m: r.arr()?,
            t0_secs: r.u32()?,
            t1_secs: r.u32()?,
            t2_secs: r.u32()?,
            reveal_secs: r.u32()?,
            escrow: r.string()?,
            maker: r.string()?,
            sig: r.arr()?,
        };
        r.end()?;
        Ok(q)
    }
}

impl Accept {
    pub fn to_bytes(&self) -> Vec<u8> {
        [&self.s_t[..], &self.quote_hash].concat()
    }
    pub fn from_bytes(b: &[u8]) -> Result<Accept> {
        if b.len() != 64 {
            bail!("bad Accept length");
        }
        Ok(Accept { s_t: b[..32].try_into()?, quote_hash: b[32..].try_into()? })
    }
}

// ---------------------------------------------------------------- end-to-end encryption

/// A Diffie-Hellman key on Pallas (generator = the standard Pallas generator).
pub fn dh_public(sk: &pallas::Scalar) -> [u8; 32] {
    (pallas::Point::generator() * sk).to_bytes()
}

fn kdf(label: &[u8], shared: &pallas::Point, a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut st = Params::new().hash_length(32).personal(b"CloakroomAEADv2_").to_state();
    for part in [label, &shared.to_bytes()[..], a, b] {
        st.update(&(part.len() as u64).to_le_bytes());
        st.update(part);
    }
    st.finalize().as_bytes().try_into().unwrap()
}

/// shared = [my_sk] their_pub; key bound to both public keys (ordered: initiator, responder).
fn session_key(label: &[u8], my_sk: &pallas::Scalar, their_pub: &[u8; 32], initiator_pub: &[u8; 32], responder_pub: &[u8; 32]) -> Result<[u8; 32]> {
    let p: pallas::Point = Option::from(pallas::Point::from_bytes(their_pub)).ok_or_else(|| anyhow!("bad DH public key"))?;
    if bool::from(p.is_identity()) {
        bail!("identity DH key");
    }
    Ok(kdf(label, &(p * my_sk), initiator_pub, responder_pub))
}

pub fn seal(label: &[u8], my_sk: &pallas::Scalar, their_pub: &[u8; 32], init_pub: &[u8; 32], resp_pub: &[u8; 32], pt: &[u8]) -> Result<Vec<u8>> {
    let k = session_key(label, my_sk, their_pub, init_pub, resp_pub)?;
    ChaCha20Poly1305::new(Key::from_slice(&k)).encrypt(Nonce::from_slice(&[0u8; 12]), pt).map_err(|_| anyhow!("encrypt"))
}

pub fn open(label: &[u8], my_sk: &pallas::Scalar, their_pub: &[u8; 32], init_pub: &[u8; 32], resp_pub: &[u8; 32], ct: &[u8]) -> Result<Vec<u8>> {
    let k = session_key(label, my_sk, their_pub, init_pub, resp_pub)?;
    ChaCha20Poly1305::new(Key::from_slice(&k)).decrypt(Nonce::from_slice(&[0u8; 12]), ct).map_err(|_| anyhow!("decryption failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::random_scalar;

    fn quote(escrow: &str, maker: &str) -> Quote {
        Quote {
            price: 300_000_000,
            premium: 50_000,
            stipend: 10u128.pow(22),
            b_pub: [3; 32],
            pok_b: [4; 64],
            s_m: [5; 32],
            t0_secs: 60,
            t1_secs: 600,
            t2_secs: 7200,
            reveal_secs: 3600,
            escrow: escrow.into(),
            maker: maker.into(),
            sig: [7; 64],
        }
    }

    #[test]
    fn rfq_quote_accept_roundtrip_and_sizes() {
        let (maker_epoch, e_t, e_m) = (random_scalar(), random_scalar(), random_scalar());
        let (pm, pt, pq) = (dh_public(&maker_epoch), dh_public(&e_t), dh_public(&e_m));
        let rfq = Rfq { rfq_id: [1; 16], zats: 100_000_000, a_pub: [3; 32], pok_a: [4; 64], commit_s_t: [5; 32], taker_pk: [6; 32], reply_addr: [7; 43] };
        // RFQ: taker -> maker epoch key
        let ct = seal(b"rfq", &e_t, &pm, &pt, &pm, &rfq.to_bytes()).unwrap();
        let env = Envelope::Rfq { epoch: 7, eph: pt, ct };
        let wire = env.encode();
        let Envelope::Rfq { eph, ct, .. } = Envelope::decode(&wire).unwrap().unwrap() else { panic!() };
        let got = Rfq::from_bytes(&open(b"rfq", &maker_epoch, &eph, &eph, &pm, &ct).unwrap()).unwrap();
        assert_eq!(got, rfq);
        // the wrong key cannot read it
        assert!(open(b"rfq", &random_scalar(), &eph, &eph, &pm, &ct).is_err());
        // Quote: maker ephemeral -> taker ephemeral; Accept on the same pair, different label.
        // Longest possible account ids: the quote must still fit in one memo.
        let long = "a".repeat(MAX_ACCOUNT_LEN);
        let q = quote(&long, &long);
        let qct = seal(b"quote", &e_m, &pt, &pt, &pq, &q.to_bytes()).unwrap();
        let qenv = Envelope::Quote { rfq_id: [1; 16], eph: pq, ct: qct.clone() };
        assert_eq!(Quote::from_bytes(&open(b"quote", &e_t, &pq, &pt, &pq, &qct).unwrap()).unwrap(), q);
        let a = Accept { s_t: [9; 32], quote_hash: [8; 32] };
        let act = seal(b"accept", &e_t, &pq, &pt, &pq, &a.to_bytes()).unwrap();
        assert_eq!(open(b"accept", &e_m, &pt, &pt, &pq, &act).unwrap(), a.to_bytes());
        assert!(open(b"quote", &e_m, &pt, &pt, &pq, &act).is_err(), "labels separate keys");
        let aenv = Envelope::Accept { rfq_id: [1; 16], ct: act };
        assert!(env.wire_len() <= 512 && qenv.wire_len() <= 512 && aenv.wire_len() <= 512);
        // decoding what was encoded gives the same envelope bytes
        for e in [&env, &qenv, &aenv, &Envelope::Fund { rfq_id: [2; 16] }] {
            assert_eq!(Envelope::decode(&e.encode()).unwrap().unwrap().encode(), e.encode());
        }
    }

    #[test]
    fn rejects_malformed_payloads() {
        let q = quote("escrow.near", "maker.near");
        let b = q.to_bytes();
        assert!(Quote::from_bytes(&b[..b.len() - 1]).is_err(), "truncated");
        assert!(Quote::from_bytes(&[b.clone(), vec![0]].concat()).is_err(), "trailing byte");
        assert!(Rfq::from_bytes(&[0u8; Rfq::LEN - 1]).is_err());
        // not one of our memos
        let mut m = [0u8; 512];
        m[0] = 0xF6;
        assert!(Envelope::decode(&m).unwrap().is_none());
        // ours, but a version we do not speak
        let mut m = Envelope::Fund { rfq_id: [1; 16] }.encode();
        m[5] = VERSION + 1;
        assert!(Envelope::decode(&m).is_err());
        // the signature digest covers every field
        let mut q2 = q.clone();
        q2.stipend += 1;
        assert_ne!(q.sig_digest(&[1; 16]), q2.sig_digest(&[1; 16]));
        assert_ne!(q.sig_digest(&[1; 16]), q.sig_digest(&[2; 16]));
    }
}
