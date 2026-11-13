//! The pre-signing check: sign a 1Click intent only if it is what the user approved.
//!
//! `/v0/generate-intent` returns a payload for the taker's key to sign. The API is a
//! third party and the connection may be intercepted, so the payload is not trusted.
//! Before signing, this module checks two things:
//!
//! 1. **The quote.** 1Click's signature over the quote is valid, and the signed quote
//!    says what the user approved: origin asset and amount, destination asset,
//!    recipient, refunds to the taker's own Verifier account, a minimum output not
//!    below the approved minimum, a firm (not dry) quote with a deposit address.
//! 2. **The payload.** Decoded strictly (unknown fields or intent types are refused):
//!    the verifying contract, the signer, a deadline that is in the future and not too
//!    far away, and exactly one `transfer` intent that moves exactly the approved
//!    amount of exactly the approved token to exactly the quote's deposit address,
//!    with no memo and no contract call.
//!
//! Only a payload that passes becomes a [`CheckedIntent`], and only a checked intent's
//! payload can be signed (an [`UnsignedPayload`] cannot be built outside this crate).

use core::fmt;

use chrono::Duration;

use crate::oneclick::{GeneratedIntent, QuoteResponse, VERIFIER_MAINNET, amount};
use crate::payload::{Intent, Nep413Payload, Standard, UnsignedPayload};
use crate::quote_sig;
use crate::time::{self, Time};

/// What the user approved on the page. The app fills it from the dry quote the user
/// saw (or from the firm quote, after showing it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedExit {
    /// The taker's Verifier account (hex of the ed25519 public key).
    pub signer_id: String,
    /// `intents.near` on mainnet.
    pub verifying_contract: String,
    /// Usually [`crate::oneclick::USDC_NEAR_ASSET`].
    pub origin_asset: String,
    pub amount_in: u128,
    pub destination_asset: String,
    pub recipient: String,
    /// The least the user accepted to receive, in the destination asset's smallest
    /// unit. The firm quote's `minAmountOut` must not be lower.
    pub min_amount_out: u128,
}

impl ApprovedExit {
    /// An exit on mainnet (`intents.near`).
    pub fn mainnet(
        signer_id: impl Into<String>,
        origin_asset: impl Into<String>,
        amount_in: u128,
        destination_asset: impl Into<String>,
        recipient: impl Into<String>,
        min_amount_out: u128,
    ) -> Self {
        Self {
            signer_id: signer_id.into(),
            verifying_contract: VERIFIER_MAINNET.into(),
            origin_asset: origin_asset.into(),
            amount_in,
            destination_asset: destination_asset.into(),
            recipient: recipient.into(),
            min_amount_out,
        }
    }
}

/// Limits that do not come from the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    /// The key that must have signed the quote.
    pub quote_key: [u8; 32],
    /// The longest a signed intent may stay valid, counted from now. 1Click sets the
    /// intent deadline to the quote's deposit deadline, which was the request deadline
    /// plus 72 hours in every quote measured; the default allows 96 hours.
    pub max_intent_validity: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            quote_key: quote_sig::ONE_CLICK_QUOTE_KEY,
            max_intent_validity: Duration::hours(96),
        }
    }
}

