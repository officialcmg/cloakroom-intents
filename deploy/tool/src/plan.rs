//! `zecret-deploy plan`: the dry run. It makes every read-only check that a deployment
//! needs, against the network in the config (mainnet included), and prints the exact plan
//! and its cost. It sends nothing: it has no signer. It reads the chain through the
//! JSON-RPC client in `rpc.rs` and asks 1Click for dry quotes only.

use serde_json::{Value, json};
use std::path::Path;

use crate::Res;
use crate::chain::{BYTE_COST, NO_CODE, code_hash};
use crate::config::{Config, Network, ONE_NEAR, fmt_near, parse_near, resolve};
use crate::deploy::{Ids, read_wasm};
use crate::keys::KeyStore;
use crate::rpc::Rpc;

/// Gas burnt by each deployment transaction on NEAR testnet (`deploy/deployment.testnet.json`,
/// 2026-11-13), in Tgas. Used to estimate the gas of the same transactions on mainnet.
pub const TGAS_CREATE_ACCOUNT: f64 = 8.57;
pub const TGAS_ESCROW_DEPLOY: f64 = 40.31;
pub const TGAS_STORAGE_DEPOSIT: f64 = 2.23;
pub const TGAS_ADD_STORAGE_FUNDS: f64 = 2.51;
/// The maker's deposit into its own Verifier account (`ft_transfer_call` on the token), burnt
/// (docs/experiments/light-payout.md, section 5). Paid by the maker, not the owner.
pub const TGAS_VERIFIER_DEPOSIT: f64 = 8.1;
/// Prepaid gas the taker attaches to `commit_claim`, `claim` and `withdraw(id)`
/// (crates/zecret-core, `near::escrow`: GAS_COMMIT, GAS_REVEAL, GAS_PAYOUT).
pub const TGAS_COMMIT_PREPAID: u128 = 15;
pub const TGAS_CLAIM_PREPAID: u128 = 25;
pub const TGAS_WITHDRAW_PREPAID: u128 = 25;
/// The taker's three calls, measured in the 2.14.1 sandbox with the light-payout escrow
/// (docs/experiments/light-payout.md, sections 1 and 3), in Ggas (10^9 gas):
/// - gas burnt to convert a transaction into a receipt, paid at the gas price;
pub const GGAS_CONVERT: u128 = 320;
/// - send and execution fees that NEAR buys with the attached gas, at the purchase price;
pub const GGAS_FEES: u128 = 887;
/// - gas that `commit_claim` and `claim` burnt in total (0.000256 and 0.001656 NEAR at
///   10^8 yoctoNEAR per gas), before the refunds.
pub const GGAS_COMMIT_BURNT: u128 = 2_560;
pub const GGAS_CLAIM_BURNT: u128 = 16_560;
/// `min_gas_purchase_price` since protocol 85, for when the RPC does not report it.
pub const MIN_GAS_PURCHASE_PRICE: u128 = 1_000_000_000;
/// The stipend must be at least this many times the largest balance NEAR checks.
pub const STIPEND_MARGIN: u128 = 2;
/// The maker's stipend (light payout) and the smallest stipend the taker accepts.
pub const MAKER_STIPEND: u128 = ONE_NEAR * 6 / 100;
pub const TAKER_MIN_STIPEND: u128 = ONE_NEAR * 3 / 100;
/// 1Click asset ids.
pub const ZEC_ASSET: &str = "nep141:zec.omft.near";
pub const BASE_USDC_ASSET: &str =
    "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
/// A well-formed EVM address used as the recipient of the dry quotes (the EIP-55 example).
pub const DRY_RECIPIENT: &str = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
/// The maker's price must stay this many basis points below the market price.
pub const MIN_DISCOUNT_BPS: u128 = 200;
/// A maker price this many basis points below the market price is a bad deal for takers.
pub const MAX_DISCOUNT_BPS: u128 = 1500;

#[derive(Default)]
pub struct Plan {
    pub fails: Vec<String>,
    pub owner_todo: Vec<String>,
}

impl Plan {
    fn pass(&mut self, s: impl AsRef<str>) {
        println!("  [PASS] {}", s.as_ref());
    }
    fn fail(&mut self, s: impl Into<String>) {
        let s = s.into();
        println!("  [FAIL] {s}");
        self.fails.push(s);
    }
    fn check(&mut self, ok: bool, s: impl Into<String>) {
        if ok {
            self.pass(s.into())
        } else {
            self.fail(s)
        }
    }
    fn info(&mut self, s: impl AsRef<str>) {
        println!("  [info] {}", s.as_ref());
    }
    fn todo(&mut self, s: impl Into<String>) {
        let s = s.into();
        println!("  [OWNER] {s}");
        self.owner_todo.push(s);
    }
}

/// One line of the cost table.
#[derive(Debug, Clone, PartialEq)]
pub struct Cost {
    pub what: String,
    pub yocto: u128,
    pub locked: bool,
}

