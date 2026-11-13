//! The two chain backends of the wallet, both over the `Transport` interface:
//! Zebra JSON-RPC (local regtest) and lightwalletd over gRPC-web (public networks).
//!
//! In `node/src/wallet.rs` the second one is a tonic gRPC client on a tokio runtime.
//! Browsers cannot speak plain gRPC, so here it is gRPC-web: the same protobuf messages
//! (from `zcash_client_backend::proto`), framed by hand and sent as one HTTP POST.

use anyhow::{anyhow, bail, Context, Result};
use incrementalmerkletree::frontier::Frontier;
use orchard::tree::MerkleHashOrchard;
use prost::Message;
use serde_json::{json, Value};
use zcash_client_backend::proto::{
    compact_formats::CompactBlock,
    service::{BlockId, BlockRange, ChainSpec, Empty, LightdInfo, PoolType, RawTransaction, SendResponse, TreeState, TxFilter},
};

use crate::net::{ensure_ok, Transport};

// ---------------------------------------------------------------- Zebra JSON-RPC

pub struct Rpc {
    t: Box<dyn Transport>,
    url: String,
}

impl Rpc {
    pub fn new(t: Box<dyn Transport>, url: &str) -> Self {
        Rpc { t, url: url.to_string() }
    }
    pub fn call_raw(&self, method: &str, params: Value) -> Result<std::result::Result<Value, Value>> {
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let r = self
            .t
            .post(&self.url, &[("content-type", "application/json")], req.to_string().as_bytes())
            .with_context(|| format!("zebra rpc {method}"))?;
        // Zebra answers JSON-RPC errors with a non-200 status and a JSON body.
        let resp: Value = serde_json::from_slice(&r.body)
            .with_context(|| format!("zebra rpc {method}: HTTP {} {}", r.status, String::from_utf8_lossy(&r.body[..r.body.len().min(200)])))?;
        if let Some(e) = resp.get("error").filter(|e| !e.is_null()) {
            return Ok(Err(e.clone()));
        }
        Ok(Ok(resp["result"].clone()))
    }
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_raw(method, params)?.map_err(|e| anyhow!("zebra rpc {method}: {e}"))
    }
}

// ---------------------------------------------------------------- lightwalletd over gRPC-web

const SERVICE: &str = "cash.z.wallet.sdk.rpc.CompactTxStreamer";

pub struct Lwd {
    t: Box<dyn Transport>,
    base: String,
    pub birthday: u64,
}

/// Split a gRPC-web response body into data messages; check the trailer's grpc-status.
pub fn grpc_web_decode(body: &[u8], header_status: Option<&str>) -> Result<Vec<Vec<u8>>> {
    let mut msgs = vec![];
    let mut status: Option<String> = header_status.map(str::to_string);
    let mut message = String::new();
    let mut i = 0;
    while i < body.len() {
        if body.len() < i + 5 {
            bail!("truncated gRPC-web frame header");
        }
        let flag = body[i];
        let len = u32::from_be_bytes(body[i + 1..i + 5].try_into().unwrap()) as usize;
        let end = i + 5 + len;
        if body.len() < end {
            bail!("truncated gRPC-web frame");
        }
        let payload = &body[i + 5..end];
        if flag & 0x80 != 0 {
            for line in String::from_utf8_lossy(payload).split("\r\n") {
                if let Some((k, v)) = line.split_once(':') {
                    match k.trim().to_ascii_lowercase().as_str() {
                        "grpc-status" => status = Some(v.trim().to_string()),
                        "grpc-message" => message = v.trim().to_string(),
                        _ => {}
                    }
                }
            }
        } else if flag == 0 {
            msgs.push(payload.to_vec());
        } else {
            bail!("unsupported gRPC-web frame flag {flag:#x} (compressed?)");
        }
        i = end;
    }
    match status.as_deref() {
        Some("0") => Ok(msgs),
        Some(s) => bail!("grpc-status {s}: {message}"),
        None => bail!("gRPC-web response without grpc-status"),
    }
}

impl Lwd {
    pub fn new(t: Box<dyn Transport>, base: &str, birthday: u64) -> Lwd {
        Lwd { t, base: base.trim_end_matches('/').to_string(), birthday }
    }