/// Why the check refused to sign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The quote is not signed by 1Click's quote key, or was changed after signing.
    QuoteSignature,
    /// A dry quote has no deposit address and cannot be executed.
    QuoteIsDry,
    /// A firm quote without a deposit address.
    NoDepositAddress,
    /// A quote field differs from what the user approved.
    QuoteMismatch {
        field: &'static str,
        expected: String,
        found: String,
    },
    /// A quote field that could redirect funds is set.
    QuoteHasUnexpectedField(&'static str),
    /// The quote's guaranteed minimum is below the approved minimum.
    MinimumTooLow { approved: u128, quoted: u128 },
    /// The quote's deposit deadline has passed.
    QuoteExpired { deadline: String },
    /// The API returned a payload for another standard than requested.
    WrongStandard { expected: String, found: String },
    /// The payload could not be decoded strictly.
    Malformed(String),
    WrongContract { expected: String, found: String },
    WrongSigner { expected: String, found: String },
    /// The intent deadline has passed.
    IntentExpired { deadline: String },
    /// The intent would stay valid longer than the policy or the quote allows.
    DeadlineTooFar { deadline: String, limit: String },
    /// Not exactly one intent.
    IntentCount(usize),
    /// An intent that is not a `transfer`.
    UnexpectedIntent(String),
    WrongReceiver { expected: String, found: String },
    WrongToken { expected: String, found: String },
    WrongAmount { expected: String, found: String },
    /// The transfer carries a memo.
    Memo,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuoteSignature => write!(f, "the quote is not signed by 1Click"),
            Self::QuoteIsDry => write!(f, "the quote is a dry quote"),
            Self::NoDepositAddress => write!(f, "the quote has no deposit address"),
            Self::QuoteMismatch {
                field,
                expected,
                found,
            } => write!(f, "quote {field} is {found:?}, approved {expected:?}"),
            Self::QuoteHasUnexpectedField(k) => write!(f, "quote sets {k}"),
            Self::MinimumTooLow { approved, quoted } => write!(
                f,
                "quote guarantees {quoted}, below the approved minimum {approved}"
            ),
            Self::QuoteExpired { deadline } => write!(f, "quote expired at {deadline}"),
            Self::WrongStandard { expected, found } => {
                write!(f, "payload standard {found}, requested {expected}")
            }
            Self::Malformed(m) => write!(f, "payload not understood: {m}"),
            Self::WrongContract { expected, found } => {
                write!(f, "verifying contract {found}, expected {expected}")
            }
            Self::WrongSigner { expected, found } => {
                write!(f, "signer {found}, expected {expected}")
            }
            Self::IntentExpired { deadline } => write!(f, "intent deadline {deadline} passed"),
            Self::DeadlineTooFar { deadline, limit } => {
                write!(f, "intent deadline {deadline} is after {limit}")
            }
            Self::IntentCount(n) => write!(f, "{n} intents, expected 1"),
            Self::UnexpectedIntent(k) => write!(f, "unexpected intent {k}"),
            Self::WrongReceiver { expected, found } => {
                write!(f, "transfer to {found}, expected {expected}")
            }
            Self::WrongToken { expected, found } => {
                write!(f, "transfer of {found}, expected {expected}")
            }
            Self::WrongAmount { expected, found } => {
                write!(f, "transfer amount {found}, expected {expected}")
            }
            Self::Memo => write!(f, "the transfer carries a memo"),
        }
    }
}

impl std::error::Error for Refusal {}

/// A 1Click intent that passed the check. Sign [`CheckedIntent::payload`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedIntent {
    pub payload: UnsignedPayload,
    pub deposit_address: String,
    pub deadline: Time,
    /// Base58; `/v0/submit-intent` must return the same value.
    pub intent_hash: String,
}

fn same(field: &'static str, expected: &str, found: &str) -> Result<(), Refusal> {
    if expected == found {
        Ok(())
    } else {
        Err(Refusal::QuoteMismatch {
            field,
            expected: expected.into(),
            found: found.into(),
        })
    }
}

/// Checks a firm quote against what the user approved.
pub fn check_quote(
    approved: &ApprovedExit,
    quote: &QuoteResponse,
    policy: &Policy,
    now: Time,
) -> Result<(), Refusal> {
    if !quote_sig::verify(&quote.raw, &policy.quote_key) {
        return Err(Refusal::QuoteSignature);
    }
    let r = &quote.quote_request;
    if r.dry {
        return Err(Refusal::QuoteIsDry);
    }
    same("swapType", "EXACT_INPUT", &r.swap_type)?;
    same("depositType", "INTENTS", &r.deposit_type)?;
    same("originAsset", &approved.origin_asset, &r.origin_asset)?;
    same("amount", &approved.amount_in.to_string(), &r.amount)?;
    same("destinationAsset", &approved.destination_asset, &r.destination_asset)?;
    same("recipientType", "DESTINATION_CHAIN", &r.recipient_type)?;
    same("recipient", &approved.recipient, &r.recipient)?;
    same("refundType", "INTENTS", &r.refund_type)?;
    same("refundTo", &approved.signer_id, &r.refund_to)?;
    if r.custom_recipient_msg.is_some() {
        return Err(Refusal::QuoteHasUnexpectedField("customRecipientMsg"));
    }
    if r.virtual_chain_recipient.is_some() {
        return Err(Refusal::QuoteHasUnexpectedField("virtualChainRecipient"));
    }
    if r.virtual_chain_refund_recipient.is_some() {
        return Err(Refusal::QuoteHasUnexpectedField("virtualChainRefundRecipient"));
    }
    if quote.quote.deposit_memo.is_some() {
        return Err(Refusal::QuoteHasUnexpectedField("depositMemo"));
    }

    let q = &quote.quote;
    let amount_in = amount(&q.amount_in, "amountIn").map_err(|e| Refusal::Malformed(e.to_string()))?;
    if amount_in != approved.amount_in {
        return Err(Refusal::QuoteMismatch {
            field: "quote.amountIn",
            expected: approved.amount_in.to_string(),
            found: q.amount_in.clone(),
        });
    }
    let quoted_min =
        amount(&q.min_amount_out, "minAmountOut").map_err(|e| Refusal::Malformed(e.to_string()))?;
    if quoted_min < approved.min_amount_out {
        return Err(Refusal::MinimumTooLow {
            approved: approved.min_amount_out,
            quoted: quoted_min,
        });
    }
    if q.deposit_address.as_deref().unwrap_or("").is_empty() {
        return Err(Refusal::NoDepositAddress);
    }
    // The swap must still be possible: the request deadline (after which 1Click
    // refunds) and the deposit deadline must both be in the future.
    for d in [Some(r.deadline.as_str()), q.deadline.as_deref()]
        .into_iter()
        .flatten()
    {
        let t = time::parse(d).map_err(|e| Refusal::Malformed(e.to_string()))?;
        if t <= now {
            return Err(Refusal::QuoteExpired { deadline: d.into() });
        }
    }
    Ok(())
}

