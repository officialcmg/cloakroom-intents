//! The exit flow over a caller-provided [`HttpTransport`].
//!
//! Each method sends one request (or a fixed short sequence) and returns. Nothing
//! sleeps or loops: the caller polls [`ExitClient::status`] on its own schedule.

use crate::check::{self, ApprovedExit, CheckedIntent, Policy};
use crate::direct;
use crate::error::{Error, Result};
use crate::http::{HttpTransport, send};
use crate::nonce::{Nonce, Salt};
use crate::oneclick::{self, ExitQuoteParams, OneClick, QuoteRequest, QuoteResponse, StatusResponse, TokenInfo};
use crate::payload::{SignedPayload, Standard};
use crate::signer::IntentSigner;
use crate::time::Time;

/// A signed exit that 1Click accepted.
#[derive(Clone, Debug, PartialEq)]
pub struct SubmittedExit {
    pub quote: QuoteResponse,
    pub checked: CheckedIntent,
    pub signed: SignedPayload,
    /// As returned by `/v0/submit-intent`; equal to `checked.intent_hash`.
    pub intent_hash: String,
}

impl SubmittedExit {
    pub fn deposit_address(&self) -> &str {
        &self.checked.deposit_address
    }
}

/// 1Click calls over a transport.
pub struct ExitClient<T> {
    pub transport: T,
    pub api: OneClick,
    pub policy: Policy,
}

impl<T: HttpTransport> ExitClient<T> {
    pub fn new(transport: T, api: OneClick) -> Self {
        Self {
            transport,
            api,
            policy: Policy::default(),
        }
    }

    pub fn tokens(&self) -> Result<Vec<TokenInfo>> {
        let r = send(&self.transport, &self.api.tokens_request())?;
        oneclick::parse_tokens(&r)
    }

    /// A quote as requested (dry or firm). The signature is checked; a quote that is
    /// not signed by 1Click is an error.
    pub fn quote(&self, params: &ExitQuoteParams) -> Result<QuoteResponse> {
        let req = QuoteRequest::exit(params);
        let r = send(&self.transport, &self.api.quote_request(&req))?;
        let q = oneclick::parse_quote(&r)?;
        if !crate::quote_sig::verify(&q.raw, &self.policy.quote_key) {
            return Err(Error::Refused(check::Refusal::QuoteSignature));
        }
        Ok(q)
    }

    /// The whole signed exit, in one call:
    ///
    /// 1. a firm quote for the approved exit (deadline `quote_deadline`);
    /// 2. the quote check (signature, fields, guaranteed minimum);
    /// 3. `/v0/generate-intent` for the taker's key;
    /// 4. the pre-signing check of the returned payload;
    /// 5. the signature;
    /// 6. `/v0/submit-intent`, and a check that the returned hash is ours.
    ///
    /// Any refusal returns `Error::Refused` before step 5, and nothing is signed.
    pub fn execute_exit(
        &self,
        signer: &IntentSigner,
        approved: &ApprovedExit,
        slippage_bps: u32,
        quote_deadline: Time,
        standard: Standard,
        now: Time,
    ) -> Result<SubmittedExit> {
        if signer.account_id() != approved.signer_id {
            return Err(Error::Invalid(
                "the approved signer is not this key's account".into(),
            ));
        }
        let params = ExitQuoteParams {
            signer_id: approved.signer_id.clone(),
            origin_asset: approved.origin_asset.clone(),
            amount: approved.amount_in,
            destination_asset: approved.destination_asset.clone(),
            recipient: approved.recipient.clone(),
            slippage_bps,
            deadline: quote_deadline,
            dry: false,
            referral: None,
            quote_waiting_time_ms: None,
        };
        let quote = self.quote(&params)?;
        check::check_quote(approved, &quote, &self.policy, now)?;
        let deposit = quote.quote.deposit_address.clone().unwrap_or_default();

        let r = send(
            &self.transport,
            &self
                .api
                .generate_intent_request(&deposit, &approved.signer_id, standard),
        )?;
        let generated = oneclick::parse_generated_intent(&r)?;
        let checked = check::check_generated_intent(
            approved,
            &quote,
            &generated,
            standard,
            &self.policy,
            now,
        )?;
        let signed = signer.sign(&checked.payload)?;

        let r = send(&self.transport, &self.api.submit_intent_request(&signed))?;
        let submitted = oneclick::parse_submit_intent(&r)?;
        if submitted.intent_hash != checked.intent_hash {
            return Err(Error::Parse(format!(
                "submit-intent returned hash {}, the signed intent has {}",
                submitted.intent_hash, checked.intent_hash
            )));
        }
        Ok(SubmittedExit {
            quote,
            checked,
            intent_hash: submitted.intent_hash,
            signed,
        })
    }

    /// One `/v0/status` call.
    pub fn status(&self, deposit_address: &str) -> Result<StatusResponse> {
        let r = send(&self.transport, &self.api.status_request(deposit_address, None))?;
        oneclick::parse_status(&r)
    }
}

/// NEAR RPC calls for the direct path.
pub struct VerifierRpc<T> {
    pub transport: T,
    pub rpc_url: String,
    pub verifier: String,
}

impl<T: HttpTransport> VerifierRpc<T> {
    pub fn current_salt(&self) -> Result<Salt> {
        let r = send(
            &self.transport,
            &direct::current_salt_request(&self.rpc_url, &self.verifier),
        )?;
        direct::parse_current_salt(&r)
    }

    pub fn balance(&self, account_id: &str, token_id: &str) -> Result<u128> {
        let r = send(
            &self.transport,
            &direct::balance_request(&self.rpc_url, &self.verifier, account_id, token_id),
        )?;
        direct::parse_balance(&r)
    }

    pub fn nonce_used(&self, account_id: &str, nonce: &Nonce) -> Result<bool> {
        let r = send(
            &self.transport,
            &direct::nonce_used_request(&self.rpc_url, &self.verifier, account_id, nonce),
        )?;
        direct::parse_nonce_used(&r)
    }

    /// Runs `simulate_intents`; `Ok` means the intents would execute now.
    pub fn simulate(&self, signed: &[SignedPayload]) -> Result<serde_json::Value> {
        let r = send(
            &self.transport,
            &direct::simulate_request(&self.rpc_url, &self.verifier, signed),
        )?;
        direct::parse_view_result(&r)
    }
}
