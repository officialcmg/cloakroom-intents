//! The 1Click Swap API (`https://1click.chaindefuser.com`): request builders and
//! response parsers. Field names follow the OpenAPI spec at
//! `https://1click.chaindefuser.com/docs/v0/openapi.yaml` (version 0.1.10) and the
//! responses recorded in `tests/fixtures/`.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::http::{HttpRequest, HttpResponse, parse_json};
use crate::payload::{SignedPayload, Standard};
use crate::time::{self, Time};

pub const DEFAULT_BASE_URL: &str = "https://1click.chaindefuser.com";

/// The Verifier contract on NEAR mainnet.
pub const VERIFIER_MAINNET: &str = "intents.near";

/// Native USDC on NEAR as a 1Click asset id (Circle's NEP-141 contract).
pub const USDC_NEAR_ASSET: &str =
    "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";

/// Where to send 1Click requests, and the optional partner key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneClick {
    pub base_url: String,
    /// Partner JWT from `partners.near-intents.org`, sent as `X-API-Key`. Optional:
    /// every endpoint this crate uses answered without a key on 2026-11-13; with no
    /// key the API adds its own app fee to each quote (see `docs/INTENTS.md`).
    pub api_key: Option<String>,
}

impl Default for OneClick {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            api_key: None,
        }
    }
}

/// The parameters of an exit quote: USDC held inside the Verifier by the taker's key,
/// to `destination_asset` delivered to `recipient` on the destination chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitQuoteParams {
    /// The taker's Verifier account (hex of the ed25519 public key). Refunds go here.
    pub signer_id: String,
    /// Usually [`USDC_NEAR_ASSET`].
    pub origin_asset: String,
    /// In the origin asset's smallest unit (USDC: 6 decimals).
    pub amount: u128,
    pub destination_asset: String,
    /// An address on the destination asset's chain.
    pub recipient: String,
    /// Slippage tolerance in basis points (100 = 1 %).
    pub slippage_bps: u32,
    /// When the swap must be done; after it, 1Click refunds into `signer_id`.
    pub deadline: Time,
    /// `true`: price only, no deposit address. `false`: a firm quote.
    pub dry: bool,
    pub referral: Option<String>,
    /// How long to wait for solver quotes. `None` lets the API use its default (0 ms).
    pub quote_waiting_time_ms: Option<u64>,
}

/// An application fee in a quote request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppFee {
    pub recipient: String,
    /// Basis points of `amountIn`.
    pub fee: u32,
}

/// `POST /v0/quote` body. Also the `quoteRequest` echoed in quote and status answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRequest {
    pub dry: bool,
    pub swap_type: String,
    pub slippage_tolerance: serde_json::Number,
    pub origin_asset: String,
    pub deposit_type: String,
    pub destination_asset: String,
    pub amount: String,
    pub refund_to: String,
    pub refund_type: String,
    pub recipient: String,
    pub recipient_type: String,
    pub deadline: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deposit_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_waiting_time_ms: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referral: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_fees: Option<Vec<AppFee>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_chain_recipient: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_chain_refund_recipient: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_recipient_msg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidentiality: Option<String>,
}

impl QuoteRequest {
    /// The exit quote: `depositType: INTENTS`, `refundType: INTENTS` (refunds stay
    /// under the taker's key inside the Verifier), `recipientType: DESTINATION_CHAIN`,
    /// `swapType: EXACT_INPUT`.
    pub fn exit(p: &ExitQuoteParams) -> Self {
        Self {
            dry: p.dry,
            swap_type: "EXACT_INPUT".into(),
            slippage_tolerance: p.slippage_bps.into(),
            origin_asset: p.origin_asset.clone(),
            deposit_type: "INTENTS".into(),
            destination_asset: p.destination_asset.clone(),
            amount: p.amount.to_string(),
            refund_to: p.signer_id.clone(),
            refund_type: "INTENTS".into(),
            recipient: p.recipient.clone(),
            recipient_type: "DESTINATION_CHAIN".into(),
            deadline: time::format(p.deadline),
            deposit_mode: None,
            quote_waiting_time_ms: p.quote_waiting_time_ms.map(Into::into),
            referral: p.referral.clone(),
            app_fees: None,
            virtual_chain_recipient: None,
            virtual_chain_refund_recipient: None,
            custom_recipient_msg: None,
            confidentiality: None,
        }
    }

    /// Sum of the app fees in basis points. With no API key, 1Click adds its own app
    /// fee here (1 bp on the USDC to USDC route, 20 bp to SOL and ETH on 2026-11-13).
    pub fn app_fee_bps(&self) -> u32 {
        self.app_fees
            .as_ref()
            .map(|f| f.iter().map(|a| a.fee).sum())
            .unwrap_or(0)
    }
}

