//! The interface the Web Worker calls. Every method blocks the worker until it is done;
//! network calls inside are synchronous XMLHttpRequests.

use orchard::keys::Scope;
use wasm_bindgen::prelude::*;

use crate::{
    backend::{Lwd, Rpc},
    memo_text,
    net::{default_transport, now_ms},
    wallet::{self, Backend, Net, Wallet},
    Me,
};

fn js(e: anyhow::Error) -> JsError {
    JsError::new(&format!("{e:#}"))
}

#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("wasm panic: {info}").into());
    }));
}

#[wasm_bindgen]
pub struct WebWallet {
    me: Me,
    w: Wallet,
}

#[wasm_bindgen]
impl WebWallet {
    /// `backend` is "zebra" (url = JSON-RPC endpoint, e.g. a local CORS proxy) or "lwd"
    /// (url = gRPC-web base URL). `sk_hex` restores a wallet; empty creates a new one.
    #[wasm_bindgen(constructor)]
    pub fn new(backend: &str, url: &str, network: &str, nu7_height: Option<u32>, birthday: Option<u64>, sk_hex: &str) -> Result<WebWallet, JsError> {
        wallet::set_network(network).map_err(js)?;
        let me = if sk_hex.is_empty() { Me::generate() } else { Me::from_hex(sk_hex).map_err(js)? };
        let (b, params) = match backend {
            "zebra" => (Backend::Zebra(Rpc::new(default_transport(), url)), Net::Local(wallet::regtest_params(nu7_height))),
            "lwd" => {
                let bd = birthday.ok_or_else(|| JsError::new("birthday required with lightwalletd"))?;
                (Backend::Lwd(Lwd::new(default_transport(), url, bd)), Net::Test)
            }
            other => return Err(JsError::new(&format!("unknown backend {other}"))),
        };
        let mut w = Wallet::new(b, params);
        w.watch("wallet", &me.fvk, &[Scope::External, Scope::Internal]);
        Ok(WebWallet { me, w })
    }

    pub fn address(&self) -> String {
        self.me.ua()
    }

    /// The spending key, for the page to keep in IndexedDB or a recovery file.
    pub fn sk_hex(&self) -> String {
        hex::encode(self.me.sk_bytes())
    }

    /// Scan up to the chain tip. Returns the scanned height.
    pub fn sync(&mut self) -> Result<u64, JsError> {
        self.w.sync().map_err(js)?;
        Ok(self.w.height)
    }

    pub fn height(&self) -> u64 {
        self.w.height
    }

    pub fn balance(&self) -> u64 {
        self.w.balance("wallet")
    }

    /// Received notes as JSON: value, height, txid, memo text, memo hex, spent.
    pub fn notes_json(&self) -> String {
        let v: Vec<_> = self
            .w
            .received
            .iter()
            .map(|r| {
                serde_json::json!({
                    "zats": r.note.value().inner(),
                    "height": r.height,
                    "txid": r.txid,
                    "memo": memo_text(&r.memo),
                    "memo_hex": hex::encode(r.memo),
                    "spent_in": r.spent_in,
                })
            })
            .collect();
        serde_json::to_string(&v).unwrap()
    }

    /// Ironwood look-alike count and other scan facts, as JSON.
    pub fn scan_info(&self) -> String {
        serde_json::json!({
            "height": self.w.height,
            "notes": self.w.received.len(),
            "ironwood_actions_scanned": self.w.actions_scanned(),
            "lookalike_txs": self.w.cover.values().map(|n| *n as u64).sum::<u64>(),
        })
        .to_string()
    }

    /// Fully shielded send of `zats` to `to_ua` with a memo of up to 512 bytes; change
    /// returns to this wallet. Returns the txid.
    pub fn pay(&mut self, to_ua: &str, zats: u64, memo: &[u8]) -> Result<String, JsError> {
        if memo.len() > 512 {
            return Err(JsError::new("memo longer than 512 bytes"));
        }
        let mut m = [0u8; 512];
        m[..memo.len()].copy_from_slice(memo);
        let to = wallet::decode_ua(to_ua).map_err(js)?;
        let (change, ovk, ask) = (self.me.change(), self.me.ovk.clone(), self.me.ask.clone());
        self.w.pay("wallet", &ask, change, &ovk, vec![(to, zats, m)]).map_err(js)
    }

    /// Timings of the last `pay`, as JSON (milliseconds).
    pub fn last_send_timings(&self) -> String {
        serde_json::to_string(&self.w.last_send).unwrap()
    }
}

