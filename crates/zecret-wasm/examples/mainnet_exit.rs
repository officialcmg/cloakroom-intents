//! The owner's first funded test of the exit leg on NEAR mainnet, and the rescue of a
//! payout with only a recovery file. Run it through `scripts/mainnet-exit-test.sh`.
//!
//! The exit uses the engine's own functions (`zecret_wasm::exit::oneclick_quote`,
//! `oneclick_submit`, `oneclick_status`), the ones the browser calls through WebAssembly,
//! here over a native HTTP transport.
//!
//! Commands (nothing is sent without `--i-understand-this-spends-real-funds`):
//!
//! - `plan`: read-only. Reads the owner's account and USDC, the Verifier salt, and a dry
//!   1Click quote for the route; prints every step and transaction that `run` would make.
//! - `run`: makes a fresh key and stores its seed in `--state-dir` (mode 0600) before
//!   anything is sent; deposits `--amount` USDC from the owner's account into that key's
//!   NEAR Intents account (`ft_transfer_call` to `intents.near`, `msg` = the key's account);
//!   then quotes, checks, signs and submits the exit, and follows the status to the end.
//! - `status`: read-only. The balance of a test key and the 1Click status of its exit.
//! - `refund`: moves a test key's USDC back to the owner's NEAR account (`ft_withdraw`
//!   signed by the test key, sent by the owner's account, which pays the gas).
//! - `rescue`: with only a recovery file of the site: moves the swap key's USDC out of
//!   NEAR Intents without 1Click, to a NEAR account (`ft_withdraw`, the account must be
//!   registered with USDC) or to another NEAR Intents account (`transfer`). The swap key's
//!   own NEAR account pays the gas from what is left of the maker's stipend, or another
//!   account does (`--gas-payer-key`). Simulated first; sent only with the flag.
//!
//! ```sh
//! cargo run -p zecret-wasm --example mainnet_exit -- plan --owner-key ~/.near-credentials/mainnet/me.near.json \
//!   --amount 1000000 --dest-asset nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near \
//!   --recipient 0xYourBaseAddress
//! ```

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use zecret_core::near::{At, NearKey, NearRpc, Signer, TGAS, tx::Action};
use zecret_core::net::SharedTransport;
use zecret_intents::{
    HttpRequest, HttpResponse, HttpTransport, IntentSigner, Method, Nonce, SignedPayload, Standard,
    client::{ExitClient, VerifierRpc},
    direct::{IntentDraft, execute_intents_args},
    oneclick::{ExitQuoteParams, OneClick, USDC_NEAR_ASSET, VERIFIER_MAINNET},
    time,
};
use zecret_node::io::Http;
use zecret_wasm::exit::{self, Approval, DIRECT_EXIT_GAS};

const FLAG: &str = "--i-understand-this-spends-real-funds";
const USDC: &str = "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
const DEFAULT_RPC: &str = "https://rpc.mainnet.fastnear.com";
/// The largest test amount without `--max-amount`: 5 USDC.
const DEFAULT_MAX: u128 = 5_000_000;
/// `ft_transfer_call` into the Verifier: 100 Tgas is what the Verifier docs name for a plain deposit.
const DEPOSIT_GAS: u64 = 100 * TGAS;
/// `execute_intents` with one `transfer` burnt 5.3 Tgas in the sandbox; this leaves room.
const TRANSFER_GAS: u64 = 20 * TGAS;

/// A blocking transport with GET (1Click status and tokens) and POST.
struct Native(reqwest::blocking::Client);

impl HttpTransport for Native {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let mut req = match r.method {
            Method::Get => self.0.get(&r.url),
            Method::Post => self.0.post(&r.url).body(r.body.clone().unwrap_or_default()),
        };
        for (k, v) in &r.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .map_err(|e| format!("{} {}: {e}", r.method.as_str(), r.url))?;
        let status = resp.status().as_u16();
        Ok(HttpResponse::new(
            status,
            resp.bytes().map_err(|e| e.to_string())?.to_vec(),
        ))
    }
}