/// The `quote` object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub amount_in: String,
    #[serde(default)]
    pub amount_in_formatted: Option<String>,
    #[serde(default)]
    pub amount_in_usd: Option<String>,
    pub min_amount_in: String,
    pub amount_out: String,
    #[serde(default)]
    pub amount_out_formatted: Option<String>,
    #[serde(default)]
    pub amount_out_usd: Option<String>,
    pub min_amount_out: String,
    /// Seconds from deposit to delivery, as estimated by 1Click.
    #[serde(default)]
    pub time_estimate: Option<f64>,
    /// Firm quotes only: when the deposit address stops working. Measured: the
    /// request deadline plus 72 hours.
    #[serde(default)]
    pub deadline: Option<String>,
    #[serde(default)]
    pub time_when_inactive: Option<String>,
    /// Firm quotes only: for `depositType: INTENTS`, a Verifier account (64 hex).
    #[serde(default)]
    pub deposit_address: Option<String>,
    #[serde(default)]
    pub deposit_memo: Option<String>,
    #[serde(default)]
    pub refund_fee: Option<String>,
    /// Already subtracted from `amountOut`.
    #[serde(default)]
    pub withdraw_fee: Option<String>,
}

pub(crate) fn amount(s: &str, what: &str) -> Result<u128> {
    s.parse::<u128>()
        .map_err(|e| Error::Parse(format!("{what} {s:?}: {e}")))
}

impl Quote {
    pub fn amount_in(&self) -> Result<u128> {
        amount(&self.amount_in, "amountIn")
    }
    pub fn amount_out(&self) -> Result<u128> {
        amount(&self.amount_out, "amountOut")
    }
    /// The guaranteed minimum: below it 1Click refunds instead of swapping.
    pub fn min_amount_out(&self) -> Result<u128> {
        amount(&self.min_amount_out, "minAmountOut")
    }
    pub fn deadline(&self) -> Result<Option<Time>> {
        self.deadline.as_deref().map(time::parse).transpose()
    }
}

/// A `POST /v0/quote` answer. `raw` keeps the answer as received; the 1Click
/// signature is checked against it ([`crate::quote_sig`]).
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteResponse {
    pub correlation_id: Option<String>,
    pub timestamp: String,
    /// `ed25519:<base58>`, by 1Click's quote key.
    pub signature: String,
    pub quote_request: QuoteRequest,
    pub quote: Quote,
    pub raw: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteResponseJson {
    #[serde(default)]
    correlation_id: Option<String>,
    timestamp: String,
    signature: String,
    quote_request: QuoteRequest,
    quote: Quote,
}

impl QuoteResponse {
    pub fn from_value(raw: Value) -> Result<Self> {
        let j: QuoteResponseJson = serde_json::from_value(raw.clone())
            .map_err(|e| Error::Parse(format!("quote response: {e}")))?;
        Ok(Self {
            correlation_id: j.correlation_id,
            timestamp: j.timestamp,
            signature: j.signature,
            quote_request: j.quote_request,
            quote: j.quote,
            raw,
        })
    }

    /// Checks the 1Click signature with the key in the official SDK.
    pub fn signature_is_valid(&self) -> bool {
        crate::quote_sig::verify(&self.raw, &crate::quote_sig::ONE_CLICK_QUOTE_KEY)
    }
}

impl<'de> Deserialize<'de> for QuoteResponse {
    fn deserialize<D: Deserializer<'de>>(d: D) -> core::result::Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Self::from_value(v).map_err(serde::de::Error::custom)
    }
}

/// One entry of `GET /v0/tokens`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    /// For example `nep141:sol.omft.near`, `nep245:v2_1.omni.hot.tg:…` or
    /// `1cs_v1:…`. Treat it as an opaque string.
    pub asset_id: String,
    pub decimals: u32,
    pub blockchain: String,
    pub symbol: String,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub price_updated_at: Option<String>,
    #[serde(default)]
    pub contract_address: Option<String>,
}

/// The unsigned intent from `POST /v0/generate-intent`. Its payload is not trusted:
/// [`crate::check::check_generated_intent`] decodes and checks it before anything is
/// signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedIntent {
    pub standard: String,
    /// A string for `raw_ed25519`, an object for `nep413`.
    pub payload: Value,
    pub correlation_id: Option<String>,
}

/// The answer of `POST /v0/submit-intent`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitIntentResponse {
    /// Base58 SHA-256 intent hash. The API returned it also for a payload with a wrong
    /// signature; acceptance here does not mean the intent is valid.
    pub intent_hash: String,
    #[serde(default)]
    pub correlation_id: Option<String>,
}