/// Build the Halo 2 proving key (and the verifying key from it). Returns milliseconds.
#[wasm_bindgen]
pub fn prepare_proving_key() -> f64 {
    wallet::prepare_proving_key()
}

/// lightwalletd over gRPC-web, read-only: server info, tip, the tree state before the last
/// `count` blocks, and those blocks. Returns JSON with what each call gave.
#[wasm_bindgen]
pub fn lwd_probe(url: &str, count: u64) -> Result<String, JsError> {
    let l = Lwd::new(default_transport(), url, 0);
    let t = now_ms();
    let info = l.info().map_err(js)?;
    let t_info = now_ms() - t;
    let t = now_ms();
    let tip = l.tip().map_err(js)?;
    let t_tip = now_ms() - t;
    let (lo, hi) = (tip + 1 - count.max(1), tip);
    let t = now_ms();
    let ts = l.tree_state(lo - 1).map_err(js)?;
    let t_tree = now_ms() - t;
    let frontier = ts.ironwood_tree().map_err(|e| js(e.into()))?.to_frontier();
    let t = now_ms();
    let blocks = l.blocks(lo, hi).map_err(js)?;
    let t_blocks = now_ms() - t;
    // GetTransaction: the last transaction in the range, preferring one with Ironwood actions.
    let pick = blocks.iter().flat_map(|b| &b.vtx).filter(|t| !t.ironwood_actions.is_empty()).last()
        .or_else(|| blocks.iter().flat_map(|b| &b.vtx).last());
    let get_tx = match pick {
        Some(ctx) => {
            let t = now_ms();
            let r = l.raw_tx(&ctx.txid);
            let mut disp = ctx.txid.clone();
            disp.reverse();
            match r {
                Ok(raw) => serde_json::json!({"txid": hex::encode(disp), "bytes": raw.len(), "ironwood_actions": ctx.ironwood_actions.len(), "ms": now_ms() - t}),
                Err(e) => serde_json::json!({"txid": hex::encode(disp), "error": format!("{e:#}")}),
            }
        }
        None => serde_json::json!({"skipped": "no transactions in range"}),
    };
    // SendTransaction with four zero bytes: checks the call path; the server must reject it.
    let t = now_ms();
    let send = match l.send(vec![0u8; 4]) {
        Ok(()) => serde_json::json!({"accepted": true}),
        Err(e) => serde_json::json!({"accepted": false, "error": format!("{e:#}"), "ms": now_ms() - t}),
    };
    Ok(serde_json::json!({
        "get_transaction": get_tx,
        "send_transaction_invalid": send,
        "info": {"vendor": info.vendor, "version": info.version, "chain": info.chain_name, "height": info.block_height, "branch": info.consensus_branch_id, "ms": t_info},
        "tip": {"height": tip, "ms": t_tip},
        "tree_state": {"height": ts.height, "ironwood_tree_len": ts.ironwood_tree.len(), "orchard_tree_len": ts.orchard_tree.len(),
                        "ironwood_leaves": frontier.value().map(|f| u64::from(f.position()) + 1).unwrap_or(0), "ms": t_tree},
        "blocks": {"from": lo, "to": hi, "count": blocks.len(), "txs": blocks.iter().map(|b| b.vtx.len()).sum::<usize>(),
                   "ironwood_actions": blocks.iter().flat_map(|b| &b.vtx).map(|t| t.ironwood_actions.len()).sum::<usize>(),
                   "orchard_actions": blocks.iter().flat_map(|b| &b.vtx).map(|t| t.actions.len()).sum::<usize>(), "ms": t_blocks},
    })
    .to_string())
}

/// What the proving key build is made of: the Halo 2 parameters (`Params::new(11)`), and how
/// long reading the same parameters back from bytes takes. orchard builds its parameters
/// inside `ProvingKey::build` and offers no way to pass them in, so this is measured with
/// halo2_proofs directly.
#[wasm_bindgen]
pub fn params_bench() -> String {
    use halo2_proofs::poly::commitment::Params;
    use pasta_curves::vesta;
    let t = now_ms();
    let p: Params<vesta::Affine> = Params::new(11);
    let t_new = now_ms() - t;
    let mut buf = vec![];
    p.write(&mut buf).unwrap();
    let t = now_ms();
    let q = Params::<vesta::Affine>::read(&mut &buf[..]).unwrap();
    let t_read = now_ms() - t;
    let mut buf2 = vec![];
    q.write(&mut buf2).unwrap();
    serde_json::json!({"params_new_ms": t_new, "params_read_ms": t_read, "params_bytes": buf.len(), "round_trip_equal": buf == buf2}).to_string()
}