struct Args {
    cmd: String,
    kv: HashMap<String, String>,
    spend: bool,
}

impl Args {
    fn parse() -> Result<Args> {
        let mut it = std::env::args().skip(1);
        let cmd = it.next().ok_or_else(|| anyhow!("usage: mainnet_exit plan|run|status|refund|rescue [options]; see scripts/mainnet-exit-test.sh"))?;
        let (mut kv, mut spend) = (HashMap::new(), false);
        while let Some(a) = it.next() {
            if a == FLAG {
                spend = true;
            } else if let Some(k) = a.strip_prefix("--") {
                kv.insert(
                    k.to_string(),
                    it.next().ok_or_else(|| anyhow!("--{k} needs a value"))?,
                );
            } else {
                bail!("unexpected argument {a:?}");
            }
        }
        Ok(Args { cmd, kv, spend })
    }
    fn get(&self, k: &str) -> Result<&str> {
        self.kv
            .get(k)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("--{k} is required"))
    }
    fn opt(&self, k: &str) -> Option<&str> {
        self.kv.get(k).map(String::as_str)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
}

fn now() -> time::Time {
    time::from_unix_millis(now_ms()).expect("time")
}

fn step(n: &str, what: &str) {
    println!("\n== {n} {what}");
}

/// A near-cli credentials file: `{account_id, private_key}` (or `secret_key`).
fn load_key(path: &str) -> Result<Signer> {
    let v: Value =
        serde_json::from_slice(&std::fs::read(path).with_context(|| format!("read {path}"))?)?;
    let id = v["account_id"]
        .as_str()
        .ok_or_else(|| anyhow!("{path}: no account_id"))?;
    let sk = v["private_key"]
        .as_str()
        .or(v["secret_key"].as_str())
        .ok_or_else(|| anyhow!("{path}: no private_key"))?;
    Ok(Signer::new(id, NearKey::from_secret_str(sk)?))
}

fn usdc(units: u128) -> String {
    format!("{}.{:06}", units / 1_000_000, units % 1_000_000)
}

struct Ctx {
    rpc_url: String,
    shared: SharedTransport,
    near: NearRpc,
    native: Native,
    api: OneClick,
}