/// The `status` field of `GET /v0/status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwapStatus {
    KnownDepositTx,
    PendingDeposit,
    IncompleteDeposit,
    Processing,
    Success,
    Refunded,
    Failed,
    Unknown(String),
}

impl SwapStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "KNOWN_DEPOSIT_TX" => Self::KnownDepositTx,
            "PENDING_DEPOSIT" => Self::PendingDeposit,
            "INCOMPLETE_DEPOSIT" => Self::IncompleteDeposit,
            "PROCESSING" => Self::Processing,
            "SUCCESS" => Self::Success,
            "REFUNDED" => Self::Refunded,
            "FAILED" => Self::Failed,
            other => Self::Unknown(other.into()),
        }
    }

    /// `SUCCESS`, `REFUNDED` and `FAILED` do not change any more.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Success | Self::Refunded | Self::Failed)
    }
}

impl<'de> Deserialize<'de> for SwapStatus {
    fn deserialize<D: Deserializer<'de>>(d: D) -> core::result::Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(d)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionDetails {
    pub hash: String,
    #[serde(default)]
    pub explorer_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SwapDetails {
    pub intent_hashes: Vec<String>,
    pub near_tx_hashes: Vec<String>,
    pub amount_in: Option<String>,
    pub amount_out: Option<String>,
    pub amount_out_formatted: Option<String>,
    pub slippage: Option<f64>,
    pub origin_chain_tx_hashes: Vec<TransactionDetails>,
    pub destination_chain_tx_hashes: Vec<TransactionDetails>,
    pub refunded_amount: Option<String>,
    pub refund_reason: Option<String>,
    pub deposited_amount: Option<String>,
    pub withdraw_fee: Option<String>,
}

/// The answer of `GET /v0/status`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub status: SwapStatus,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub swap_details: SwapDetails,
    #[serde(default)]
    pub quote_response: Option<Value>,
    #[serde(default)]
    pub correlation_id: Option<String>,
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl OneClick {
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
            ..Self::default()
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth(&self, r: HttpRequest) -> HttpRequest {
        match &self.api_key {
            Some(k) => r.with_header("x-api-key", k),
            None => r,
        }
    }

    /// `GET /v0/tokens`.
    pub fn tokens_request(&self) -> HttpRequest {
        self.auth(HttpRequest::get(self.url("/v0/tokens")))
    }

    /// `POST /v0/quote`.
    pub fn quote_request(&self, q: &QuoteRequest) -> HttpRequest {
        let body = serde_json::to_value(q).unwrap_or(Value::Null);
        self.auth(HttpRequest::post_json(self.url("/v0/quote"), &body))
    }

    /// `POST /v0/generate-intent` for a firm quote's deposit address.
    pub fn generate_intent_request(
        &self,
        deposit_address: &str,
        signer_id: &str,
        standard: Standard,
    ) -> HttpRequest {
        let body = json!({
            "type": "swap_transfer",
            "standard": standard.as_str(),
            "signerId": signer_id,
            "depositAddress": deposit_address,
        });
        self.auth(HttpRequest::post_json(
            self.url("/v0/generate-intent"),
            &body,
        ))
    }

    /// `POST /v0/submit-intent`.
    pub fn submit_intent_request(&self, signed: &SignedPayload) -> HttpRequest {
        let body = json!({ "type": "swap_transfer", "signedData": signed.to_json() });
        self.auth(HttpRequest::post_json(self.url("/v0/submit-intent"), &body))
    }

    /// `GET /v0/status?depositAddress=…[&depositMemo=…]`.
    pub fn status_request(&self, deposit_address: &str, deposit_memo: Option<&str>) -> HttpRequest {
        let mut url = self.url(&format!(
            "/v0/status?depositAddress={}",
            percent_encode(deposit_address)
        ));
        if let Some(m) = deposit_memo {
            url.push_str(&format!("&depositMemo={}", percent_encode(m)));
        }
        self.auth(HttpRequest::get(url))
    }
}

pub fn parse_tokens(r: &HttpResponse) -> Result<Vec<TokenInfo>> {
    parse_json(r.ok_body()?, "tokens")
}

pub fn parse_quote(r: &HttpResponse) -> Result<QuoteResponse> {
    let v: Value = parse_json(r.ok_body()?, "quote")?;
    QuoteResponse::from_value(v)
}

pub fn parse_generated_intent(r: &HttpResponse) -> Result<GeneratedIntent> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Inner {
        standard: String,
        payload: Value,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Outer {
        intent: Inner,
        #[serde(default)]
        correlation_id: Option<String>,
    }
    let o: Outer = parse_json(r.ok_body()?, "generate-intent")?;
    Ok(GeneratedIntent {
        standard: o.intent.standard,
        payload: o.intent.payload,
        correlation_id: o.correlation_id,
    })
}

pub fn parse_submit_intent(r: &HttpResponse) -> Result<SubmitIntentResponse> {
    parse_json(r.ok_body()?, "submit-intent")
}

pub fn parse_status(r: &HttpResponse) -> Result<StatusResponse> {
    parse_json(r.ok_body()?, "status")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Method;

    fn params(dry: bool) -> ExitQuoteParams {
        ExitQuoteParams {
            signer_id: "0d7550754e0800a5d237eef5826035766b9b3e5a15868a940ab289958788e3b0".into(),
            origin_asset: USDC_NEAR_ASSET.into(),
            amount: 5_000_000,
            destination_asset: "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near"
                .into(),
            recipient: "0x1111111111111111111111111111111111111111".into(),
            slippage_bps: 100,
            deadline: time::parse("2026-11-13T03:33:00.000Z").unwrap(),
            dry,
            referral: None,
            quote_waiting_time_ms: None,
        }
    }

    #[test]
    fn quote_body_has_the_exit_shape() {
        let r = OneClick::default().quote_request(&QuoteRequest::exit(&params(false)));
        assert_eq!(r.method, Method::Post);
        assert_eq!(r.url, "https://1click.chaindefuser.com/v0/quote");
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.header("x-api-key"), None);
        assert_eq!(
            r.json_body().unwrap(),
            json!({
                "dry": false,
                "swapType": "EXACT_INPUT",
                "slippageTolerance": 100,
                "originAsset": USDC_NEAR_ASSET,
                "depositType": "INTENTS",
                "destinationAsset": "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near",
                "amount": "5000000",
                "refundTo": "0d7550754e0800a5d237eef5826035766b9b3e5a15868a940ab289958788e3b0",
                "refundType": "INTENTS",
                "recipient": "0x1111111111111111111111111111111111111111",
                "recipientType": "DESTINATION_CHAIN",
                "deadline": "2026-11-13T03:33:00.000Z",
            })
        );
        let dry = OneClick::default().quote_request(&QuoteRequest::exit(&params(true)));
        assert_eq!(dry.json_body().unwrap()["dry"], json!(true));
    }

