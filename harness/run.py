#!/usr/bin/env python3
"""Zecret scenario runner.

Runs the full two-party protocol on a local Zebra regtest node (Docker, harness/zebra/)
and a local NEAR sandbox (nearcore 2.14.1) holding the escrow (contracts/near-escrow),
the live wrap.near code (the stand-in token) and the live intents.near code. The maker
and the taker are separate OS processes (`crates/target/debug/zecret`); they communicate
only through encrypted Zcash memos and the escrow.

  source .tools/env.sh
  python3 -I harness/run.py            # every scenario (NU7 chain, then s7 on NU6.3)
  python3 -I harness/run.py s1 s4      # selected scenarios

Evidence for every scenario is written to harness/evidence/<scenario>/ and a summary to
harness/RESULTS.md. The sandbox time is moved forward with `sandbox_fast_forward` where a
scenario needs a deadline to pass; Zcash blocks are mined in real time.
"""
import base64
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CRATES = os.path.join(ROOT, "crates")
BIN = os.environ.get("ZECRET_BIN") or os.path.join(CRATES, "target", "debug", "zecret")
ESCROW_DIR = os.path.join(ROOT, "contracts", "near-escrow")
ESCROW_WASM = os.path.join(ESCROW_DIR, "target", "wasm32-unknown-unknown", "release", "near_escrow.wasm")
EVID = os.path.join(HERE, "evidence")
WORK = os.path.join(HERE, "work")
ZEBRA = "http://127.0.0.1:18232"
NEAR_PORT = 3030
NEAR = f"http://127.0.0.1:{NEAR_PORT}"

ONE_NEAR = 10 ** 24
LOT = 100_000_000                 # 1 ZEC sold per swap
FEE = 10_000                      # ZIP 317 fee of every swap transaction (2 actions)
PRICE_PER_ZEC = 30 * ONE_NEAR     # 30 wNEAR per ZEC (wNEAR stands in for USDC)
PRICE = LOT * PRICE_PER_ZEC // 100_000_000
PREMIUM = 50_000                  # zatoshis paid to the maker with the acceptance
STIPEND = 3 * ONE_NEAR // 100     # 0.03 NEAR
COVER_K = 4
# NEAR timing for the sandbox (seconds): D, F, R, fund window, k_Z.
TIMING = {"inclusion_secs": 3, "finality_secs": 3, "reorg_budget": 2, "margin_secs": 0, "fund_secs": 30, "k_z": 2}
BUDGET = 3 * 3 + 2 * 3            # B_R = (R+1)D + RF
WINDOWS = {"t0_secs": 120, "t1_secs": 60, "t2_secs": 60, "reveal_secs": 45}

state = {"nu7": None, "net": None, "branch": None}
zebra_proc = None
near_proc = None
stop_miner = threading.Event()
mine_lock = threading.Lock()


def log(msg):
    print(f"[harness {time.strftime('%H:%M:%S')}] {msg}", flush=True)


def post(url, body, timeout=60):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def zrpc(method, params=None):
    resp = post(ZEBRA, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []})
    if resp.get("error"):
        raise RuntimeError(f"{method}: {resp['error']}")
    return resp["result"]


def nrpc(method, params):
    resp = post(NEAR, {"jsonrpc": "2.0", "id": "h", "method": method, "params": params})
    if resp.get("error"):
        raise RuntimeError(f"NEAR {method}: {resp['error']}")
    return resp["result"]


def near_block(finality="final"):
    h = nrpc("block", {"finality": finality})["header"]
    return h["height"], h["timestamp"] // 10 ** 9


def near_now():
    return near_block()[1]


def view(contract, method, args):
    r = nrpc("query", {"request_type": "call_function", "finality": "final", "account_id": contract,
                       "method_name": method, "args_base64": base64.b64encode(json.dumps(args).encode()).decode()})
    return json.loads(bytes(r["result"]).decode()) if r["result"] else None


def near_account(acct):
    try:
        return int(nrpc("query", {"request_type": "view_account", "finality": "final", "account_id": acct})["amount"])
    except RuntimeError as e:
        if "UNKNOWN_ACCOUNT" in str(e) or "does not exist" in str(e):
            return None
        raise


def get_swap(eid):
    return view(state["net"]["escrow"], "get_swap", {"id": eid})


def escrow_state(eid):
    s = get_swap(eid)
    return s["state"] if s else None


def wnear(acct):
    return int(view(state["net"]["token"], "ft_balance_of", {"account_id": acct}))


def intents_balance(acct):
    return int(view(state["net"]["intents"], "mt_balance_of", {"account_id": acct, "token_id": "nep141:" + state["net"]["token"]}))