impl Ctx {
    fn new(a: &Args) -> Result<Ctx> {
        let rpc_url = a.opt("rpc").unwrap_or(DEFAULT_RPC).to_string();
        let shared: SharedTransport = Rc::new(Http::new()?);
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Ctx {
            near: NearRpc::new(shared.clone(), &rpc_url),
            rpc_url,
            shared,
            native: Native(client),
            api: OneClick {
                base_url: a
                    .opt("oneclick")
                    .unwrap_or(zecret_intents::oneclick::DEFAULT_BASE_URL)
                    .to_string(),
                api_key: None,
            },
        })
    }
    fn verifier(&self) -> VerifierRpc<&Native> {
        VerifierRpc {
            transport: &self.native,
            rpc_url: self.rpc_url.clone(),
            verifier: VERIFIER_MAINNET.into(),
        }
    }
    fn intents_balance(&self, account: &str) -> Result<u128> {
        self.verifier()
            .balance(account, USDC_NEAR_ASSET)
            .map_err(|e| anyhow!("mt_balance_of: {e}"))
    }
    fn ft_balance(&self, account: &str) -> Result<u128> {
        let v = self.near.view(
            USDC,
            "ft_balance_of",
            &json!({ "account_id": account }),
            &At::Final,
        )?;
        Ok(v.as_str().unwrap_or("0").parse()?)
    }
    fn chain_id(&self) -> Result<String> {
        let s = self.near.call("status", json!([]))?;
        Ok(s["chain_id"].as_str().unwrap_or("").to_string())
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct State {
    created_at: String,
    owner: String,
    /// The test key's 32-byte ed25519 seed, hex. Anyone with this file can move the test USDC.
    seed: String,
    account: String,
    amount: String,
    destination_asset: String,
    recipient: String,
    min_amount_out: Option<String>,
    deposit_tx: Option<String>,
    deposit_address: Option<String>,
    intent_hash: Option<String>,
    firm_quote: Option<Value>,
    final_status: Option<String>,
}

fn state_path(dir: &str, account: &str) -> String {
    format!("{dir}/exit-test-{}.json", &account[..16])
}

fn save_state(dir: &str, st: &State) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let p = state_path(dir, &st.account);
    let tmp = format!("{p}.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(st)? + "\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

fn load_state(path: &str) -> Result<(State, IntentSigner)> {
    let st: State =
        serde_json::from_slice(&std::fs::read(path).with_context(|| format!("read {path}"))?)?;
    let seed: [u8; 32] = hex::decode(&st.seed)?
        .try_into()
        .map_err(|_| anyhow!("seed is not 32 bytes"))?;
    let signer = IntentSigner::from_seed(&seed);
    if signer.account_id() != st.account {
        bail!("the state file's seed does not own {}", st.account);
    }
    Ok((st, signer))
}

/// The route and amount of `plan` and `run`, checked against limits.
fn route(a: &Args) -> Result<(u128, String, String)> {
    let amount: u128 = a
        .get("amount")?
        .parse()
        .context("--amount is USDC base units (6 decimals), e.g. 1000000 for 1 USDC")?;
    let max: u128 = a
        .opt("max-amount")
        .map(str::parse)
        .transpose()?
        .unwrap_or(DEFAULT_MAX);
    if amount == 0 || amount > max {
        bail!(
            "--amount {} is outside 1 unit to {} USDC (raise the cap with --max-amount)",
            usdc(amount),
            usdc(max)
        );
    }
    Ok((
        amount,
        a.get("dest-asset")?.to_string(),
        a.get("recipient")?.to_string(),
    ))
}

/// A dry quote (signature checked) and the minimum to approve: its minAmountOut less
/// `--headroom-bps` (default 50), or `--min-out`.
fn dry_quote(
    ctx: &Ctx,
    account: &str,
    amount: u128,
    dest: &str,
    recipient: &str,
    a: &Args,
) -> Result<(Value, u128)> {
    let c = ExitClient::new(&ctx.native, ctx.api.clone());
    let q = c
        .quote(&ExitQuoteParams {
            signer_id: account.into(),
            origin_asset: USDC_NEAR_ASSET.into(),
            amount,
            destination_asset: dest.into(),
            recipient: recipient.into(),
            slippage_bps: 100,
            deadline: now() + chrono::Duration::minutes(30),
            dry: true,
            referral: None,
            quote_waiting_time_ms: None,
        })
        .map_err(|e| anyhow!("dry quote: {e}"))?;
    let min_q: u128 = q.quote.min_amount_out.parse()?;
    let min = match a.opt("min-out") {
        Some(m) => m.parse()?,
        None => {
            let bps: u128 = a
                .opt("headroom-bps")
                .map(str::parse)
                .transpose()?
                .unwrap_or(50);
            min_q - min_q * bps / 10_000
        }
    };
    println!(
        "dry quote: amountIn {} amountOut {} minAmountOut {} timeEstimate {:?} s, appFees {} bp, withdrawFee {:?}",
        q.quote.amount_in,
        q.quote.amount_out,
        q.quote.min_amount_out,
        q.quote.time_estimate,
        q.quote_request.app_fee_bps(),
        q.quote.withdraw_fee
    );
    println!("approved minimum for the exit: {min} (destination base units)");
    Ok((q.raw, min))
}

fn cmd_plan(a: &Args) -> Result<()> {
    let ctx = Ctx::new(a)?;
    let (amount, dest, recipient) = route(a)?;
    let owner = load_key(a.get("owner-key")?)?;
    step("1", "NEAR mainnet and the owner's account (read-only)");
    let chain = ctx.chain_id()?;
    println!("RPC {} chain_id {chain}", ctx.rpc_url);
    if chain != "mainnet" {
        bail!("the RPC endpoint is not NEAR mainnet");
    }
    let acct = ctx
        .near
        .account(&owner.account_id, &At::Final)?
        .ok_or_else(|| anyhow!("{} does not exist", owner.account_id))?;
    println!("owner {} holds {} yoctoNEAR", owner.account_id, acct.amount);
    let bal = ctx.ft_balance(&owner.account_id)?;
    println!("owner USDC (plain NEP-141): {}", usdc(bal));
    if bal < amount {
        println!(
            "WARNING: the owner holds less USDC than --amount; `run` would fail at the deposit"
        );
    }
    step("2", "NEAR Intents (read-only)");
    let salt = ctx
        .verifier()
        .current_salt()
        .map_err(|e| anyhow!("current_salt: {e}"))?;
    println!("intents.near current_salt {}", salt.to_hex());
    step("3", "1Click dry quote for the route (read-only)");
    let probe = IntentSigner::generate()?;
    dry_quote(&ctx, &probe.account_id(), amount, &dest, &recipient, a)?;
    step("4", "what `run` would do (nothing is sent now)");
    println!(
        "a. make a fresh ed25519 key; store its seed in --state-dir (0600) before anything is sent"
    );
    println!(
        "b. tx from {}: {USDC}.ft_transfer_call(receiver_id=intents.near, amount={amount}, msg=<key account>), 1 yoctoNEAR, 100 Tgas",
        owner.account_id
    );
    println!(
        "c. wait until intents.near mt_balance_of(<key account>, {USDC_NEAR_ASSET}) >= {amount} at a final block"
    );
    println!(
        "d. POST /v0/quote (firm), quote check; POST /v0/generate-intent, payload check; one signature; POST /v0/submit-intent (no gas)"
    );
    println!(
        "e. GET /v0/status every 5 s until SUCCESS, REFUNDED or FAILED; then the key's balance"
    );
    println!(
        "f. on REFUNDED or FAILED: `refund` moves the USDC back to {} (the owner pays the gas)",
        owner.account_id
    );
    println!("\nPLAN OK: nothing was sent.");
    Ok(())
}

fn cmd_run(a: &Args) -> Result<()> {
    if !a.spend {
        bail!("`run` spends real funds: it needs {FLAG}");
    }
    let ctx = Ctx::new(a)?;
    let (amount, dest, recipient) = route(a)?;
    let owner = load_key(a.get("owner-key")?)?;
    let dir = a
        .opt("state-dir")
        .unwrap_or("./exit-test-state")
        .to_string();
    if ctx.chain_id()? != "mainnet" {
        bail!("the RPC endpoint is not NEAR mainnet");
    }
    if ctx.ft_balance(&owner.account_id)? < amount {
        bail!("{} holds less than {} USDC", owner.account_id, usdc(amount));
    }

    step("1", "fresh key, stored before anything is sent");
    let key = IntentSigner::generate()?;
    let account = key.account_id();
    let (_, min) = dry_quote(&ctx, &account, amount, &dest, &recipient, a)?;
    let mut st = State {
        created_at: time::format(now()),
        owner: owner.account_id.clone(),
        seed: hex::encode(*key.seed()),
        account: account.clone(),
        amount: amount.to_string(),
        destination_asset: dest.clone(),
        recipient: recipient.clone(),
        min_amount_out: Some(min.to_string()),
        deposit_tx: None,
        deposit_address: None,
        intent_hash: None,
        firm_quote: None,
        final_status: None,
    };
    let path = save_state(&dir, &st)?;
    println!("key account {account}\nstate file {path} (keep it until the test is over)");

    step("2", "deposit USDC into the key's NEAR Intents account");
    let out = ctx.near.transact(
        &owner,
        USDC,
        vec![Action::call("ft_transfer_call", &json!({ "receiver_id": VERIFIER_MAINNET, "amount": amount.to_string(), "msg": account }), DEPOSIT_GAS, 1)],
    )?;
    println!(
        "deposit tx {} https://nearblocks.io/txns/{}",
        out.hash, out.hash
    );
    st.deposit_tx = Some(out.hash.clone());
    save_state(&dir, &st)?;
    out.ok()
        .context("the deposit failed; nothing reached NEAR Intents (check the owner's USDC)")?;
    let t0 = now_ms();
    loop {
        let b = ctx.intents_balance(&account)?;
        println!("key balance in NEAR Intents: {}", usdc(b));
        if b >= amount {
            break;
        }
        if now_ms() - t0 > 180_000 {
            bail!(
                "the deposit did not show in NEAR Intents within 3 minutes; check the tx, then `status --state {path}`"
            );
        }
        std::thread::sleep(Duration::from_secs(3));
    }

    step(
        "3",
        "the exit: the engine's own steps (exit::oneclick_quote, oneclick_submit)",
    );
    let approval = Approval {
        signer_id: account.clone(),
        verifying_contract: VERIFIER_MAINNET.into(),
        origin_asset: USDC_NEAR_ASSET.into(),
        amount_in: amount.to_string(),
        destination_asset: dest.clone(),
        recipient: recipient.clone(),
        min_amount_out: min.to_string(),
    };
    let q = exit::oneclick_quote(&ctx.native, &ctx.api, &approval, now_ms()).map_err(|e| {
        anyhow!(
            "firm quote: {} {}{}",
            e.code,
            e.message,
            e.offered
                .map(|o| format!(" (offered minAmountOut {})", o.min_amount_out))
                .unwrap_or_default()
        )
    })?;
    println!(
        "firm quote: deposit address {} amountOut {} minAmountOut {} timeEstimate {:?} s fee {} bp",
        q.deposit_address, q.amount_out, q.min_amount_out, q.time_estimate_s, q.fee_bps
    );
    st.firm_quote = Some(q.raw.clone());
    st.deposit_address = Some(q.deposit_address.clone());
    save_state(&dir, &st)?;
    let seed = key.seed();
    let sub = exit::oneclick_submit(&ctx.native, &ctx.api, &seed, &approval, &q.raw, now_ms())
        .map_err(|e| anyhow!("submit: {} {}", e.code, e.message))?;
    println!(
        "submitted: intent hash {} deposit address {}\nexplorer https://explorer.near-intents.org/transactions/{}",
        sub.intent_hash, sub.deposit_address, sub.deposit_address
    );
    st.intent_hash = Some(sub.intent_hash);
    save_state(&dir, &st)?;

    step("4", "status (exit::oneclick_status)");
    follow(&ctx, &mut st, &dir)
}

fn follow(ctx: &Ctx, st: &mut State, dir: &str) -> Result<()> {
    let dep = st
        .deposit_address
        .clone()
        .ok_or_else(|| anyhow!("no deposit address in the state file"))?;
    let t0 = now_ms();
    let mut last = String::new();
    loop {
        match exit::oneclick_status(&ctx.native, &ctx.api, &dep) {
            Ok(s) => {
                let line = format!(
                    "{} amountOut {:?} refunded {:?} reason {:?} txs {:?}",
                    s.status, s.amount_out, s.refunded_amount, s.refund_reason, s.destination_txs
                );
                if line != last {
                    println!("[{}] {line}", time::format(now()));
                    last = line;
                }
                if s.is_final {
                    let b = ctx.intents_balance(&st.account)?;
                    println!(
                        "final: {} ; the key now holds {} USDC in NEAR Intents",
                        s.status,
                        usdc(b)
                    );
                    st.final_status = Some(s.status.clone());
                    save_state(dir, st)?;
                    if s.status != "SUCCESS" {
                        println!(
                            "the USDC can go back to the owner: scripts/mainnet-exit-test.sh refund --state {} {FLAG}",
                            state_path(dir, &st.account)
                        );
                    }
                    return Ok(());
                }
            }
            Err(e) => println!("status: {} {}", e.code, e.message),
        }
        if now_ms() - t0 > 30 * 60_000 {
            println!(
                "no final status after 30 minutes; run `status --state {}` later",
                state_path(dir, &st.account)
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn cmd_status(a: &Args) -> Result<()> {
    let ctx = Ctx::new(a)?;
    let path = a.get("state")?.to_string();
    let (mut st, _) = load_state(&path)?;
    println!(
        "key account {} balance {} USDC in NEAR Intents",
        st.account,
        usdc(ctx.intents_balance(&st.account)?)
    );
    if st.deposit_address.is_some() {
        let dir = std::path::Path::new(&path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .to_string();
        follow(&ctx, &mut st, &dir)?;
    }
    Ok(())
}

/// Signs one intent with `key`, simulates it on `intents.near`, and (with the flag) sends it
/// with `execute_intents` from `payer`, attaching `gas`.
fn simulate_and_send(
    ctx: &Ctx,
    signed: SignedPayload,
    payer: &Signer,
    gas: u64,
    spend: bool,
) -> Result<()> {
    println!("signed payload: {}", signed.to_json());
    match ctx.verifier().simulate(std::slice::from_ref(&signed)) {
        Ok(v) => println!("simulate_intents: ok {v}"),
        Err(e) => bail!("simulate_intents refused it, so nothing was sent: {e}"),
    }
    if !spend {
        println!(
            "\nDRY RUN: simulated only. Add {FLAG} to send it from {} with {} Tgas.",
            payer.account_id,
            gas / TGAS
        );
        return Ok(());
    }
    let out = ctx.near.transact(
        payer,
        VERIFIER_MAINNET,
        vec![Action::call(
            "execute_intents",
            &execute_intents_args(&[signed]),
            gas,
            0,
        )],
    )?;
    println!(
        "execute_intents tx {} https://nearblocks.io/txns/{}",
        out.hash, out.hash
    );
    out.ok()?;
    println!("executed");
    Ok(())
}

fn intent_draft(ctx: &Ctx, signer_id: &str) -> Result<(IntentDraft, time::Time)> {
    let salt = ctx
        .verifier()
        .current_salt()
        .map_err(|e| anyhow!("current_salt: {e}"))?;
    let t = now();
    let deadline = time::from_unix_millis((t + chrono::Duration::minutes(10)).timestamp_millis())?;
    Ok((
        IntentDraft::new(
            signer_id,
            VERIFIER_MAINNET,
            deadline,
            Nonce::versioned_random(salt, deadline)?,
        ),
        t,
    ))
}

fn cmd_refund(a: &Args) -> Result<()> {
    let ctx = Ctx::new(a)?;
    let (st, key) = load_state(a.get("state")?)?;
    let owner = load_key(a.get("owner-key")?)?;
    if owner.account_id != st.owner {
        bail!(
            "the state file's owner is {}, the key file is for {}",
            st.owner,
            owner.account_id
        );
    }
    let bal = ctx.intents_balance(&st.account)?;
    if bal == 0 {
        bail!("the test key holds no USDC in NEAR Intents");
    }
    println!(
        "ft_withdraw {} USDC from {} to {}",
        usdc(bal),
        st.account,
        st.owner
    );
    let (draft, t) = intent_draft(&ctx, &st.account)?;
    let signed = draft
        .ft_withdraw(USDC, &st.owner, bal)
        .sign(&key, Standard::RawEd25519, t)?;
    simulate_and_send(&ctx, signed, &owner, DIRECT_EXIT_GAS * 2, a.spend)
}

/// The swap's NEAR key from a recovery file of the site (`records["swap/<id>"].near_sk`).
fn recovery_seed(path: &str) -> Result<[u8; 32]> {
    let f: Value =
        serde_json::from_slice(&std::fs::read(path).with_context(|| format!("read {path}"))?)?;
    if f["format"] != "zecret-recovery" {
        bail!("{path} is not a Zecret recovery file");
    }
    let id = f["swapId"].as_str().ok_or_else(|| anyhow!("no swapId"))?;
    let rec: Value = serde_json::from_str(
        f["records"][format!("swap/{id}")]
            .as_str()
            .ok_or_else(|| anyhow!("no swap record"))?,
    )?;
    let sk = rec["near_sk"].as_str().ok_or_else(|| {
        anyhow!("the swap record has no NEAR key (the swap did not reach the claim)")
    })?;
    hex::decode(sk)?
        .try_into()
        .map_err(|_| anyhow!("near_sk is not 32 bytes"))
}

fn cmd_rescue(a: &Args) -> Result<()> {
    let ctx = Ctx::new(a)?;
    let seed = recovery_seed(a.get("recovery-file")?)?;
    let key = IntentSigner::from_seed(&seed);
    let account = key.account_id();
    let bal = ctx.intents_balance(&account)?;
    let near = ctx
        .near
        .account(&account, &At::Final)?
        .map(|i| i.amount)
        .unwrap_or(0);
    println!(
        "swap key account {account}: {} USDC in NEAR Intents, {near} yoctoNEAR on NEAR",
        usdc(bal)
    );
    if bal == 0 {
        bail!("the swap key holds no USDC in NEAR Intents: nothing to rescue");
    }
    let payer = match a.opt("gas-payer-key") {
        Some(p) => load_key(p)?,
        None => Signer::new(&account, NearKey::from_seed(seed)),
    };
    match (a.opt("to-near-account"), a.opt("to-intents-account")) {
        (Some(to), None) if a.opt("gas-payer-key").is_none() => {
            // The engine's own direct exit (sandbox-tested): checks the receiver exists and is
            // registered with USDC, simulates, and sends from the swap key's account.
            let plan = exit::direct_prepare(
                &ctx.shared,
                &ctx.rpc_url,
                VERIFIER_MAINNET,
                USDC,
                &seed,
                to,
                now_ms(),
            )
            .map_err(|e| anyhow!("{} {}", e.code, e.message))?;
            println!(
                "plan: ft_withdraw {} USDC to {to}, intent hash {}",
                usdc(plan.amount.parse()?),
                plan.intent_hash
            );
            if !a.spend {
                let (draft, t) = intent_draft(&ctx, &account)?;
                let signed =
                    draft
                        .ft_withdraw(USDC, to, bal)
                        .sign(&key, Standard::RawEd25519, t)?;
                return simulate_and_send(&ctx, signed, &payer, DIRECT_EXIT_GAS, false);
            }
            let r = exit::direct_submit_with_gas(
                &ctx.shared,
                &ctx.rpc_url,
                &seed,
                &plan,
                now_ms(),
                DIRECT_EXIT_GAS,
            )
            .map_err(|e| anyhow!("{} {}", e.code, e.message))?;
            println!(
                "executed: tx {:?}, {} USDC to {to}; the swap key's NEAR {} -> {}",
                r.tx_hash,
                usdc(r.delivered.parse()?),
                r.near_before,
                r.near_after
            );
            Ok(())
        }
        (Some(to), None) => {
            if ctx
                .near
                .view(
                    USDC,
                    "storage_balance_of",
                    &json!({ "account_id": to }),
                    &At::Final,
                )?
                .is_null()
            {
                bail!(
                    "{to} is not registered with USDC (storage_deposit), so it cannot receive it; nothing was signed"
                );
            }
            let (draft, t) = intent_draft(&ctx, &account)?;
            let signed = draft
                .ft_withdraw(USDC, to, bal)
                .sign(&key, Standard::RawEd25519, t)?;
            simulate_and_send(&ctx, signed, &payer, DIRECT_EXIT_GAS, a.spend)
        }
        (None, Some(to)) => {
            if !zecret_intents::check::valid_account_id(to) || to == account {
                bail!("{to:?} is not another NEAR Intents account");
            }
            let (draft, t) = intent_draft(&ctx, &account)?;
            let signed =
                draft
                    .transfer(to, USDC_NEAR_ASSET, bal)
                    .sign(&key, Standard::RawEd25519, t)?;
            simulate_and_send(&ctx, signed, &payer, TRANSFER_GAS, a.spend)
        }
        _ => bail!(
            "give exactly one of --to-near-account (registered with USDC) or --to-intents-account"
        ),
    }
}

fn main() -> Result<()> {
    let a = Args::parse()?;
    match a.cmd.as_str() {
        "plan" => cmd_plan(&a),
        "run" => cmd_run(&a),
        "status" => cmd_status(&a),
        "refund" => cmd_refund(&a),
        "rescue" => cmd_rescue(&a),
        other => bail!("unknown command {other:?}: plan, run, status, refund, rescue"),
    }
}