    #[test]
    fn optional_fields_and_api_key() {
        let mut p = params(true);
        p.referral = Some("zecret".into());
        p.quote_waiting_time_ms = Some(3000);
        let r = OneClick::with_api_key("jwt").quote_request(&QuoteRequest::exit(&p));
        assert_eq!(r.header("x-api-key"), Some("jwt"));
        let b = r.json_body().unwrap();
        assert_eq!(b["referral"], json!("zecret"));
        assert_eq!(b["quoteWaitingTimeMs"], json!(3000));
    }

    #[test]
    fn generate_submit_status_and_tokens_requests() {
        let api = OneClick {
            base_url: "http://localhost:9/".into(),
            api_key: None,
        };
        let g = api.generate_intent_request("dep", "signer", Standard::Nep413);
        assert_eq!(g.url, "http://localhost:9/v0/generate-intent");
        assert_eq!(
            g.json_body().unwrap(),
            json!({"type":"swap_transfer","standard":"nep413","signerId":"signer","depositAddress":"dep"})
        );
        let s = api.status_request("ab cd", Some("m&1"));
        assert_eq!(s.method, Method::Get);
        assert_eq!(
            s.url,
            "http://localhost:9/v0/status?depositAddress=ab%20cd&depositMemo=m%261"
        );
        assert!(s.body.is_none());
        let t = api.tokens_request();
        assert_eq!(t.url, "http://localhost:9/v0/tokens");
        assert_eq!(t.method, Method::Get);
    }

    #[test]
    fn status_values() {
        assert_eq!(SwapStatus::parse("SUCCESS"), SwapStatus::Success);
        assert!(SwapStatus::parse("REFUNDED").is_final());
        assert!(SwapStatus::parse("FAILED").is_final());
        assert!(!SwapStatus::parse("PENDING_DEPOSIT").is_final());
        assert!(!SwapStatus::parse("PROCESSING").is_final());
        assert!(!SwapStatus::parse("KNOWN_DEPOSIT_TX").is_final());
        assert!(!SwapStatus::parse("INCOMPLETE_DEPOSIT").is_final());
        assert_eq!(
            SwapStatus::parse("NEW_THING"),
            SwapStatus::Unknown("NEW_THING".into())
        );
    }

    #[test]
    fn amounts_parse_strictly() {
        assert_eq!(amount("4996757", "x").unwrap(), 4_996_757);
        assert!(amount("4.99", "x").is_err());
        assert!(amount("-1", "x").is_err());
    }
}
