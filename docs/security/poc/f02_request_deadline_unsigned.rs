// Proof of concept for review-1 finding F-02 (the quote freshness check reads an
// unsigned field).
//
// Where it goes: append this test to the end of
// crates/zecret-intents/tests/recorded.rs. It uses that file's helpers (both, Case,
// Refusal, json).
// Run: cd crates/zecret-intents && cargo test --test recorded poc_f02
//
// It asserts the invariant and FAILS on the reviewed code: after the quote's request
// deadline has passed, a network attacker edits `quoteRequest.deadline`; 1Click's
// signature stays valid (on a firm quote `quote.deadline` replaces the request deadline
// in the signed message), and `check_generated_intent` accepts the exit.
// It passes once the fix in review-1.md (freshness from the signed `timestamp`) is in.

#[test]
fn poc_f02_expired_request_deadline_cannot_be_hidden() {
    for mut c in both() {
        // 31 minutes after the recording: the 30-minute request deadline has passed.
        c.now += chrono::Duration::minutes(31);
        assert!(matches!(c.check(), Err(Refusal::QuoteExpired { .. })));
        // The attacker moves the unsigned request deadline into the future.
        c.edit_quote(|q| q["quoteRequest"]["deadline"] = json!("2026-12-31T00:00:00.000Z"));
        assert!(c.quote.signature_is_valid(), "the edit does not break the signature");
        assert!(
            c.check().is_err(),
            "F-02: an exit was accepted for a quote whose request deadline had passed"
        );
    }
}