/// Checks a generated intent against the quote and what the user approved. On
/// success the returned [`CheckedIntent`] holds the exact payload to sign.
pub fn check_generated_intent(
    approved: &ApprovedExit,
    quote: &QuoteResponse,
    generated: &GeneratedIntent,
    standard: Standard,
    policy: &Policy,
    now: Time,
) -> Result<CheckedIntent, Refusal> {
    check_quote(approved, quote, policy, now)?;
    let deposit_address = quote.quote.deposit_address.clone().unwrap_or_default();

    if generated.standard != standard.as_str() {
        return Err(Refusal::WrongStandard {
            expected: standard.as_str().into(),
            found: generated.standard.clone(),
        });
    }
    let payload = match standard {
        Standard::RawEd25519 => {
            let s = generated
                .payload
                .as_str()
                .ok_or_else(|| Refusal::Malformed("raw_ed25519 payload is not a string".into()))?;
            UnsignedPayload::raw(s.to_owned())
        }
        Standard::Nep413 => {
            let p: Nep413Payload = serde_json::from_value(generated.payload.clone())
                .map_err(|e| Refusal::Malformed(format!("nep413 payload: {e}")))?;
            UnsignedPayload::nep413(p)
        }
    };
    let d = payload
        .decode()
        .map_err(|e| Refusal::Malformed(e.to_string()))?;

    if d.verifying_contract != approved.verifying_contract {
        return Err(Refusal::WrongContract {
            expected: approved.verifying_contract.clone(),
            found: d.verifying_contract,
        });
    }
    if d.signer_id != approved.signer_id {
        return Err(Refusal::WrongSigner {
            expected: approved.signer_id.clone(),
            found: d.signer_id,
        });
    }
    if d.deadline <= now {
        return Err(Refusal::IntentExpired {
            deadline: time::format(d.deadline),
        });
    }
    let mut limit = now + policy.max_intent_validity;
    if let Ok(Some(qd)) = quote.quote.deadline() {
        limit = limit.min(qd);
    }
    if d.deadline > limit {
        return Err(Refusal::DeadlineTooFar {
            deadline: time::format(d.deadline),
            limit: time::format(limit),
        });
    }

    if d.intents.len() != 1 {
        return Err(Refusal::IntentCount(d.intents.len()));
    }
    let t = match &d.intents[0] {
        Intent::Transfer(t) => t,
        other => return Err(Refusal::UnexpectedIntent(other.kind().into())),
    };
    if t.receiver_id != deposit_address {
        return Err(Refusal::WrongReceiver {
            expected: deposit_address,
            found: t.receiver_id.clone(),
        });
    }
    if t.memo.is_some() {
        return Err(Refusal::Memo);
    }
    let mut tokens = t.tokens.iter();
    let (token, value) = match (tokens.next(), tokens.next()) {
        (Some(one), None) => one,
        _ => {
            return Err(Refusal::WrongToken {
                expected: approved.origin_asset.clone(),
                found: t.tokens.keys().cloned().collect::<Vec<_>>().join(","),
            });
        }
    };
    if *token != approved.origin_asset {
        return Err(Refusal::WrongToken {
            expected: approved.origin_asset.clone(),
            found: token.clone(),
        });
    }
    // Compare as numbers so that "05000000" cannot pass as 5000000 by accident, and a
    // non-number is refused.
    let found = value.parse::<u128>().ok();
    if found != Some(approved.amount_in) || *value != approved.amount_in.to_string() {
        return Err(Refusal::WrongAmount {
            expected: approved.amount_in.to_string(),
            found: value.clone(),
        });
    }

    let intent_hash = payload
        .intent_hash_base58()
        .map_err(|e| Refusal::Malformed(e.to_string()))?;
    Ok(CheckedIntent {
        payload,
        deposit_address,
        deadline: d.deadline,
        intent_hash,
    })
}