def warp_to(ts):
    """Move NEAR sandbox time forward until a final block is at or after ts. Each jump is
    awaited: sandbox_fast_forward returns before the blocks exist, and the final block
    catches up later."""
    start = near_now()
    while True:
        h, t = near_block()
        if t >= ts:
            break
        delta = max(1, min(200, (ts - t) // 2))
        head = near_block("optimistic")[0]
        nrpc("sandbox_fast_forward", {"delta_height": delta})
        while near_block()[0] < head + delta:
            time.sleep(0.2)
    log(f"warped NEAR time forward {near_now() - start}s to {near_now()}")


def sh(*args, cwd=None, check=True, env=None):
    p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, env=env)
    if check and p.returncode != 0:
        raise RuntimeError(f"{' '.join(args[:3])}...: {p.stderr.strip() or p.stdout.strip()}")
    return p.stdout.strip()


def wait_for(fn, timeout, what):
    t = time.time()
    while time.time() - t < timeout:
        v = fn()
        if v:
            return v
        time.sleep(0.5)
    raise RuntimeError(f"timed out waiting for {what}")


# ---------------------------------------------------------------- chains

def miner():
    """Mine a Zcash block whenever the mempool is non-empty, and every 3 s regardless."""
    last = 0.0
    while not stop_miner.is_set():
        try:
            with mine_lock:
                if zrpc("getrawmempool") or time.time() - last > 3:
                    zrpc("generate", [1])
                    last = time.time()
        except Exception:
            pass
        time.sleep(0.4)


def build():
    log("building the escrow wasm and the zecret binary")
    env = {k: v for k, v in os.environ.items() if k not in ("LD_PRELOAD", "CARGO_TARGET_DIR")}
    sh("cargo", "build", "--target", "wasm32-unknown-unknown", "--release", cwd=ESCROW_DIR, env=env)
    sh("cargo", "build", "-p", "zecret-node", cwd=CRATES, env=env)


def start_zebra(nu7):
    global zebra_proc
    conf = "zebrad-nu7.toml" if nu7 else "zebrad-nu63.toml"
    env = dict(os.environ, ZEBRA_CONF=conf)
    subprocess.run(["docker", "compose", "down"], cwd=os.path.join(HERE, "zebra"), env=env, capture_output=True)
    subprocess.run(["docker", "compose", "up", "-d"], cwd=os.path.join(HERE, "zebra"), env=env, check=True, capture_output=True)
    for _ in range(90):
        try:
            zrpc("getblockchaininfo")
            break
        except Exception:
            time.sleep(1)
    zrpc("generate", [3])
    info = zrpc("getblockchaininfo")
    log(f"zebra up: {zrpc('getinfo')['subversion']} tip {info['blocks']} next-block branch {info['consensus']['nextblock']}")
    state["nu7"] = 2 if nu7 else None
    state["branch"] = info["consensus"]["nextblock"]


def start_near():
    global near_proc
    home = os.path.join(WORK, "near")
    shutil.rmtree(home, ignore_errors=True)
    binp = os.environ["NEAR_SANDBOX_BIN_PATH"]
    env = dict(os.environ)
    if os.environ.get("ZECRET_SANDBOX_PRELOAD"):
        env["LD_PRELOAD"] = os.environ["ZECRET_SANDBOX_PRELOAD"]
    sh(binp, "--home", home, "init", env=env)
    cfgp = os.path.join(home, "config.json")
    cfg = json.load(open(cfgp))
    cfg["rpc"]["addr"] = f"127.0.0.1:{NEAR_PORT}"
    json.dump(cfg, open(cfgp, "w"))
    near_proc = subprocess.Popen([binp, "--home", home, "run"], env=env, stdout=open(os.path.join(WORK, "near.log"), "w"), stderr=subprocess.STDOUT)
    wait_for(lambda: _near_up(), 60, "NEAR sandbox")
    maker_seed = os.path.join(WORK, "maker-near")
    shutil.rmtree(maker_seed, ignore_errors=True)
    out = sh(BIN, "devnet", "--rpc", NEAR, "--root-key", os.path.join(home, "validator_key.json"),
             "--wasm-dir", os.environ["ZECRET_WASM_DIR"], "--escrow-wasm", ESCROW_WASM, "--maker-dir", maker_seed,
             "--maker-wnear", "10000")
    state["net"] = json.loads(out)
    state["maker_key"] = os.path.join(maker_seed, "near_key.json")
    ver = post(NEAR, {"jsonrpc": "2.0", "id": 1, "method": "status", "params": []})["result"]["version"]
    keys = nrpc("query", {"request_type": "view_access_key_list", "finality": "final", "account_id": state["net"]["escrow"]})["keys"]
    log(f"escrow code hash {state['net']['escrow_code_hash']}, access keys on the escrow account: {len(keys)}")
    log(f"NEAR sandbox up: nearcore {ver['version']}; token {state['net']['token']}, intents {state['net']['intents']}, escrow {state['net']['escrow']}, maker {state['net']['maker']}")


def _near_up():
    try:
        near_block()
        return True
    except Exception:
        return False


def start_chains(nu7):
    start_zebra(nu7)
    if near_proc is None:
        start_near()
    stop_miner.clear()
    threading.Thread(target=miner, daemon=True).start()


def stop_chains():
    stop_miner.set()
    time.sleep(0.5)
    subprocess.run(["docker", "compose", "down"], cwd=os.path.join(HERE, "zebra"), capture_output=True)


def stop_near():
    global near_proc
    if near_proc:
        near_proc.terminate()
        near_proc.wait()
        near_proc = None


# ---------------------------------------------------------------- parties

def chains():
    n = state["net"]
    return {"zebra_rpc": ZEBRA, "nu7_height": state["nu7"], "near_rpc": NEAR, "escrow": n["escrow"], "token": n["token"], "intents": n["intents"]}


class Party:
    def __init__(self, scen, role, cfg):
        self.dir = os.path.join(WORK, scen, role)
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(self.dir)
        self.role, self.scen = role, scen
        self.logpath = os.path.join(EVID, scen, f"{role}.log")
        open(self.logpath, "w").close()
        self.write_config(cfg)
        self.ua = sh(BIN, "init", "--dir", self.dir)
        self.proc = None

    def write_config(self, cfg):
        with open(os.path.join(self.dir, "config.json"), "w") as f:
            json.dump(cfg, f, indent=2)

    def config(self):
        return json.load(open(os.path.join(self.dir, "config.json")))

    def fund_coinbase(self):
        with mine_lock:
            zrpc("generatetoaddress", [1, self.ua])

    def balance(self):
        return json.loads(sh(BIN, "balance", "--dir", self.dir))["zats"]

    def start(self, *args):
        f = open(self.logpath, "a")
        f.write(f"\n===== start {self.role} {' '.join(args)} =====\n")
        f.flush()
        self.proc = subprocess.Popen([BIN, self.role, "--dir", self.dir, *args], stdout=f, stderr=subprocess.STDOUT)
        return self.proc

    def wait(self, timeout):
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            raise RuntimeError(f"{self.role} timed out after {timeout}s (see {self.logpath})")

    def kill(self):
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()

    def logtext(self):
        return open(self.logpath).read()

    def wait_log(self, needle, timeout):
        t = time.time()
        while time.time() - t < timeout:
            if needle in self.logtext():
                return
            if self.proc and self.proc.poll() is not None and needle not in self.logtext():
                raise RuntimeError(f"{self.role} exited before logging {needle!r}")
            time.sleep(0.5)
        raise RuntimeError(f"timeout waiting for {self.role} to log {needle!r}")

    def swap(self):
        d = os.path.join(self.dir, "swap")
        files = sorted(f for f in os.listdir(d) if f.endswith(".json")) if os.path.isdir(d) else []
        return json.load(open(os.path.join(d, files[0]))) if files else None


def stage_name(sw):
    st = (sw or {}).get("stage")
    return next(iter(st)) if isinstance(st, dict) else st


def abort_reason(sw):
    st = (sw or {}).get("stage")
    return st.get("Aborted", "") if isinstance(st, dict) else ""


def parties(scen, maker_extra=None, taker_extra=None):
    os.makedirs(os.path.join(EVID, scen), exist_ok=True)
    mcfg = {"chains": chains(), "timing": TIMING, "near_account": state["net"]["maker"], "price_per_zec": str(PRICE_PER_ZEC),
            "min_zats": 1000, "max_zats": 5 * LOT, "premium_zats": PREMIUM, "stipend": str(STIPEND), "windows": WINDOWS}
    mcfg.update(maker_extra or {})
    maker = Party(scen, "maker", mcfg)
    shutil.copy(state["maker_key"], os.path.join(maker.dir, "near_key.json"))
    entry = json.loads(sh(BIN, "directory", "--dir", maker.dir))
    tcfg = {"chains": chains(), "timing": TIMING, "maker": entry, "escrow_code_hash": state["net"]["escrow_code_hash"],
            "min_price": str(PRICE), "max_premium": 2 * PREMIUM,
            "min_stipend": str(STIPEND), "quote_timeout_secs": 400}
    tcfg.update(taker_extra or {})
    taker = Party(scen, "taker", tcfg)
    maker.fund_coinbase()
    taker.fund_coinbase()
    time.sleep(4)
    for p in (maker, taker):
        if p.balance() <= 0:
            raise RuntimeError(f"{p.role} wallet not funded")
    return maker, taker


def zec_txids(maker, taker):
    t, m = taker.swap() or {}, maker.swap() or {}
    ids = [t.get(k) for k in ("rfq_txid", "accept_txid", "fund_txid", "sweep_txid")] + [m.get(k) for k in ("quote_txid", "sweep_txid")]
    for sw in (t, m):
        p = sw.get("pending") or {}
        ids.append(p.get("txid"))
    return sorted({i for i in ids if i})


def privacy_audit(c, maker, taker):
    """Every Zcash transaction either party broadcast must be fully shielded: no transparent
    inputs or outputs, no Sapling or Orchard-pool activity, Ironwood actions only."""
    bad, seen = [], []
    for txid in zec_txids(maker, taker):
        try:
            v = zrpc("getrawtransaction", [txid, 1])
        except RuntimeError:
            continue  # built but never mined (for example a funding that expired)
        seen.append(txid)
        iw = v.get("ironwood") or {}
        shielded_only = not v.get("vin") and not v.get("vout") and not (v.get("orchard") or {}).get("actions") \
            and not v.get("vShieldedSpend") and not v.get("vShieldedOutput")
        if not shielded_only or not iw.get("actions"):
            bad.append(txid)
        c.facts.setdefault("zcash_txs", []).append({"txid": txid, "height": v.get("height"), "version": v.get("version"),
            "ironwood_actions": len(iw.get("actions", [])), "ironwood_value_balance": iw.get("valueBalanceZat", iw.get("valueBalance")),
            "transparent_in": len(v.get("vin") or []), "transparent_out": len(v.get("vout") or [])})
    c.ok(f"privacy audit: all {len(seen)} Zcash txs by maker/taker are fully shielded Ironwood v6 (no transparent in/out)", seen and not bad, bad)


MAKER_FINAL = {"Swept", "Refunded", "Aborted"}
TAKER_FINAL = {"Completed", "Recovered", "Aborted"}


def secrets_erased(c, maker, taker):
    """R4 for every party in a terminal stage. A party that vanished mid-swap keeps its
    secrets (it needs them when it returns), so it is reported and not checked."""
    m, t = maker.swap() or {}, taker.swap() or {}
    checked, skipped, bad = [], [], []
    if stage_name(m) in MAKER_FINAL:
        checked.append("maker:" + stage_name(m))
        if m.get("b") or m.get("s_m") or m.get("e_m") or m.get("joint_fvk"):
            bad.append("maker")
    elif m:
        skipped.append("maker:" + str(stage_name(m)))
    if stage_name(t) in TAKER_FINAL:
        checked.append("taker:" + stage_name(t))
        if t.get("a") or t.get("s_t") or t.get("e_t") or t.get("joint_fvk"):
            bad.append("taker")
    elif t:
        skipped.append("taker:" + str(stage_name(t)))
    c.facts["r4_checked"], c.facts["r4_not_terminal"] = checked, skipped
    c.ok("R4: per-swap secrets erased by every party in a terminal stage (b, s_M, e_M / a, s_T, e_T; joint FVK)",
         checked and not bad, f"checked={checked} not_terminal={skipped} bad={bad}")


class Checks:
    def __init__(self, scen, title):
        self.scen, self.title, self.items, self.facts = scen, title, [], {}

    def ok(self, desc, cond, detail=""):
        self.items.append({"check": desc, "pass": bool(cond), "detail": str(detail)})
        log(f"  {'PASS' if cond else 'FAIL'}  {desc} {detail}")

    def save(self, maker, taker):
        has_swap = bool(maker.swap() or taker.swap())
        if has_swap and not any(i["check"].startswith("privacy audit") for i in self.items):
            privacy_audit(self, maker, taker)
        if has_swap and not any(i["check"].startswith("R4:") for i in self.items):
            secrets_erased(self, maker, taker)
        rec = {"scenario": self.scen, "title": self.title, "chain_branch": state.get("branch"), "checks": self.items, "facts": self.facts,
               "maker_swap": redact(maker.swap()), "taker_swap": redact(taker.swap())}
        with open(os.path.join(EVID, self.scen, "result.json"), "w") as f:
            json.dump(rec, f, indent=2)
        return all(i["pass"] for i in self.items)


SECRET_FIELDS = ("a", "b", "s_t", "s_m", "e_t", "e_m", "joint_fvk", "near_sk", "quote_memo")


def redact(sw):
    if not sw:
        return sw
    sw = dict(sw)
    for k in SECRET_FIELDS:
        if sw.get(k):
            sw[k] = "<redacted>"
    if sw.get("pending"):
        sw["pending"] = dict(sw["pending"], raw_hex="<omitted>")
    return sw


def settle():
    """Wait until every broadcast Zcash transaction is mined, plus one more block."""
    for _ in range(60):
        if not zrpc("getrawmempool"):
            break
        time.sleep(0.5)
    h = zrpc("getblockcount")
    while zrpc("getblockcount") <= h:
        time.sleep(0.3)


def snapshot(maker, taker):
    settle()
    return {"maker_zec": maker.balance(), "taker_zec": taker.balance(), "maker_wnear": wnear(state["net"]["maker"])}


# ---------------------------------------------------------------- scenarios

def s1_happy(scen="s1_happy", title="Happy path: request -> quote -> acceptance + premium -> open + stipend -> fund -> set_ready -> commit_claim -> claim -> sweep -> withdraw into intents.near"):
    maker, taker = parties(scen)
    c = Checks(scen, title)
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT))
    taker.wait(600)
    maker.wait(300)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    acct = t["taker_account"]
    c.facts.update(before=before, after=after, escrow_id=t["escrow_id"], taker_account=acct)
    c.ok("taker reached Completed", stage_name(t) == "Completed", stage_name(t))
    c.ok("maker reached Swept", stage_name(m) == "Swept", stage_name(m))
    c.ok("escrow state Claimed at a final block", escrow_state(t["escrow_id"]) == "Claimed")
    c.ok("the taker's NEAR account is the hex of its ed25519 key and the stipend created it", near_account(acct) is not None and len(acct) == 64, near_account(acct))
    c.ok("taker sent commit_claim and claim itself (no relayer)", t.get("commit_tx") and t.get("claim_tx"))
    c.ok("taker's balance inside intents.near equals the price", intents_balance(acct) == PRICE, intents_balance(acct))
    c.ok("escrow credit of the taker is zero after withdraw", int(view(state["net"]["escrow"], "get_credit", {"token": state["net"]["token"], "owner": acct})["intents"]) == 0)
    c.ok("maker paid exactly the price", before["maker_wnear"] - after["maker_wnear"] == PRICE, before["maker_wnear"] - after["maker_wnear"])
    c.ok("maker ZEC: +lot + premium - quote fee (exact)", after["maker_zec"] - before["maker_zec"] == LOT + PREMIUM - FEE, after["maker_zec"] - before["maker_zec"])
    c.ok("taker ZEC: -(lot + sweep fee) - premium - 3 tx fees (exact)", before["taker_zec"] - after["taker_zec"] == LOT + FEE + PREMIUM + 3 * FEE, before["taker_zec"] - after["taker_zec"])
    privacy_audit(c, maker, taker)
    secrets_erased(c, maker, taker)
    return c.save(maker, taker)