/// What the owner account pays, before anything is sent. `wasm_len` is the escrow wasm,
/// `registrations` the token registrations still missing.
#[allow(clippy::too_many_arguments)]
pub fn owner_costs(
    cfg: &Config,
    byte_cost: u128,
    gas_price: u128,
    wasm_len: usize,
    escrow_exists: bool,
    maker_exists: bool,
    registrations: u128,
    registration_min: u128,
) -> Res<Vec<Cost>> {
    let mut v = vec![];
    let mut tgas = 0.0;
    if !escrow_exists {
        v.push(Cost {
            what: format!("escrow account: {wasm_len} bytes of code storage"),
            yocto: wasm_len as u128 * byte_cost,
            locked: true,
        });
        v.push(Cost {
            what: "escrow margin (escrow.margin_near)".into(),
            yocto: parse_near(&cfg.escrow.margin_near)?,
            locked: true,
        });
        tgas += TGAS_CREATE_ACCOUNT + TGAS_ESCROW_DEPLOY;
    }
    if !maker_exists {
        v.push(Cost {
            what: "maker account (maker.initial_balance_near; pays its gas, stipends and storage funds)"
                .into(),
            yocto: parse_near(&cfg.maker.initial_balance_near)?,
            locked: false,
        });
        tgas += TGAS_CREATE_ACCOUNT + TGAS_ADD_STORAGE_FUNDS;
    }
    if registrations > 0 {
        v.push(Cost {
            what: format!(
                "token registrations: {registrations} × {}",
                fmt_near(registration_min)
            ),
            yocto: registrations * registration_min,
            locked: true,
        });
        tgas += TGAS_STORAGE_DEPOSIT * registrations as f64;
    }
    // Gas, at the current price, of the transactions above (testnet figures).
    let gas = (tgas * 1e12) as u128 * gas_price;
    v.push(Cost {
        what: format!(
            "gas of the deployment transactions (about {tgas:.1} Tgas at {gas_price} yoctoNEAR/gas)"
        ),
        yocto: gas,
        locked: false,
    });
    Ok(v)
}

/// What NEAR checks before a transaction (nearcore 2.14.1, `calculate_tx_cost`): the
/// conversion gas at the gas price `p`, plus the attached gas and the receipt fees at
/// `max(p, m)`, where `m` is `min_gas_purchase_price`.
fn tx_cost(attached_tgas: u128, p: u128, m: u128) -> u128 {
    const G: u128 = 1_000_000_000;
    GGAS_CONVERT * G * p + (attached_tgas * 1_000 + GGAS_FEES) * G * p.max(m)
}

/// The stipend check for the light payout. The taker's key sends `commit_claim`, `claim`
/// and `withdraw(id)` from the stipend alone. Before each call NEAR checks that the key
/// holds [`tx_cost`] on top of what the earlier calls burnt. Returns (the largest balance
/// checked, the largest gas price at which `stipend` still covers it). With the measured
/// figures, at p = 10^8 and m = 10^9 the need is 0.027831 NEAR (before `withdraw`).
pub fn stipend_need(stipend: u128, gas_price: u128, min_purchase: u128) -> (u128, u128) {
    const G: u128 = 1_000_000_000;
    let need_at = |p: u128| {
        let commit = tx_cost(TGAS_COMMIT_PREPAID, p, min_purchase);
        let claim = GGAS_COMMIT_BURNT * G * p + tx_cost(TGAS_CLAIM_PREPAID, p, min_purchase);
        let withdraw = (GGAS_COMMIT_BURNT + GGAS_CLAIM_BURNT) * G * p
            + tx_cost(TGAS_WITHDRAW_PREPAID, p, min_purchase);
        commit.max(claim).max(withdraw)
    };
    let need = need_at(gas_price);
    // need_at grows with p: find the largest p with need_at(p) <= stipend.
    let (mut lo, mut hi) = (0u128, 1u128 << 50);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if need_at(mid) <= stipend {
            lo = mid
        } else {
            hi = mid - 1
        }
    }
    (need, lo)
}

/// The maker's price rule. `maker_micro` and `market_micro` are USDC base units (6
/// decimals) for 1 ZEC. Err: the maker would pay more than the market minus the minimum
/// discount (a loss on every swap). Ok(Some(..)): a warning.
pub fn price_rule(maker_micro: u128, market_micro: u128) -> Result<Option<String>, String> {
    if maker_micro == 0 {
        return Err("price_per_zec is 0: set the maker's price".into());
    }
    if market_micro == 0 {
        return Err("no market price".into());
    }
    let ceiling = market_micro * (10_000 - MIN_DISCOUNT_BPS) / 10_000;
    if maker_micro > ceiling {
        return Err(format!(
            "price_per_zec {maker_micro} is above {ceiling} (market {market_micro} less {MIN_DISCOUNT_BPS} bp): the maker would pay more for ZEC than it is worth"
        ));
    }
    let floor = market_micro * (10_000 - MAX_DISCOUNT_BPS) / 10_000;
    if maker_micro < floor {
        return Ok(Some(format!(
            "price_per_zec {maker_micro} is more than {MAX_DISCOUNT_BPS} bp below the market {market_micro}: takers get a bad rate"
        )));
    }
    Ok(None)
}

fn u128_of(v: &Value) -> Option<u128> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64().map(u128::from),
        _ => None,
    }
}

