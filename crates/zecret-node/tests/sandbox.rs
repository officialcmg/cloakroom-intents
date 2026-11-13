//! The NEAR adapter of the core against a NEAR 2.14.1 sandbox with the real escrow, the
//! live wrap.near code and the live intents.near code. No Zcash here.
//!
//! ```sh
//! source .tools/env.sh
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test -p zecret-node --test sandbox -- --nocapture
//! ```
//!
//! The escrow wasm is read from `ESCROW_WASM`, or built in `contracts/near-escrow`.

use group::GroupEncoding;
use serde_json::json;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
    time::Duration,
};
use zecret_core::{
    keys,
    near::{escrow, tx::Action, At, Escrow, NearKey, NearRpc, Signer, SwapState, ONE_NEAR},
};
use zecret_node::{devnet, io::Http};

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
    let port = 20000 + (std::process::id() % 10000) as u16;
    let home = std::env::temp_dir().join(format!("zecret-sandbox-{port}"));
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
    for _ in 0..60 {
        if rpc.block(&At::Final).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Sandbox { child, home, url }
}

fn escrow_wasm() -> Vec<u8> {
    if let Ok(p) = std::env::var("ESCROW_WASM") {
        return std::fs::read(p).unwrap();
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/near-escrow");
    let st = Command::new(env!("CARGO"))
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .current_dir(&dir)
        .env_remove("LD_PRELOAD")
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::read(dir.join("target/wasm32-unknown-unknown/release/near_escrow.wasm")).unwrap()
}

/// Moves sandbox time forward until a final block is at or after `target` (seconds).
/// `sandbox_fast_forward` returns before the blocks exist, so each jump is awaited.
fn advance_to(rpc: &NearRpc, target: u64) {
    loop {
        let fin = rpc.block(&At::Final).unwrap();
        if fin.secs() >= target {
            return;
        }
        let delta = ((target - fin.secs()) / 2).clamp(1, 200);
        let head = rpc.block(&At::Optimistic).unwrap().height;
        rpc.call("sandbox_fast_forward", json!({"delta_height": delta})).unwrap();
        // The final block catches up with the jump only after a while.
        while rpc.block(&At::Final).unwrap().height < head + delta {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Waits until a final block shows the swap in `state`.
fn wait_final(e: &Escrow, id: &[u8; 32], state: SwapState) -> zecret_core::near::SwapView {
    for _ in 0..60 {
        if let Some(v) = e.get_swap(id, &At::Final).unwrap()
            && v.state == state {
                return v;
            }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("swap never reached {state:?} at a final block");
}

#[test]
fn open_ready_commit_claim_withdraw_and_refund() {
    let sb = start_sandbox();
    let rpc = NearRpc::new(Rc::new(Http::new().unwrap()), &sb.url);
    let root: serde_json::Value = serde_json::from_slice(&std::fs::read(sb.home.join("validator_key.json")).unwrap()).unwrap();
    let wasm_dir = PathBuf::from(std::env::var("ZECRET_WASM_DIR").expect("ZECRET_WASM_DIR"));
    let maker_key = NearKey::generate();
    let setup = devnet::Setup {
        rpc: &rpc,
        root: Signer::new(root["account_id"].as_str().unwrap(), NearKey::from_secret_str(root["secret_key"].as_str().unwrap()).unwrap()),
        wrap_wasm: std::fs::read(wasm_dir.join("wrap.near.wasm")).unwrap(),
        intents_wasm: std::fs::read(wasm_dir.join("intents.near.wasm")).unwrap(),
        escrow_wasm: escrow_wasm(),
        maker_key: maker_key.clone(),
        maker_wnear: 100 * ONE_NEAR,
    };
    let net = setup.run().unwrap();

    // The taker's escrow checks (security review 1, NV-01): the deployed escrow runs the
    // expected code, has no access keys, and pays into the expected Intents account.
    let fin = At::Final;
    std::thread::sleep(Duration::from_secs(2));
    escrow::check_escrow(&rpc, &net.escrow, &net.escrow_code_hash, &net.intents, &fin).unwrap();
    let e1 = escrow::check_escrow(&rpc, &net.escrow, &zecret_core::near::tx::code_hash(b"other"), &net.intents, &fin).unwrap_err();
    assert!(format!("{e1:#}").contains("runs code"), "{e1:#}");
    let e2 = escrow::check_escrow(&rpc, &net.escrow, &net.escrow_code_hash, "evil.test.near", &fin).unwrap_err();
    assert!(format!("{e2:#}").contains("pays takers into"), "{e2:#}");
    // the same code in an account that keeps a key is refused
    let keyed = setup.create("escrow2", 10 * ONE_NEAR, &setup.root.key, Some(&setup.escrow_wasm)).unwrap();
    rpc.transact(&Signer::new(&keyed, setup.root.key.clone()), &keyed, vec![Action::call("new", &json!({"intents_account": net.intents}), 30 * zecret_core::near::TGAS, 0)]).unwrap().ok().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let e3 = escrow::check_escrow(&rpc, &keyed, &net.escrow_code_hash, &net.intents, &fin).unwrap_err();
    assert!(format!("{e3:#}").contains("access key"), "{e3:#}");
    let maker = Signer::new(&net.maker, maker_key);
    let e = Escrow { rpc: &rpc, escrow: net.escrow.clone(), token: net.token.clone(), intents: net.intents.clone() };
    let (to, actions) = e.add_storage_funds(ONE_NEAR / 10);
    rpc.transact(&maker, &to, actions).unwrap().ok().unwrap();

    // ---------------------------------------------------------------- claim path
    let a = keys::random_scalar();
    let b = keys::random_scalar();
    let (a_pub, b_pub) = ((keys::g() * a).to_bytes(), (keys::g() * b).to_bytes());
    let taker = NearKey::generate();
    let salt: [u8; 32] = zecret_core::random_bytes();
    let id = escrow::swap_id(&net.maker, &salt);
    let view_id = rpc.view(&net.escrow, "swap_id", &json!({"maker": net.maker, "salt": hex::encode(salt)}), &At::Final).unwrap();
    assert_eq!(view_id.as_str().unwrap(), hex::encode(id), "swap id computed by the client = the contract's");
    let amount = 3 * ONE_NEAR;
    let now = rpc.block(&At::Final).unwrap().secs();
    let (to, actions) = e.open(amount, &salt, &taker.public_bytes(), &a_pub, &b_pub, now + 600, now + 1200, now + 1800, 600);
    let out = rpc.transact(&maker, &to, actions).unwrap();
    assert!(out.is_success(), "open: {:?}", out.failure());
    let v = e.get_swap(&id, &At::Optimistic).unwrap().expect("swap stored");
    assert_eq!((v.amount, v.state, v.taker_account.clone()), (amount, SwapState::Open, taker.implicit_account()));
    assert_eq!(v.a_pub, hex::encode(a_pub));

    // stipend: 0.03 NEAR creates the taker's implicit account
    let stipend = 3 * ONE_NEAR / 100;
    rpc.transact(&maker, &taker.implicit_account(), vec![Action::Transfer { deposit: stipend }]).unwrap().ok().unwrap();
    let (to, actions) = e.set_ready(&id);
    rpc.transact(&maker, &to, actions).unwrap().ok().unwrap();
    wait_final(&e, &id, SwapState::Ready);

    // commit: the hashes the client computes equal the contract's views
    let a_bytes = keys::scalar_to_bytes(&a);
    let c = escrow::commitment(&id, &a_bytes);
    let vc = rpc.view(&net.escrow, "commitment_of", &json!({"id": hex::encode(id), "a": hex::encode(a_bytes)}), &At::Final).unwrap();
    assert_eq!(vc.as_str().unwrap(), hex::encode(c));
    let msg = escrow::commit_message(&net.escrow, &id, &c);
    let vm = rpc.view(&net.escrow, "commit_message", &json!({"id": hex::encode(id), "commitment": hex::encode(c)}), &At::Final).unwrap();
    assert_eq!(vm.as_str().unwrap(), hex::encode(&msg));
    let tsigner = Signer::new(&taker.implicit_account(), taker.clone());
    // a wrong signature is refused
    let (to, actions) = e.commit_claim(&id, &c, &NearKey::generate().sign(&msg));
    let bad = rpc.transact(&tsigner, &to, actions).unwrap();
    assert!(bad.failure().unwrap().contains("BadSignature"), "{:?}", bad.failure());
    let (to, actions) = e.commit_claim(&id, &c, &taker.sign(&msg));
    rpc.transact(&tsigner, &to, actions).unwrap().ok().unwrap();
    wait_final(&e, &id, SwapState::Locked);
    let (to, actions) = e.claim(&id, &a_bytes);
    rpc.transact(&tsigner, &to, actions).map_err(|e| format!("{e:#}")).unwrap().ok().unwrap();
    let v = wait_final(&e, &id, SwapState::Claimed);
    assert_eq!(v.a.unwrap(), hex::encode(a_bytes), "a is readable from contract state");
    std::thread::sleep(Duration::from_secs(2)); // gas refunds arrive a few blocks later
    let left = rpc.account(&taker.implicit_account(), &At::Final).unwrap().unwrap().amount;
    println!("stipend left after commit_claim and claim: {left} of {stipend} yoctoNEAR (spent {})", stipend - left);
    assert_eq!(e.credit(&taker.implicit_account(), &At::Final).unwrap().intents, amount);
    // the taker tries the withdraw itself; if its stipend is not enough, the maker does it
    let (to, actions) = e.withdraw(&taker.implicit_account());
    match rpc.transact(&tsigner, &to, actions.clone()) {
        Ok(o) if o.is_success() => println!("the taker paid for its own withdraw"),
        r => {
            println!("the taker could not withdraw ({:?}); the maker withdraws for it", r.map(|o| o.failure().map(str::to_string)));
            rpc.transact(&maker, &to, actions).unwrap().ok().unwrap();
        }
    }
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(e.intents_balance(&taker.implicit_account(), &At::Final).unwrap(), amount, "the payout is in intents.near under the taker key");
    assert_eq!(e.credit(&taker.implicit_account(), &At::Final).unwrap().intents, 0);

    // ---------------------------------------------------------------- refund path
    let salt2: [u8; 32] = zecret_core::random_bytes();
    let id2 = escrow::swap_id(&net.maker, &salt2);
    let before = e.ft_balance(&net.maker, &At::Final).unwrap();
    let now = rpc.block(&At::Final).unwrap().secs();
    let (to, actions) = e.open(amount, &salt2, &taker.public_bytes(), &a_pub, &b_pub, now + 60, now + 120, now + 600, 60);
    rpc.transact(&maker, &to, actions).unwrap().ok().unwrap();
    let b_bytes = keys::scalar_to_bytes(&b);
    // too early: no refund window is open between t0 and t1
    advance_to(&rpc, now + 70);
    let (to, actions) = e.refund(&id2, &b_bytes);
    let early = rpc.transact(&maker, &to, actions).unwrap();
    assert!(early.failure().map(|f| f.contains("TooEarly")).unwrap_or(false), "{early:?} at {}", rpc.block(&At::Final).unwrap().secs() - now);
    advance_to(&rpc, now + 121);
    let (to, actions) = e.refund(&id2, &b_bytes);
    rpc.transact(&maker, &to, actions).unwrap().ok().unwrap();
    let v = wait_final(&e, &id2, SwapState::Refunded);
    assert_eq!(v.b.unwrap(), hex::encode(b_bytes), "b is readable from contract state");
    let (to, actions) = e.withdraw(&net.maker);
    rpc.transact(&maker, &to, actions).unwrap().ok().unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(e.ft_balance(&net.maker, &At::Final).unwrap(), before, "the refund returned the maker's tokens");
}