def s2_taker_vanishes_before_funding():
    scen = "s2_taker_vanishes_before_funding"
    maker, taker = parties(scen)
    c = Checks(scen, "Taker accepts (pays premium), then disappears before funding: maker refunds before t0 (R3e)")
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT), "--vanish-at", "Accepted")
    taker.wait(300)
    maker.wait_log("STIPEND", 200)
    m = maker.swap()
    warp_to(m["t0"] - TIMING["fund_secs"] // 2)
    maker.wait(200)
    m = maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("taker vanished at Accepted", stage_name(taker.swap()) == "Accepted")
    c.ok("maker refunded early because nothing was funded", stage_name(m) == "Refunded" and any("no funding before t0" in l for l in m["log"]), stage_name(m))
    c.ok("escrow state Refunded", escrow_state(m["escrow_id"]) == "Refunded")
    c.ok("maker tokens fully restored (withdrawn)", after["maker_wnear"] == before["maker_wnear"], after["maker_wnear"] - before["maker_wnear"])
    c.ok("maker kept the premium: ZEC +premium - quote fee (exact)", after["maker_zec"] - before["maker_zec"] == PREMIUM - FEE)
    return c.save(maker, taker)


def s3_taker_vanishes_after_funding():
    scen = "s3_taker_vanishes_after_funding"
    maker, taker = parties(scen)
    c = Checks(scen, "Maker refund path: the taker funds, then disappears and never commits; returns after t1: R2c refuses to commit, the maker refunds from a final block past t1 (R3d), the taker recovers the ZEC with a + b")
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT), "--vanish-at", "Funded")
    taker.wait(400)
    maker.wait_log("READY", 200)
    m = maker.swap()
    warp_to(m["t1"] + 1)
    taker.start()
    taker.wait_log("R2c", 120)
    taker.wait(300)
    maker.wait(200)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("taker refused to commit after t1 (R2c)", any(l.startswith("R2c") for l in t["log"]))
    c.ok("maker refunded only from a final block past t1", stage_name(m) == "Refunded" and any("past t1" in l for l in m["log"]), stage_name(m))
    c.ok("taker recovered the joint note with a + b", stage_name(t) == "Recovered" and any("swept the joint note back" in l for l in t["log"]), stage_name(t))
    c.ok("maker tokens fully restored", after["maker_wnear"] == before["maker_wnear"])
    c.ok("taker ZEC: premium + 4 tx fees lost (exact)", before["taker_zec"] - after["taker_zec"] == PREMIUM + 4 * FEE, before["taker_zec"] - after["taker_zec"])
    c.ok("a never published (escrow has no a)", get_swap(m["escrow_id"]).get("a") is None)
    return c.save(maker, taker)