/// Checks a maker `config.json` against the deployment's ids and the market. Only reads
/// the file; it holds no key.
pub fn check_maker_config(
    p: &mut Plan,
    m: &Value,
    cfg: &Config,
    ids: &Ids,
    market_micro: Option<u128>,
    yocto_near_per_zec_market: Option<u128>,
) {
    let c = &m["chains"];
    let want_net = match cfg.network {
        Network::Mainnet => "main",
        _ => "test",
    };
    p.check(
        c["network"] == want_net,
        format!("maker chains.network = {} (want {want_net})", c["network"]),
    );
    p.check(
        m["near_account"] == ids.maker.as_str(),
        format!(
            "maker near_account = {} (want {})",
            m["near_account"], ids.maker
        ),
    );
    p.check(
        c["escrow"] == ids.escrow.as_str(),
        format!(
            "maker chains.escrow = {} (want {})",
            c["escrow"], ids.escrow
        ),
    );
    p.check(
        c["token"] == ids.token.as_str(),
        format!("maker chains.token = {}", c["token"]),
    );
    p.check(
        c["intents"] == ids.intents.as_str(),
        format!("maker chains.intents = {}", c["intents"]),
    );
    // `near_rpc` is one URL or an ordered list (incident 2026-11-14).
    let https = |v: &Value| match v {
        Value::String(u) => u.starts_with("https://"),
        Value::Array(a) => !a.is_empty() && a.iter().all(|u| u.as_str().is_some_and(|u| u.starts_with("https://"))),
        _ => false,
    };
    p.check(
        https(&c["near_rpc"]) && https(&c["lightwalletd"]),
        "maker endpoints are https",
    );
    p.check(
        c["near_rpc"].as_array().is_some_and(|a| a.len() >= 2),
        "maker names at least two NEAR RPC endpoints (docs/MAINNET.md, RPC availability)",
    );
    let text = m.to_string();
    p.check(
        !text.contains("OWNER") && !text.contains("REPLACE"),
        "maker config has no placeholder left",
    );
    p.check(
        m.get("misbehave").is_none_or(Value::is_null),
        "maker config sets no misbehave",
    );
    let birthday = c["birthday"].as_u64().unwrap_or(0);
    if birthday == 0 {
        p.todo("set chains.birthday in the maker config to the Zcash height before its wallet was made");
    } else {
        p.pass(format!("maker birthday {birthday}"));
    }
    let stipend = u128_of(&m["stipend"]).unwrap_or(0);
    p.check(
        stipend == MAKER_STIPEND,
        format!(
            "maker stipend {} ({} decided in docs/experiments/light-payout.md)",
            fmt_near(stipend),
            fmt_near(MAKER_STIPEND)
        ),
    );
    let (min, max, prem) = (
        m["min_zats"].as_u64().unwrap_or(0),
        m["max_zats"].as_u64().unwrap_or(0),
        m["premium_zats"].as_u64().unwrap_or(0),
    );
    p.check(
        min > 0 && min <= max,
        format!("maker limits {min}..{max} zats, premium {prem} zats"),
    );
    let price = u128_of(&m["price_per_zec"]).unwrap_or(0);
    match market_micro {
        Some(mk) => match price_rule(price, mk) {
            Ok(None) => p.pass(format!("maker price {price} USDC units per ZEC is {MIN_DISCOUNT_BPS}..{MAX_DISCOUNT_BPS} bp below the market {mk}")),
            Ok(Some(w)) => p.info(format!("WARNING: {w}")),
            Err(e) => {
                if price == 0 {
                    p.todo(e)
                } else {
                    p.fail(e)
                }
            }
        },
        None => p.fail("no market price from 1Click to compare the maker price with"),
    }
    let pc = &m["premium_check"];
    if pc.is_null() {
        p.fail("premium_check is missing (required on mainnet)");
        return;
    }
    let ynpz = u128_of(&pc["yocto_near_per_zec"]).unwrap_or(0);
    let costs = u128_of(&pc["costs_per_request_yocto"]).unwrap_or(15 * ONE_NEAR / 1000);
    let worth = prem as u128 * ynpz / 100_000_000;
    p.check(
        worth >= stipend + costs,
        format!(
            "premium_check: {prem} zats at {} per ZEC is {} ≥ stipend + costs {}",
            fmt_near(ynpz),
            fmt_near(worth),
            fmt_near(stipend + costs)
        ),
    );
    if let Some(mkt) = yocto_near_per_zec_market {
        p.check(ynpz <= mkt, format!("premium_check.yocto_near_per_zec {} is not above the market {} (it must be a low estimate)", fmt_near(ynpz), fmt_near(mkt)));
        let at_market = prem as u128 * mkt / 100_000_000;
        p.info(format!(
            "the premium is worth {} at the market price",
            fmt_near(at_market)
        ));
    }
}

async fn http_json(
    http: &reqwest::Client,
    req: &zecret_intents::HttpRequest,
) -> Res<(u16, Value, Vec<u8>)> {
    let mut b = match req.method {
        zecret_intents::Method::Get => http.get(&req.url),
        zecret_intents::Method::Post => http.post(&req.url),
    };
    for (k, v) in &req.headers {
        b = b.header(k, v);
    }
    if let Some(body) = &req.body {
        b = b.body(body.clone());
    }
    let r = b.send().await?;
    let status = r.status().as_u16();
    let bytes = r.bytes().await?.to_vec();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, v, bytes))
}