    /// One gRPC-web call; returns every data message (one for unary, many for streams).
    fn call<Req: Message>(&self, method: &str, req: &Req) -> Result<Vec<Vec<u8>>> {
        let msg = req.encode_to_vec();
        let mut body = Vec::with_capacity(msg.len() + 5);
        body.push(0);
        body.extend_from_slice(&(msg.len() as u32).to_be_bytes());
        body.extend_from_slice(&msg);
        let url = format!("{}/{SERVICE}/{method}", self.base);
        let r = self
            .t
            .post(&url, &[("content-type", "application/grpc-web+proto"), ("x-grpc-web", "1"), ("accept", "application/grpc-web+proto")], &body)
            .with_context(|| format!("gRPC-web {method}"))?;
        ensure_ok(&r, method)?;
        grpc_web_decode(&r.body, r.header("grpc-status")).with_context(|| format!("gRPC-web {method}"))
    }

    fn unary<Req: Message, Resp: Message + Default>(&self, method: &str, req: &Req) -> Result<Resp> {
        let msgs = self.call(method, req)?;
        let m = msgs.first().ok_or_else(|| anyhow!("{method}: empty response"))?;
        Ok(Resp::decode(&m[..])?)
    }

    pub fn info(&self) -> Result<LightdInfo> {
        self.unary("GetLightdInfo", &Empty {})
    }
    pub fn tip(&self) -> Result<u64> {
        Ok(self.unary::<_, BlockId>("GetLatestBlock", &ChainSpec {})?.height)
    }
    pub fn tree_state(&self, height: u64) -> Result<TreeState> {
        self.unary("GetTreeState", &BlockId { height, hash: vec![] })
    }
    pub fn tree_before(&self, height: u64) -> Result<Frontier<MerkleHashOrchard, 32>> {
        Ok(self.tree_state(height)?.ironwood_tree()?.to_frontier())
    }
    pub fn blocks(&self, lo: u64, hi: u64) -> Result<Vec<CompactBlock>> {
        let req = BlockRange {
            start: Some(BlockId { height: lo, hash: vec![] }),
            end: Some(BlockId { height: hi, hash: vec![] }),
            pool_types: vec![PoolType::Ironwood as i32],
        };
        self.call("GetBlockRange", &req)?.iter().map(|m| Ok(CompactBlock::decode(&m[..])?)).collect()
    }
    pub fn raw_tx(&self, txid_internal: &[u8]) -> Result<Vec<u8>> {
        Ok(self.unary::<_, RawTransaction>("GetTransaction", &TxFilter { block: None, index: 0, hash: txid_internal.to_vec() })?.data)
    }
    pub fn send(&self, data: Vec<u8>) -> Result<()> {
        let r: SendResponse = self.unary("SendTransaction", &RawTransaction { data, height: 0 })?;
        if r.error_code != 0 {
            bail!("lightwalletd rejected tx: {} {}", r.error_code, r.error_message);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(flag: u8, p: &[u8]) -> Vec<u8> {
        let mut v = vec![flag];
        v.extend_from_slice(&(p.len() as u32).to_be_bytes());
        v.extend_from_slice(p);
        v
    }

    #[test]
    fn decodes_stream_and_trailer() {
        let a = BlockId { height: 7, hash: vec![1, 2] }.encode_to_vec();
        let b = BlockId { height: 8, hash: vec![] }.encode_to_vec();
        let mut body = frame(0, &a);
        body.extend(frame(0, &b));
        body.extend(frame(0x80, b"grpc-status:0\r\ngrpc-message:\r\n"));
        let msgs = grpc_web_decode(&body, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(BlockId::decode(&msgs[1][..]).unwrap().height, 8);
    }

    #[test]
    fn reports_error_status_and_missing_status() {
        let body = frame(0x80, b"grpc-status:5\r\ngrpc-message:not found\r\n");
        let e = grpc_web_decode(&body, None).unwrap_err().to_string();
        assert!(e.contains("grpc-status 5") && e.contains("not found"), "{e}");
        assert!(grpc_web_decode(&frame(0, b"x"), None).is_err());
        // trailers-only responses carry the status in the HTTP headers
        assert!(grpc_web_decode(&[], Some("0")).unwrap().is_empty());
        assert!(grpc_web_decode(&[1, 0, 0], None).is_err());
    }
}