def s4_maker_vanishes_after_open():
    scen = "s4_maker_vanishes_after_open"
    maker, taker = parties(scen)
    c = Checks(scen, "Punish path: the maker opens and sends the stipend, then disappears (never sets Ready) after the taker funds. From t2 the taker commits and claims; the returning maker sweeps the ZEC and pays the taker's credit into intents.near")
    before = snapshot(maker, taker)
    maker.start("--vanish-at", "Opened")
    taker.start("--zats", str(LOT))
    maker.wait(300)
    taker.wait_log("FUNDED", 200)
    m = maker.swap()
    warp_to(m["t2"] + 1)
    taker.wait_log("punish path", 120)
    taker.wait_log("claim is final", 120)
    claim_final_before = maker.proc.poll() is not None
    maker.start("--exit-after", "1")  # returns after the claim
    maker.wait(300)
    taker.wait(300)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("taker locked via the punish path after t2", any("punish path" in l for l in t["log"]))
    c.ok("taker's claim was final before the maker returned (no hostage)", claim_final_before)
    c.ok("taker reached Completed", stage_name(t) == "Completed", stage_name(t))
    c.ok("returning maker swept the ZEC", stage_name(m) == "Swept", stage_name(m))
    c.ok("taker's balance inside intents.near equals the price", intents_balance(t["taker_account"]) == PRICE)
    c.ok("maker ZEC: +lot + premium - quote fee (exact)", after["maker_zec"] - before["maker_zec"] == LOT + PREMIUM - FEE)
    return c.save(maker, taker)


