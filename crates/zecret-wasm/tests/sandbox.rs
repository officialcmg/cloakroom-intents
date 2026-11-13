//! The direct exit against a NEAR 2.14.1 sandbox with the live `intents.near` and
//! `wrap.near` code: the taker key signs one `ft_withdraw` and the maker's relay
//! (`zecret_node::relay`) sends it with `execute_intents`. Also measured here: what is left
//! of the maker's 0.03 NEAR stipend after commit and claim (0.0281 NEAR, measured in
//! contracts/near-escrow) cannot pay for that call from the taker's own account.
//!
//! ```sh
//! source .tools/env.sh
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test -p zecret-wasm --test sandbox -- --nocapture
//! ```

use serde_json::json;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zecret_core::{
    near::{tx::Action, At, NearKey, NearRpc, Signer, ONE_NEAR, TGAS},
    net::SharedTransport,
};
use zecret_node::io::Http;
use zecret_wasm::exit::{direct_prepare, direct_relay, direct_submit_with_gas, DIRECT_EXIT_GAS};

struct Sandbox {
    child: Child,
    home: PathBuf,
    url: String,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn start_sandbox() -> Sandbox {
    let bin = std::env::var("NEAR_SANDBOX_BIN_PATH").expect("NEAR_SANDBOX_BIN_PATH (source .tools/env.sh)");
    let port = 31000 + (std::process::id() % 2000) as u16 * 2;
    let home = std::env::temp_dir().join(format!("zecret-wasm-sandbox-{port}"));
    let _ = std::fs::remove_dir_all(&home);
    let st = Command::new(&bin).args(["--home", home.to_str().unwrap(), "init"]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
    assert!(st.success(), "sandbox init failed (is LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD set?)");
    let cfg_path = home.join("config.json");
    let mut cfg: serde_json::Value = serde_json::from_slice(&std::fs::read(&cfg_path).unwrap()).unwrap();
    cfg["rpc"]["addr"] = json!(format!("127.0.0.1:{port}"));
    cfg["network"]["addr"] = json!(format!("127.0.0.1:{}", port + 1));
    std::fs::write(&cfg_path, cfg.to_string()).unwrap();
    let child = Command::new(&bin).args(["--home", home.to_str().unwrap(), "run"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let url = format!("http://127.0.0.1:{port}");
    let rpc = NearRpc::new(Rc::new(Http::new().unwrap()), &url);
    for _ in 0..120 {
        if rpc.block(&At::Final).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Sandbox { child, home, url }
}

fn ok(what: &str, out: zecret_core::near::TxOutcome) {
    if let Some(f) = out.failure() {
        panic!("{what}: {f}");
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

struct World {
    _sb: Sandbox,
    t: SharedTransport,
    rpc: NearRpc,
    root: Signer,
    wrap: String,
    intents: String,
}

fn world() -> World {
    let sb = start_sandbox();
    let t: SharedTransport = Rc::new(Http::new().unwrap());
    let rpc = NearRpc::new(t.clone(), &sb.url);
    let key: serde_json::Value = serde_json::from_slice(&std::fs::read(sb.home.join("validator_key.json")).unwrap()).unwrap();
    let root_id = key["account_id"].as_str().unwrap().to_string();
    let root = Signer::new(&root_id, NearKey::from_secret_str(key["secret_key"].as_str().unwrap()).unwrap());
    let dir = PathBuf::from(std::env::var("ZECRET_WASM_DIR").expect("ZECRET_WASM_DIR (source .tools/env.sh)"));
    let mut make = |name: &str, code: Vec<u8>| {
        let id = format!("{name}.{root_id}");
        let actions = vec![Action::CreateAccount, Action::Transfer { deposit: 50 * ONE_NEAR }, Action::AddFullAccessKey { public_key: root.key.public_bytes() }, Action::DeployContract { code }];
        ok(&format!("create {id}"), rpc.transact(&root, &id, actions).unwrap());
        id
    };
    let wrap = make("wrap", std::fs::read(dir.join("wrap.near.wasm")).unwrap());
    let intents = make("intents", std::fs::read(dir.join("intents.near.wasm")).unwrap());
    let as_c = |id: &str| Signer::new(id, root.key.clone());
    ok("wrap.new", rpc.transact(&as_c(&wrap), &wrap, vec![Action::call("new", &json!({}), 30 * TGAS, 0)]).unwrap());
    ok(
        "intents.new",
        rpc.transact(&as_c(&intents), &intents, vec![Action::call("new", &json!({"config": {"wnear_id": wrap, "fees": {"fee": 0, "fee_collector": intents}}}), 300 * TGAS, 0)]).unwrap(),
    );
    ok("storage intents", rpc.transact(&root, &wrap, vec![Action::call("storage_deposit", &json!({"account_id": intents}), 30 * TGAS, ONE_NEAR / 80)]).unwrap());
    ok("storage root", rpc.transact(&root, &wrap, vec![Action::call("storage_deposit", &json!({"account_id": root_id}), 30 * TGAS, ONE_NEAR / 80)]).unwrap());
    ok("near_deposit", rpc.transact(&root, &wrap, vec![Action::call("near_deposit", &json!({}), 30 * TGAS, 1000 * ONE_NEAR)]).unwrap());
    World { _sb: sb, t, rpc, root, wrap, intents }
}

impl World {
    /// A fresh taker key: `near` yoctoNEAR on its implicit account and `tokens` wNEAR in
    /// its Verifier account. A receiver account, registered with the token if `register`.
    fn taker(&self, near: u128, tokens: u128, receiver: &str, register: bool) -> ([u8; 32], String, String) {
        let key = NearKey::generate();
        let acct = key.implicit_account();
        ok("stipend", self.rpc.transact(&self.root, &acct, vec![Action::Transfer { deposit: near }]).unwrap());
        ok(
            "deposit into intents",
            self.rpc
                .transact(&self.root, &self.wrap, vec![Action::call("ft_transfer_call", &json!({"receiver_id": self.intents, "amount": tokens.to_string(), "msg": acct}), 100 * TGAS, 1)])
                .unwrap(),
        );
        let rid = format!("{receiver}.{}", self.root.account_id);
        ok("receiver", self.rpc.transact(&self.root, &rid, vec![Action::CreateAccount, Action::Transfer { deposit: ONE_NEAR }, Action::AddFullAccessKey { public_key: self.root.key.public_bytes() }]).unwrap());
        if register {
            ok("storage receiver", self.rpc.transact(&self.root, &self.wrap, vec![Action::call("storage_deposit", &json!({"account_id": rid}), 30 * TGAS, ONE_NEAR / 80)]).unwrap());
        }
        (key.seed(), acct, rid)
    }

    fn balance(&self, contract: &str, method: &str, args: serde_json::Value) -> u128 {
        self.rpc.view(contract, method, &args, &At::Final).unwrap().as_str().unwrap().parse().unwrap()
    }

    fn settle(&self) {
        // Views at a final block: wait for the last receipts to be final.
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// Starts the maker's relay on a free port, with a maker store that knows `takers`.
fn relay(w: &World, takers: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("zecret-wasm-relay-{}-{}", std::process::id(), takers.len()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("swap")).unwrap();
    for (i, t) in takers.iter().enumerate() {
        std::fs::write(dir.join(format!("swap/{i:02}.json")), json!({"taker_account": t}).to_string()).unwrap();
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let r = zecret_node::relay::Relay { near_rpc: w.rpc.url.clone(), verifier: w.intents.clone(), maker_account: w.root.account_id.clone(), key: w.root.key.clone(), store_dir: dir };
    let addr = format!("127.0.0.1:{port}");
    let a = addr.clone();
    std::thread::spawn(move || r.serve(&a).unwrap());
    std::thread::sleep(Duration::from_millis(300));
    format!("http://{addr}/execute-intents")
}

const STIPEND_LEFT: u128 = 28_100_000_000_000_000_000_000; // 0.0281 NEAR

#[test]
fn direct_exit_through_the_makers_relay() {
    let w = world();
    let amount = 30 * ONE_NEAR;
    let (seed, acct, receiver) = w.taker(STIPEND_LEFT, amount, "alice", true);
    let (stranger_seed, _, stranger_receiver) = w.taker(STIPEND_LEFT, ONE_NEAR, "mallory", true);
    w.settle();
    let url = relay(&w, &[&acct]);

    // A key that is not one of the maker's takers is refused by the relay.
    let plan = direct_prepare(&w.t, &w.rpc.url, &w.intents, &w.wrap, &stranger_seed, &stranger_receiver, now_ms()).unwrap();
    let e = direct_relay(&w.t, &w.rpc.url, &url, &stranger_seed, &plan, now_ms()).unwrap_err();
    assert!(e.message.contains("is not the taker"), "{e:?}");

    let plan = direct_prepare(&w.t, &w.rpc.url, &w.intents, &w.wrap, &seed, &receiver, now_ms()).expect("prepare");
    assert_eq!(plan.amount, amount.to_string());
    let r = direct_relay(&w.t, &w.rpc.url, &url, &seed, &plan, now_ms()).expect("relay");
    w.settle();
    let after = w.rpc.account(&acct, &At::Final).unwrap().unwrap().amount;
    println!("direct exit through the relay: tx {:?}; taker NEAR {} -> {}", r.tx_hash, r.near_before, after);
    assert!(r.tx_hash.is_some());
    assert_eq!(after, STIPEND_LEFT, "the relay paid the gas");
    assert_eq!(w.balance(&w.wrap, "ft_balance_of", json!({"account_id": receiver})), amount, "the receiver got the whole payout");
    assert_eq!(w.balance(&w.intents, "mt_balance_of", json!({"account_id": acct, "token_id": format!("nep141:{}", w.wrap)})), 0);

    // The same plan again: the nonce is used, nothing is sent twice.
    let again = direct_relay(&w.t, &w.rpc.url, &url, &seed, &plan, now_ms()).expect("resubmit");
    assert!(again.tx_hash.is_none());
    assert_eq!(w.balance(&w.wrap, "ft_balance_of", json!({"account_id": receiver})), amount);
}

#[test]
fn the_rest_of_the_stipend_cannot_pay_and_bad_receivers_are_refused() {
    let w = world();
    let (seed, acct, receiver) = w.taker(STIPEND_LEFT, 5 * ONE_NEAR, "carol", true);
    let (seed2, _, unregistered) = w.taker(STIPEND_LEFT, 5 * ONE_NEAR, "bob", false);
    w.settle();
    // What is left of the stipend after commit and claim cannot attach the 40 Tgas that
    // ft_withdraw needs: NEAR refuses the transaction before it runs.
    let plan = direct_prepare(&w.t, &w.rpc.url, &w.intents, &w.wrap, &seed, &receiver, now_ms()).expect("prepare");
    let e = direct_submit_with_gas(&w.t, &w.rpc.url, &seed, &plan, now_ms(), DIRECT_EXIT_GAS).unwrap_err();
    println!("taker pays {} Tgas from 0.0281 NEAR: {}", DIRECT_EXIT_GAS / TGAS, e.message);
    assert_eq!(e.code, "EXIT_FAILED");
    assert!(e.message.contains("NotEnoughBalance"), "{e:?}");
    w.settle();
    assert_eq!(w.balance(&w.intents, "mt_balance_of", json!({"account_id": acct, "token_id": format!("nep141:{}", w.wrap)})), 5 * ONE_NEAR, "nothing moved");

    let e = direct_prepare(&w.t, &w.rpc.url, &w.intents, &w.wrap, &seed2, &unregistered, now_ms()).unwrap_err();
    assert_eq!(e.code, "EXIT_REFUSED", "{e:?}");
    assert!(e.message.contains("not registered"), "{e:?}");
    let e = direct_prepare(&w.t, &w.rpc.url, &w.intents, &w.wrap, &seed2, "nobody.example", now_ms()).unwrap_err();
    assert!(e.message.contains("does not exist"), "{e:?}");
}

/// Measurement: `ZECRET_GAS_SWEEP=1` runs the direct exit with several amounts of gas
/// attached from a well funded key and prints what each burnt and whether it delivered.
#[test]
fn gas_sweep() {
    if std::env::var("ZECRET_GAS_SWEEP").is_err() {
        return;
    }
    let w = world();
    for (i, tgas) in [30u64, 35, 40, 50].into_iter().enumerate() {
        let (seed, acct, receiver) = w.taker(ONE_NEAR, ONE_NEAR, &format!("r{i}"), true);
        w.settle();
        let plan = direct_prepare(&w.t, &w.rpc.url, &w.intents, &w.wrap, &seed, &receiver, now_ms()).unwrap();
        let r = direct_submit_with_gas(&w.t, &w.rpc.url, &seed, &plan, now_ms(), tgas * TGAS);
        w.settle();
        let got = w.balance(&w.wrap, "ft_balance_of", json!({"account_id": receiver}));
        let left = w.balance(&w.intents, "mt_balance_of", json!({"account_id": acct, "token_id": format!("nep141:{}", w.wrap)}));
        let after = w.rpc.account(&acct, &At::Final).unwrap().unwrap().amount;
        println!("{tgas} Tgas: ok={} delivered={got} left_in_intents={left} near_cost={:.6} err={:?}", r.is_ok(), (ONE_NEAR - after) as f64 / ONE_NEAR as f64, r.err().map(|e| e.message));
    }
}