/// A dry quote through the crate's request builder; the 1Click signature is checked.
async fn dry_quote(
    http: &reqwest::Client,
    origin: &str,
    dest: &str,
    amount: u128,
) -> Res<zecret_intents::QuoteResponse> {
    use zecret_intents::{ExitQuoteParams, OneClick, QuoteRequest, oneclick, quote_sig};
    let api = OneClick::default();
    let deadline =
        zecret_intents::time::from_unix_millis((chrono::Utc::now().timestamp() + 1800) * 1000)?;
    let params = ExitQuoteParams {
        // Any Verifier account id: a dry quote moves nothing and has no deposit address.
        signer_id: "0".repeat(64),
        origin_asset: origin.into(),
        amount,
        destination_asset: dest.into(),
        recipient: DRY_RECIPIENT.into(),
        slippage_bps: 100,
        deadline,
        dry: true,
        referral: None,
        quote_waiting_time_ms: None,
    };
    let req = api.quote_request(&QuoteRequest::exit(&params));
    let (status, _, bytes) = http_json(http, &req).await?;
    let q = oneclick::parse_quote(&zecret_intents::HttpResponse::new(status, bytes))?;
    if !quote_sig::verify(&q.raw, &quote_sig::ONE_CLICK_QUOTE_KEY) {
        return Err("the dry quote's 1Click signature does not verify".into());
    }
    if !q.quote_request.dry {
        return Err("1Click answered a firm quote to a dry request".into());
    }
    Ok(q)
}

pub struct PlanArgs<'a> {
    pub owner: &'a str,
    pub maker_config: Option<&'a Path>,
    pub expect_escrow_hash: Option<&'a str>,
}