def quote_rejected(scen, evil, title, why):
    maker, taker = parties(scen, maker_extra={"misbehave": evil})
    c = Checks(scen, title)
    before = snapshot(maker, taker)
    maker.start()
    taker.start("--zats", str(LOT))
    taker.wait(300)
    maker.kill()
    t = taker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after)
    c.ok(f"taker rejected the quote ({why})", stage_name(t) == "Aborted" and why in abort_reason(t), t["stage"])
    c.ok("taker never funded and paid no premium (only the request fee)", t.get("fund_txid") is None and before["taker_zec"] - after["taker_zec"] == FEE, before["taker_zec"] - after["taker_zec"])
    return c.save(maker, taker)


def s5_rogue_key():
    return quote_rejected("s5_rogue_key", "rogue-key", "Malicious maker sends a rogue key B = [b']G - A: taker rejects the quote and never funds", "proof of knowledge")


def s6_underfund():
    scen = "s6_underfund"
    maker, taker = parties(scen, taker_extra={"misbehave": "underfund"})
    c = Checks(scen, "Taker underfunds the joint address: maker refuses Ready and refunds before t0; taker recovers")
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT))
    taker.wait_log("FUNDED", 300)
    time.sleep(8)
    m = maker.swap()
    warp_to(m["t0"] - TIMING["fund_secs"] // 2)
    maker.wait(200)
    taker.wait(300)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("maker did not set Ready", not m.get("ready_tx"))
    c.ok("maker refunded because underfunded", stage_name(m) == "Refunded" and any("underfunded" in l for l in m["log"]))
    c.ok("taker recovered", stage_name(t) == "Recovered", stage_name(t))
    c.ok("maker tokens fully restored", after["maker_wnear"] == before["maker_wnear"])
    c.ok("taker ZEC: premium + 4 tx fees lost (exact)", before["taker_zec"] - after["taker_zec"] == PREMIUM + 4 * FEE, before["taker_zec"] - after["taker_zec"])
    return c.save(maker, taker)


def s7_happy_nu63():
    return s1_happy("s7_happy_nu63", "Happy path again, on a Zcash chain without NU7 (NU6.3 rules)")


def s8_free_claim_attempt():
    scen = "s8_free_claim_attempt"
    maker, taker = parties(scen, taker_extra={"misbehave": "free-claim"})
    c = Checks(scen, "A taker who never funds tries to claim while the maker is offline. Rejected before t2 (claims need Ready); the maker comes back once in [t1, t2) (assumption L_M) and refunds, closing the punish path")
    before = snapshot(maker, taker)
    maker.start("--vanish-at", "Opened")
    taker.start("--zats", str(LOT))
    maker.wait(300)
    taker.wait_log("NOT funding", 200)
    m = maker.swap()
    warp_to(m["t0"] + 1)
    taker.wait_log("REJECTED", 120)
    warp_to(m["t1"] + 1)
    maker.start("--exit-after", "1")
    maker.wait(200)
    taker.wait(200)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("unfunded lock before t2 rejected by the escrow", any("REJECTED" in l for l in t["log"]))
    c.ok("maker refunded within [t1, t2)", stage_name(m) == "Refunded" and any("past t1" in l for l in m["log"]), stage_name(m))
    c.ok("no claim: a never published", get_swap(m["escrow_id"]).get("a") is None)
    c.ok("maker tokens fully restored", after["maker_wnear"] == before["maker_wnear"])
    return c.save(maker, taker)


def s9_late_commit():
    scen = "s9_late_commit"
    maker, taker = parties(scen)
    c = Checks(scen, "A commit that lands after the deadline: the late commit_claim fails publishing only the commitment; maker refunds; taker recovers; a is never revealed")
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT), "--vanish-at", "Funded")
    taker.wait(400)
    maker.wait_log("READY", 200)
    m = maker.swap()
    warp_to(m["t1"] + 1)
    cfg = taker.config()
    cfg["misbehave"] = "late-commit"
    taker.write_config(cfg)
    taker.start()
    taker.wait_log("REVERTED", 120)
    maker.wait(200)
    taker.wait(300)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("late commit failed", any("REVERTED" in l for l in t["log"]))
    c.ok("no claim: a never published", get_swap(m["escrow_id"]).get("a") is None)
    c.ok("maker refunded (did not sweep: it never learned a)", stage_name(m) == "Refunded", stage_name(m))
    c.ok("taker recovered", stage_name(t) == "Recovered", stage_name(t))
    c.ok("taker ZEC: premium + 4 tx fees lost (exact)", before["taker_zec"] - after["taker_zec"] == PREMIUM + 4 * FEE, before["taker_zec"] - after["taker_zec"])
    return c.save(maker, taker)


def s10_lock_without_reveal():
    scen = "s10_lock_without_reveal"
    maker, taker = parties(scen)
    c = Checks(scen, "Taker locks (commit_claim) then disappears before revealing: maker refunds once a final block is past the reveal deadline (R3l); taker returns and recovers")
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT), "--vanish-at", "Committed")
    taker.wait(400)
    eid = taker.swap()["escrow_id"]
    sw = wait_for(lambda: (lambda s: s if s and s["state"] == "Locked" else None)(get_swap(eid)), 60, "lock final")
    warp_to(sw["locked_at"] + sw["reveal_window"] + 1)
    maker.wait(200)
    taker.start()
    taker.wait(300)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("escrow was Locked before the refund", any("LOCKED" in l for l in t["log"]))
    c.ok("maker refunded after the reveal deadline", stage_name(m) == "Refunded" and any("did not reveal" in l for l in m["log"]), stage_name(m))
    c.ok("taker recovered", stage_name(t) == "Recovered", stage_name(t))
    c.ok("no claim: a never published", get_swap(m["escrow_id"]).get("a") is None)
    c.ok("maker tokens fully restored", after["maker_wnear"] == before["maker_wnear"])
    return c.save(maker, taker)


def s11_maker_liveness_violated():
    scen = "s11_maker_liveness_violated"
    maker, taker = parties(scen, taker_extra={"misbehave": "free-claim"})
    c = Checks(scen, "Assumption boundary (Proposition 3): the maker is offline for ALL of [t1, t2) and the taker never funds. The punish path then lets her claim for free. This is the expected, documented consequence of violating L_M")
    before = snapshot(maker, taker)
    maker.start("--vanish-at", "Opened")
    taker.start("--zats", str(LOT))
    maker.wait(300)
    taker.wait_log("NOT funding", 200)
    m = maker.swap()
    warp_to(m["t2"] + 1)
    taker.wait_log("punish path", 120)
    taker.wait_log("claim is final", 120)
    maker.start("--exit-after", "1")
    maker.wait(200)
    taker.wait(200)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("taker never funded the joint address", t.get("fund_txid") is None)
    c.ok("taker claimed via the punish path and was paid", stage_name(t) == "Completed", stage_name(t))
    c.ok("maker lost the escrowed tokens, as Proposition 3 predicts", before["maker_wnear"] - after["maker_wnear"] == PRICE)
    c.ok("maker detected and reported the L_M violation", any("L_M" in l for l in m["log"]), m["stage"])
    return c.save(maker, taker)


def lookalikes_between(lo, hi):
    """Fully shielded, Ironwood-only, 2-action txs in blocks lo < h < hi (independent count)."""
    n = 0
    for h in range(lo + 1, hi):
        for txid in zrpc("getblock", [str(h), 1])["tx"]:
            v = zrpc("getrawtransaction", [txid, 1])
            iw = v.get("ironwood") or {}
            if not v.get("vin") and not v.get("vout") and not (v.get("orchard") or {}).get("actions") \
                    and not v.get("vShieldedSpend") and not v.get("vShieldedOutput") and len(iw.get("actions", [])) == 2:
                n += 1
    return n


def s12_cover_traffic():
    scen = "s12_cover_traffic"
    cover = {"cover": {"min_secs": 5, "max_secs": 15, "k": COVER_K}}
    maker, taker = parties(scen, maker_extra=cover, taker_extra=cover)
    noise = Party(scen, "noise", {"chains": chains()})
    noise.fund_coinbase()
    time.sleep(4)
    stop = threading.Event()

    def chatter():
        while not stop.is_set():
            subprocess.run([BIN, "pay", "--dir", noise.dir, "--to", noise.ua, "--zats", "1000"], capture_output=True)
            time.sleep(2)
    th = threading.Thread(target=chatter, daemon=True)
    th.start()
    c = Checks(scen, f"Cover traffic: with look-alike background traffic, every delayed Zcash broadcast waits until at least k={COVER_K} look-alikes have been mined since its trigger, and the swap still completes")
    try:
        maker.start("--exit-after", "1")
        taker.start("--zats", str(LOT))
        taker.wait(900)
        maker.wait(400)
    finally:
        stop.set()
        th.join(timeout=30)
    t, m = taker.swap(), maker.swap()
    c.ok("swap completed under cover delays", stage_name(t) == "Completed" and stage_name(m) == "Swept", (stage_name(t), stage_name(m)))
    logs = taker.logtext() + maker.logtext()
    waits = re.findall(r"(QUOTE|ACCEPT|FUND|SWEEP|OPEN|READY): cover: waited (\d+)s over (\d+) random draw\(s\), (\d+) look-alike", logs)
    c.facts["cover_waits"] = waits
    c.ok("the delayable Zcash broadcasts waited for cover", {"ACCEPT", "FUND", "QUOTE", "SWEEP"} <= {w[0] for w in waits}, sorted({w[0] for w in waits}))
    c.ok(f"each wait saw >= {COVER_K} look-alikes (wallet count)", waits and all(int(w[3]) >= COVER_K for w in waits))
    txs = {"RFQ": t.get("rfq_txid"), "QUOTE": m.get("quote_txid"), "ACCEPT": t.get("accept_txid")}
    if all(txs.values()):
        h = {k: zrpc("getrawtransaction", [v, 1])["height"] for k, v in txs.items()}
        rq, qa = lookalikes_between(h["RFQ"], h["QUOTE"]), lookalikes_between(h["QUOTE"], h["ACCEPT"])
        c.facts["independent_counts"] = {"rfq_to_quote": rq, "quote_to_accept": qa, "heights": h}
        c.ok(f"independent chain count: >= {COVER_K} look-alikes mined between RFQ and QUOTE, and between QUOTE and ACCEPT", rq >= COVER_K and qa >= COVER_K, (rq, qa))
    else:
        c.ok("found RFQ/QUOTE/ACCEPT txids for the independent check", False, txs)
    privacy_audit(c, maker, taker)
    return c.save(maker, taker)