/// The dry run. Returns the plan; the caller decides the exit code.
pub async fn plan(cfg: &Config, base: &Path, a: PlanArgs<'_>) -> Res<Plan> {
    let mut p = Plan::default();
    let rpc = Rpc::new(&cfg.rpc_url)?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let ids = crate::deploy::ids(cfg, a.owner);
    println!(
        "== dry run on {}: nothing is signed or sent",
        cfg.network.as_str()
    );

    // 1. The network.
    println!("network");
    let status = rpc
        .call("status", json!([]))
        .await?
        .map_err(|e| format!("status: {e}"))?;
    let want_chain = cfg.network.as_str();
    p.check(
        status["chain_id"] == want_chain,
        format!(
            "RPC {} serves chain_id {} (want {want_chain}), nearcore {}",
            cfg.rpc_url, status["chain_id"], status["version"]["version"]
        ),
    );
    let fb = rpc.block(None, "final").await?;
    let gas_price: u128 = fb["header"]["gas_price"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or("block without gas_price")?;
    p.info(format!(
        "final block {} ({}), gas price {gas_price} yoctoNEAR per gas",
        fb["header"]["height"],
        fb["header"]["hash"].as_str().unwrap_or("")
    ));
    let pc = rpc
        .call(
            "EXPERIMENTAL_protocol_config",
            json!({ "finality": "final" }),
        )
        .await?
        .map_err(|e| format!("protocol config: {e}"))?;
    let byte_cost: u128 = pc["runtime_config"]["storage_amount_per_byte"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or("protocol config without storage_amount_per_byte")?;
    p.check(
        byte_cost == BYTE_COST,
        format!(
            "storage price {byte_cost} yoctoNEAR per byte ({} per 100 kB) equals the tool's",
            fmt_near(byte_cost * 100_000)
        ),
    );
    p.info(format!(
        "protocol version {}, min_gas_price in the protocol config {}",
        pc["protocol_version"], pc["min_gas_price"]
    ));

    // 2. The escrow wasm that would be deployed.
    println!("escrow code");
    let wasm = read_wasm(base, &cfg.escrow.wasm)?;
    let hash = code_hash(&wasm);
    p.info(format!(
        "escrow wasm {}: {} bytes, code hash {hash}",
        resolve(base, &cfg.escrow.wasm)?.display(),
        wasm.len()
    ));
    match a.expect_escrow_hash {
        Some(h) => p.check(h == hash, format!("escrow code hash {hash} equals the reviewed hash {h}")),
        None => p.todo(format!("confirm that {hash} is the code hash of the reviewed escrow build (pass --expect-escrow-hash)")),
    }

    // 3. The owner.
    println!("owner {}", a.owner);
    let owner_acc = rpc.account(a.owner).await?;
    let owner_free = match &owner_acc {
        None => {
            p.fail(format!("owner account {} does not exist", a.owner));
            0
        }
        Some(acc) => {
            let amount: u128 = acc["amount"].as_str().unwrap_or("0").parse()?;
            let usage: u128 = acc["storage_usage"].as_u64().unwrap_or(0).into();
            let free = amount.saturating_sub(usage * byte_cost);
            p.pass(format!(
                "owner {} exists: balance {}, storage {} bytes, free {}",
                a.owner,
                fmt_near(amount),
                usage,
                fmt_near(free)
            ));
            free
        }
    };
    if a.owner.len() == 64 && a.owner.chars().all(|c| c.is_ascii_hexdigit()) {
        p.fail("the owner is an implicit account; use a named account (docs/DEPLOY.md)");
    }
    let keys = KeyStore::open(&resolve(base, &cfg.keys_dir)?)?;
    match keys.get(a.owner) {
        Ok(Some(sk)) if owner_acc.is_some() => {
            let on_chain = rpc.access_keys(a.owner).await?;
            let pk = sk.public_key().to_string();
            p.check(
                on_chain.contains(&pk),
                format!(
                    "the owner's key file holds an access key of {} ({pk})",
                    a.owner
                ),
            );
        }
        Ok(Some(_)) => {}
        Ok(None) => p.todo(format!(
            "put the owner's full-access key in {}/{}.json (mode 600) before deploy",
            keys.dir().display(),
            a.owner
        )),
        Err(e) => p.fail(format!("owner key file: {e}")),
    }

    // 4. The accounts the deployment makes.
    println!("accounts");
    let escrow_hash = rpc.code_hash(&ids.escrow).await?;
    let escrow_exists = escrow_hash.is_some();
    match &escrow_hash {
        None => p.info(format!("CREATE {}: one temporary key, then one transaction deploys {hash}, calls new(intents_account = {}) and deletes the key", ids.escrow, ids.intents)),
        Some(h) if h == &hash => p.pass(format!("{} already runs {hash}; nothing to do", ids.escrow)),
        Some(h) if h == NO_CODE => p.info(format!("{} exists with no code; deploy will finish it if the tool holds its key", ids.escrow)),
        Some(h) => p.fail(format!("{} already runs {h}, not {hash}; a deployed escrow cannot change: use another escrow.sub_account", ids.escrow)),
    }
    let maker_acc = rpc.account(&ids.maker).await?;
    match &maker_acc {
        None => p.info(format!(
            "CREATE {} with {} NEAR, then add_storage_funds {} NEAR from it",
            ids.maker, cfg.maker.initial_balance_near, cfg.maker.storage_funds_near
        )),
        Some(m) => p.info(format!(
            "{} exists, holds {}",
            ids.maker,
            fmt_near(m["amount"].as_str().unwrap_or("0").parse().unwrap_or(0))
        )),
    }
    let initial = parse_near(&cfg.maker.initial_balance_near)?;
    let funds = parse_near(&cfg.maker.storage_funds_near)?;
    p.check(
        maker_acc.is_some() || initial >= funds + ONE_NEAR / 10,
        format!(
            "maker initial balance {} covers its storage funds {} plus 0.1 NEAR for gas",
            fmt_near(initial),
            fmt_near(funds)
        ),
    );
    if maker_acc.is_none() && initial < funds + 3 * ONE_NEAR / 10 {
        p.info("the maker keeps less than 0.2 NEAR free after its storage funds: NEAR refuses its 200 Tgas withdraw below about 0.2 NEAR (docs/ENGINE.md); add NEAR to the maker after deploy");
    }

    // 5. The token.
    println!("token {}", ids.token);
    let mut registration_min = 0;
    let mut registrations = 0u128;
    match rpc.code_hash(&ids.token).await? {
        None => p.fail(format!("token {} does not exist", ids.token)),
        Some(h) => {
            p.info(format!("token code hash {h}"));
            match rpc.view(&ids.token, "ft_metadata", &json!({})).await? {
                Ok(m) => p.check(
                    m["decimals"].as_u64() == Some(cfg.token.decimals as u64),
                    format!(
                        "token symbol {} decimals {} (config {})",
                        m["symbol"], m["decimals"], cfg.token.decimals
                    ),
                ),
                Err(e) => p.fail(format!("token ft_metadata: {e}")),
            }
            let b = rpc
                .view_ok(&ids.token, "storage_balance_bounds", &json!({}))
                .await?;
            registration_min = b["min"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
            p.info(format!(
                "token storage_balance_bounds.min {}",
                fmt_near(registration_min)
            ));
            // The light-payout escrow keeps its tokens inside the Verifier and never holds
            // the plain token, so it needs no registration (deploy::deploy).
            for who in [&ids.intents, &ids.maker] {
                let sb = rpc
                    .view(
                        &ids.token,
                        "storage_balance_of",
                        &json!({ "account_id": who }),
                    )
                    .await?;
                if matches!(&sb, Ok(v) if !v.is_null()) {
                    p.pass(format!("{who} is registered on the token"));
                } else {
                    p.info(format!(
                        "REGISTER {who} on the token (storage_deposit, paid by the owner)"
                    ));
                    registrations += 1;
                }
            }
            if !cfg.token.deploy && cfg.network == Network::Mainnet {
                p.check(
                    ids.token
                        == zecret_intents::oneclick::USDC_NEAR_ASSET.trim_start_matches("nep141:"),
                    "the token is Circle's USDC on NEAR (the 1Click USDC asset)",
                );
            }
        }
    }

    // 6. The Verifier.
    println!("Verifier {}", ids.intents);
    match rpc.code_hash(&ids.intents).await? {
        None => p.fail(format!("Verifier {} does not exist", ids.intents)),
        Some(h) => {
            if let Ok(local) = resolve(base, "$ZECRET_WASM_DIR/intents.near.wasm")
                .and_then(|f| Ok(std::fs::read(f)?))
            {
                p.info(format!(
                    "Verifier code hash {h}; the copy in $ZECRET_WASM_DIR has {}",
                    code_hash(&local)
                ));
            } else {
                p.info(format!("Verifier code hash {h}"));
            }
            match rpc.view(&ids.intents, "current_salt", &json!({})).await? {
                Ok(s) => p.pass(format!("Verifier current_salt = {s}")),
                Err(e) => p.fail(format!("Verifier current_salt: {e}")),
            }
        }
    }

    // 7. The cost.
    println!("cost to the owner");
    let costs = owner_costs(
        cfg,
        byte_cost,
        gas_price,
        wasm.len(),
        escrow_exists,
        maker_acc.is_some(),
        registrations,
        registration_min,
    )?;
    let total: u128 = costs.iter().map(|c| c.yocto).sum();
    let locked: u128 = costs.iter().filter(|c| c.locked).map(|c| c.yocto).sum();
    for c in &costs {
        println!(
            "  {:>14}  {}{}",
            fmt_near(c.yocto),
            c.what,
            if c.locked { " [locked]" } else { "" }
        );
    }
    println!(
        "  {:>14}  TOTAL, of which {} stays locked (escrow storage and margin, registrations)",
        fmt_near(total),
        fmt_near(locked)
    );
    // The tool itself stops unless the owner holds its bound (deploy::funds_needed): the
    // escrow and maker transfers plus 0.5 NEAR for gas and registrations.
    let mut bound = ONE_NEAR / 2;
    if !escrow_exists {
        bound += wasm.len() as u128 * BYTE_COST + parse_near(&cfg.escrow.margin_near)?;
    }
    if maker_acc.is_none() {
        bound += initial;
    }
    p.info(format!("deploy refuses to start unless the owner holds {} free (its own bound: the transfers plus 0.5 NEAR for gas and registrations)", fmt_near(bound)));
    if owner_acc.is_some() {
        if owner_free >= bound {
            p.pass(format!(
                "owner free balance {} ≥ {}",
                fmt_near(owner_free),
                fmt_near(bound)
            ));
        } else {
            p.todo(format!(
                "fund the owner: it holds {} free, deploy needs {} (add {})",
                fmt_near(owner_free),
                fmt_near(bound),
                fmt_near(bound - owner_free)
            ));
        }
    }

    // 8. The stipend at today's gas price.
    println!("taker stipend (commit_claim, claim and withdraw(id) from the taker's key)");
    let min_purchase = match &pc["runtime_config"]["min_gas_purchase_price"] {
        Value::Null => {
            p.info(format!("the protocol config has no min_gas_purchase_price; using {MIN_GAS_PURCHASE_PRICE}"));
            MIN_GAS_PURCHASE_PRICE
        }
        v => u128_of(v).ok_or("min_gas_purchase_price is not a number")?,
    };
    p.info(format!(
        "min_gas_purchase_price {min_purchase} yoctoNEAR per gas (protocol config), gas price {gas_price}"
    ));
    let (need, max_price) = stipend_need(MAKER_STIPEND, gas_price, min_purchase);
    p.check(
        need * STIPEND_MARGIN <= MAKER_STIPEND,
        format!(
            "the maker's stipend {} is at least {STIPEND_MARGIN}× the largest balance NEAR checks ({} before withdraw: {} + {} + {} Tgas attached, measured burn): {:.2}×; it stays enough up to a gas price of {max_price} ({:.2}× today)",
            fmt_near(MAKER_STIPEND),
            fmt_near(need),
            TGAS_COMMIT_PREPAID,
            TGAS_CLAIM_PREPAID,
            TGAS_WITHDRAW_PREPAID,
            MAKER_STIPEND as f64 / need as f64,
            max_price as f64 / gas_price as f64
        ),
    );
    let (_, min_max_price) = stipend_need(TAKER_MIN_STIPEND, gas_price, min_purchase);
    p.check(
        need <= TAKER_MIN_STIPEND,
        format!(
            "the taker's min_stipend {} covers the need today; it stays enough up to a gas price of {min_max_price} ({:.2}× today)",
            fmt_near(TAKER_MIN_STIPEND),
            min_max_price as f64 / gas_price as f64
        ),
    );

    // 9. 1Click.
    println!("1Click (dry quotes only)");
    let (st, tokens, _) =
        http_json(&http, &zecret_intents::OneClick::default().tokens_request()).await?;
    let list = tokens.as_array().cloned().unwrap_or_default();
    p.check(
        st == 200 && !list.is_empty(),
        format!("GET /v0/tokens: HTTP {st}, {} assets", list.len()),
    );
    let find = |id: &str| list.iter().find(|t| t["assetId"] == id).cloned();
    let usdc = find(zecret_intents::oneclick::USDC_NEAR_ASSET);
    p.check(
        usdc.as_ref().is_some_and(|t| t["decimals"] == 6),
        "1Click lists USDC on NEAR with 6 decimals",
    );
    let zec_usd = find(ZEC_ASSET).and_then(|t| t["price"].as_f64());
    let near_usd = find("nep141:wrap.near").and_then(|t| t["price"].as_f64());
    p.info(format!(
        "1Click token prices: ZEC {zec_usd:?} USD, wNEAR {near_usd:?} USD"
    ));
    match dry_quote(&http, zecret_intents::oneclick::USDC_NEAR_ASSET, BASE_USDC_ASSET, 5_000_000).await {
        Ok(q) => p.pass(format!("exit route 5 USDC (NEAR) → USDC on Base: amountOut {} minAmountOut {} in about {:?} s; 1Click signature valid", q.quote.amount_out, q.quote.min_amount_out, q.quote.time_estimate)),
        Err(e) => p.fail(format!("exit route dry quote: {e}")),
    }
    let mut market_micro = None;
    match dry_quote(&http, ZEC_ASSET, BASE_USDC_ASSET, 500_000).await {
        Ok(q) => {
            let out: u128 = q.quote.amount_out.parse().unwrap_or(0);
            let per_zec = out * 100_000_000 / 500_000;
            market_micro = Some(per_zec);
            p.pass(format!("0.005 ZEC → USDC on Base: amountOut {out} ({:.4} USDC per ZEC); 1Click signature valid", per_zec as f64 / 1e6));
        }
        Err(e) => p.fail(format!("ZEC → USDC on Base dry quote: {e}")),
    }
    let ynpz_market = match (zec_usd, near_usd) {
        (Some(z), Some(n)) if n > 0.0 => Some((z / n * 1e6) as u128 * (ONE_NEAR / 1_000_000)),
        _ => None,
    };

    // 10. The maker's settings.
    if let Some(f) = a.maker_config {
        println!("maker config {}", f.display());
        let m: Value = serde_json::from_slice(
            &std::fs::read(f).map_err(|e| format!("read {}: {e}", f.display()))?,
        )?;
        check_maker_config(&mut p, &m, cfg, &ids, market_micro, ynpz_market);
        if let Some(acc) = &maker_acc {
            let bal = rpc
                .view(
                    &ids.token,
                    "ft_balance_of",
                    &json!({ "account_id": ids.maker }),
                )
                .await?;
            let plain: u128 = bal.ok().as_ref().and_then(u128_of).unwrap_or(0);
            let vb = rpc
                .view(
                    &ids.intents,
                    "mt_balance_of",
                    &json!({ "account_id": ids.maker, "token_id": format!("nep141:{}", ids.token) }),
                )
                .await?;
            let in_verifier: u128 = vb.ok().as_ref().and_then(u128_of).unwrap_or(0);
            // The maker opens from its Verifier account and deposits the difference from its
            // plain balance before an open, so both count.
            let have = plain + in_verifier;
            let price = u128_of(&m["price_per_zec"]).unwrap_or(0);
            let max = m["max_zats"].as_u64().unwrap_or(0) as u128;
            let per_swap = max * price / 100_000_000;
            // The maker does not check its float before it quotes; every request that can
            // wait at once (max_pending_quotes, default 32) may need a full swap.
            let pending = m["max_pending_quotes"].as_u64().unwrap_or(32) as u128;
            p.info(format!(
                "maker USDC float {have} units ({plain} plain, {in_verifier} in its Verifier account); one swap at max_zats pays {per_swap}; NEAR {}",
                fmt_near(acc["amount"].as_str().unwrap_or("0").parse().unwrap_or(0))
            ));
            if per_swap > 0 && have < per_swap * pending {
                p.todo(format!(
                    "send USDC to {}: it holds {have} units; {pending} requests at once (max_pending_quotes) at the maximum swap need {}",
                    ids.maker,
                    per_swap * pending
                ));
            }
            if pending > 5 {
                p.info(format!("max_pending_quotes is {pending}: the float must cover that many maximum swaps at once; the mainnet template uses 3"));
            }
        }
    } else {
        p.todo("check the maker's config.json with --maker-config FILE before it starts");
    }

    println!("== plan for {} on {}", a.owner, cfg.network.as_str());
    println!(
        "  1. create {} and deploy {hash} in one transaction that deletes its key",
        ids.escrow
    );
    println!(
        "  2. register {} on {} if needed (the escrow needs no registration: its tokens stay in the Verifier)",
        ids.intents, ids.token
    );
    println!(
        "  3. create {}, register it on the token, add {} storage funds in the escrow",
        ids.maker, cfg.maker.storage_funds_near
    );
    println!(
        "  4. verify (read-only); no USDC moves: the owner sends USDC to {}; the maker deposits it into its own Verifier account (ft_transfer_call, about {TGAS_VERIFIER_DEPOSIT} Tgas) before its first open, or deploy does it up to maker.verifier_target",
        ids.maker
    );
    println!("  total from the owner: about {}", fmt_near(total));
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mainnet() -> Config {
        Config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../configs/mainnet.toml"))
            .unwrap()
            .0
    }

    #[test]
    fn costs_match_the_wasm_and_the_config() {
        let cfg = mainnet();
        let c = owner_costs(
            &cfg,
            BYTE_COST,
            100_000_000,
            525_881,
            false,
            false,
            2,
            ONE_NEAR / 800,
        )
        .unwrap();
        // 525 881 bytes at 10^19 yoctoNEAR = 5.25881 NEAR, margin 0.2, maker 1, two registrations 0.0025.
        assert_eq!(c[0].yocto, 525_881 * BYTE_COST);
        assert_eq!(c[1].yocto, ONE_NEAR / 5);
        assert_eq!(c[2].yocto, ONE_NEAR);
        assert_eq!(c[3].yocto, ONE_NEAR / 400);
        let total: u128 = c.iter().map(|x| x.yocto).sum();
        assert!(
            total > 646 * ONE_NEAR / 100 && total < 647 * ONE_NEAR / 100,
            "{}",
            fmt_near(total)
        );
        // Nothing to create: only gas (zero transactions) remains.
        let none = owner_costs(&cfg, BYTE_COST, 100_000_000, 525_881, true, true, 0, 0).unwrap();
        assert_eq!(none.iter().map(|x| x.yocto).sum::<u128>(), 0);
    }

    #[test]
    fn stipend_need_matches_the_light_payout_measurement() {
        // docs/experiments/light-payout.md, section 3: 0.027831 NEAR before withdraw at
        // p = 10^8 and min_gas_purchase_price = 10^9.
        let (need, max) = stipend_need(MAKER_STIPEND, 100_000_000, MIN_GAS_PURCHASE_PRICE);
        assert_eq!(need, 27_831 * ONE_NEAR / 1_000_000);
        assert!(need * STIPEND_MARGIN <= MAKER_STIPEND);
        // Section 4: 0.06 NEAR holds up to about 1.33 · 10^9, 0.03 NEAR up to about 2.1 · 10^8.
        assert!((1_320_000_000..1_330_000_000).contains(&max), "{max}");
        let (_, max03) = stipend_need(TAKER_MIN_STIPEND, 100_000_000, MIN_GAS_PURCHASE_PRICE);
        assert!((210_000_000..215_000_000).contains(&max03), "{max03}");
        assert!(stipend_need(TAKER_MIN_STIPEND, max03, MIN_GAS_PURCHASE_PRICE).0 <= TAKER_MIN_STIPEND);
        assert!(stipend_need(TAKER_MIN_STIPEND, max03 + 1, MIN_GAS_PURCHASE_PRICE).0 > TAKER_MIN_STIPEND);
        // Without the purchase floor (before protocol 85) the need is the plain gas cost.
        let (old, _) = stipend_need(MAKER_STIPEND, 100_000_000, 0);
        assert!(old < ONE_NEAR / 100);
    }

    #[test]
    fn the_price_rule() {
        let market = 930_000_000; // 930 USDC per ZEC
        assert!(price_rule(0, market).is_err());
        assert!(
            price_rule(market, market).is_err(),
            "the market price itself is a loss after fees"
        );
        assert!(price_rule(market * 99 / 100, market).is_err());
        assert_eq!(price_rule(market * 97 / 100, market), Ok(None));
        assert!(price_rule(market * 80 / 100, market).unwrap().is_some());
    }

    #[test]
    fn the_mainnet_maker_template_needs_the_owner_inputs() {
        let cfg = mainnet();
        let ids = crate::deploy::ids(&cfg, "zecret.near");
        let tpl: Value = serde_json::from_str(include_str!(
            "../../../crates/zecret-node/config/mainnet-maker.json"
        ))
        .unwrap();
        let mut p = Plan::default();
        check_maker_config(
            &mut p,
            &tpl,
            &cfg,
            &ids,
            Some(930_000_000),
            Some(270 * ONE_NEAR),
        );
        // Placeholders: accounts and escrow fail; the price and the birthday are owner inputs.
        assert!(p.fails.iter().any(|f| f.contains("near_account")));
        assert!(p.fails.iter().any(|f| f.contains("placeholder")));
        assert!(
            p.owner_todo
                .iter()
                .any(|t| t.contains("price_per_zec is 0"))
        );
        assert!(p.owner_todo.iter().any(|t| t.contains("birthday")));

        // Filled in as the runbook says: everything passes.
        let filled = tpl
            .to_string()
            .replace("OWNER.near", "zecret.near")
            .replace("\"birthday\":0", "\"birthday\":3570000")
            .replace("\"price_per_zec\":\"0\"", "\"price_per_zec\":\"900000000\"");
        let m: Value = serde_json::from_str(&filled).unwrap();
        let mut p = Plan::default();
        check_maker_config(
            &mut p,
            &m,
            &cfg,
            &ids,
            Some(930_000_000),
            Some(270 * ONE_NEAR),
        );
        assert!(
            p.fails.is_empty() && p.owner_todo.is_empty(),
            "{:?} {:?}",
            p.fails,
            p.owner_todo
        );

        // A price above the market, or a premium_check price above the market, fails.
        let mut p = Plan::default();
        check_maker_config(
            &mut p,
            &m,
            &cfg,
            &ids,
            Some(910_000_000),
            Some(150 * ONE_NEAR),
        );
        assert_eq!(p.fails.len(), 2, "{:?}", p.fails);
    }
}