def s15_impostor_directory():
    scen = "s15_impostor_directory"
    maker, taker = parties(scen)
    c = Checks(scen, "Impostor directory entries: (1) the real maker's NEAR key but signed by another key; (2) signed by a key that is not an access key of the maker's account. The taker refuses both before sending a request or paying anything")
    cfg = taker.config()
    good = dict(cfg["maker"])
    other = os.path.join(WORK, scen, "impostor")
    shutil.rmtree(other, ignore_errors=True)
    os.makedirs(other)
    sh(BIN, "near-key", "--dir", other)
    shutil.copy(os.path.join(maker.dir, "wallet.json"), other)
    shutil.copy(os.path.join(maker.dir, "rfq_key.json"), other)
    json.dump(maker.config(), open(os.path.join(other, "config.json"), "w"))
    forged = json.loads(sh(BIN, "directory", "--dir", other))  # same UA, epoch, RFQ key, account; impostor's key
    before = snapshot(maker, taker)
    codes, outs = [], []
    for entry in ({**good, "sig": forged["sig"]}, forged):
        cfg["maker"] = entry
        taker.write_config(cfg)
        taker.start("--zats", str(LOT))
        codes.append(taker.wait(60))
        outs.append(taker.logtext())
    after = snapshot(maker, taker)
    c.ok("entry signed by another key: taker refused to start", codes[0] != 0 and "is not signed by" in outs[0], codes[0])
    c.ok("entry with a key that is not the maker account's: taker refused to start", codes[1] != 0 and "is not a key of" in outs[1], codes[1])
    c.ok("no request sent and nothing paid", after["taker_zec"] == before["taker_zec"] and taker.swap() is None)
    cfg["maker"] = good
    taker.write_config(cfg)
    return c.save(maker, taker)


def s16_short_reveal_window():
    return quote_rejected("s16_short_reveal_window", "short-reveal",
        "Malicious maker quotes a reveal window W <= F + B_R (a lock could never be revealed safely): taker rejects the quote (R1q) and never funds", "reveal window")


def s17_far_horizon():
    return quote_rejected("s17_far_horizon", "far-horizon",
        "Malicious maker quotes a punish deadline t2 about 136 years away (hostage by delay): taker rejects the quote (R1q) and never funds", "deadline horizon")


def s18_late_funding():
    scen = "s18_late_funding"
    maker, taker = parties(scen)
    c = Checks(scen, "Taker's funding stays unmined until after the maker's no-funding refund (b public): the taker keeps a (R4) and sweeps the late note with a + b")
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT))
    maker.wait_log("STIPEND", 200)
    mine_lock.acquire()                  # hold Zcash blocks: the funding will sit in the mempool
    try:
        while zrpc("getrawmempool"):     # first mine what is already pending (the acceptance)
            zrpc("generate", [1])
        zrpc("generate", [2])
        taker.wait_log("FUNDED", 200)
        held = len(zrpc("getrawmempool"))
        m = maker.swap()
        warp_to(m["t0"] - TIMING["fund_secs"] // 2)
        maker.wait(150)
        taker.wait_log("keeping a", 120)
        mid = taker.swap()
    finally:
        mine_lock.release()
    taker.wait(300)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok("funding was unmined when the maker refunded for lack of funding", held > 0 and any("no funding before t0" in l for l in m["log"]), held)
    c.ok("maker reached Refunded", stage_name(m) == "Refunded", stage_name(m))
    c.ok("taker kept a while its funding was not visible", stage_name(mid) == "Funded" and mid["a"] != "", stage_name(mid))
    c.ok("taker swept the late joint note back with (a+b)", stage_name(t) == "Recovered" and any("swept the joint note back" in l for l in t["log"]), stage_name(t))
    c.ok("maker tokens fully restored", after["maker_wnear"] == before["maker_wnear"])
    c.ok("taker ZEC: premium + 4 tx fees lost (exact)", before["taker_zec"] - after["taker_zec"] == PREMIUM + 4 * FEE, before["taker_zec"] - after["taker_zec"])
    return c.save(maker, taker)


def escrow_rejected(scen, evil, title, needle, warp=True, taker_extra=None):
    """The maker opens an escrow the taker must not fund; the taker aborts and the maker
    refunds once the funding window is over."""
    maker, taker = parties(scen, maker_extra={"misbehave": evil} if evil else None, taker_extra=taker_extra)
    c = Checks(scen, title)
    before = snapshot(maker, taker)
    maker.start("--exit-after", "1")
    taker.start("--zats", str(LOT))
    maker.wait_log("escrow OPEN", 300)
    m = maker.swap()
    if warp:
        taker.wait_log("waiting for the stipend", 120)
        warp_to(m["t0"] - TIMING["fund_secs"] // 2)
    taker.wait(300)
    if not warp:
        warp_to(m["t0"] - TIMING["fund_secs"] // 2)
    maker.wait(300)
    t, m = taker.swap(), maker.swap()
    after = snapshot(maker, taker)
    c.facts.update(before=before, after=after, escrow_id=m["escrow_id"])
    c.ok(f"taker refused to fund ({needle})", stage_name(t) == "Aborted" and needle in abort_reason(t) and t.get("fund_txid") is None, t["stage"])
    c.ok("maker refunded early (no funding)", stage_name(m) == "Refunded", stage_name(m))
    c.ok("maker tokens restored", after["maker_wnear"] == before["maker_wnear"], after["maker_wnear"] - before["maker_wnear"])
    c.ok("taker lost only the premium and 2 tx fees", before["taker_zec"] - after["taker_zec"] == PREMIUM + 2 * FEE, before["taker_zec"] - after["taker_zec"])
    return c.save(maker, taker)


def s19_no_stipend():
    return escrow_rejected("s19_no_stipend", "no-stipend",
        "The maker opens the escrow but sends no stipend: the taker does not fund (its key could not pay for commit_claim and claim) and aborts when R1b closes; the maker refunds",
        "R1b")


def s20_short_open():
    return escrow_rejected("s20_short_open", "short-open",
        "The maker opens the escrow with less than the quoted price: the taker's R1 check refuses to fund; the maker refunds",
        "amount does not match", warp=False)


def s21_unexpected_escrow_code():
    return escrow_rejected("s21_unexpected_escrow_code", None,
        "The escrow account does not run the code the taker expects (taker configured with another code hash): the taker refuses to fund; the maker refunds. The same check refuses an escrow with access keys or another Intents account (crates/zecret-node/tests/sandbox.rs)",
        "escrow contract check failed", warp=False, taker_extra={"escrow_code_hash": "11111111111111111111111111111111"})


SCENARIOS = {"s1": s1_happy, "s2": s2_taker_vanishes_before_funding, "s3": s3_taker_vanishes_after_funding,
             "s4": s4_maker_vanishes_after_open, "s5": s5_rogue_key, "s6": s6_underfund,
             "s8": s8_free_claim_attempt, "s9": s9_late_commit, "s10": s10_lock_without_reveal,
             "s11": s11_maker_liveness_violated, "s12": s12_cover_traffic, "s15": s15_impostor_directory,
             "s16": s16_short_reveal_window, "s17": s17_far_horizon, "s18": s18_late_funding,
             "s19": s19_no_stipend, "s20": s20_short_open, "s21": s21_unexpected_escrow_code}


def write_report():
    rows, detail = [], []
    names = [n for n in os.listdir(EVID) if os.path.exists(os.path.join(EVID, n, "result.json"))]
    key = lambda n: int(re.match(r"s(\d+)", n).group(1)) if re.match(r"s(\d+)", n) else 999
    for name in sorted(names, key=key):
        r = json.load(open(os.path.join(EVID, name, "result.json")))
        ok = all(c["pass"] for c in r["checks"])
        m, t = r.get("maker_swap") or {}, r.get("taker_swap") or {}
        rows.append(f"| `{name}` | {r['title']} | `{r['chain_branch']}` | {stage_name(m)} | {stage_name(t)} | {'PASS' if ok else 'FAIL'} |")
        detail.append(f"\n### {name}\n\n{r['title']}\n")
        detail += [f"- {'PASS' if c['pass'] else 'FAIL'}: {c['check']} {('(' + c['detail'] + ')') if c['detail'] and c['detail'] not in ('[]', 'True') else ''}" for c in r["checks"]]
        for who, sw in (("taker", t), ("maker", m)):
            if sw:
                detail.append(f"\n{who} log:\n")
                detail += [f"    {l}" for l in sw.get("log", [])]
        for tx in r["facts"].get("zcash_txs", []):
            detail.append(f"- Zcash tx `{tx['txid']}` height {tx['height']}, v{tx['version']}, {tx['ironwood_actions']} Ironwood actions, value balance {tx['ironwood_value_balance']}, transparent in/out {tx['transparent_in']}/{tx['transparent_out']}")
    head = ["# Zecret local verification results", "",
            f"Generated by `harness/run.py` on {time.strftime('%Y-%m-%d %H:%M:%S %Z')}. Zebra 7.0.0-rc.0 regtest (branch `77190ad9` = NU7, `37a5165b` = NU6.3) and a NEAR 2.14.1 sandbox with the escrow (`contracts/near-escrow`), the live `wrap.near` code (token) and the live `intents.near` code. Maker and taker are separate processes that communicate only through shielded memos and the escrow.", "",
            "| Scenario | What it tests | Zcash branch | Maker end | Taker end | Result |", "|---|---|---|---|---|---|"]
    open(os.path.join(HERE, "RESULTS.md"), "w").write("\n".join(head + rows + ["", "## Details"] + detail) + "\n")


def run_one(name, fn, results):
    log(f"=== {name} ===")
    t = time.time()
    try:
        results[name] = (fn(), round(time.time() - t))
    except Exception as e:
        log(f"  ERROR {e}")
        results[name] = (False, round(time.time() - t))


def main():
    sel = sys.argv[1:] or list(SCENARIOS) + ["s7"]
    os.makedirs(EVID, exist_ok=True)
    os.makedirs(WORK, exist_ok=True)
    build()
    results = {}
    try:
        main_sel = [s for s in sel if s in SCENARIOS]
        if main_sel:
            start_chains(nu7=True)
            for s in main_sel:
                run_one(s, SCENARIOS[s], results)
            stop_chains()
        if "s7" in sel:
            start_chains(nu7=False)
            run_one("s7", s7_happy_nu63, results)
    finally:
        stop_chains()
        stop_near()
    log("SUMMARY: " + ", ".join(f"{k} {'PASS' if v[0] else 'FAIL'} ({v[1]}s)" for k, v in results.items()))
    with open(os.path.join(EVID, "summary.json"), "w") as f:
        json.dump({k: {"pass": v[0], "seconds": v[1]} for k, v in results.items()}, f, indent=2)
    write_report()
    sys.exit(0 if results and all(v[0] for v in results.values()) else 1)


if __name__ == "__main__":
    main()
