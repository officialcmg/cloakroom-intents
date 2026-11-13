#!/usr/bin/env python3
"""Chains for the browser scenarios (harness/browser/run.mjs), on their own ports.

Reuses harness/run.py: builds the escrow and the `zecret` binary, starts a Zebra regtest
node (NU7) in its own Docker container, a NEAR 2.14.1 sandbox with the escrow, the live
wrap.near and intents.near code (`zecret devnet`) and the block miner. Prints one JSON
line with the endpoints and account ids, then keeps the chains up until stdin closes.

Ports (distinct from harness/run.py, so both can run at once):
  Zebra JSON-RPC 127.0.0.1:18332 (container zecret-browser-zebra), NEAR sandbox 127.0.0.1:3130.

  source .tools/env.sh
  python3 -I harness/browser_chains.py         # normally started by harness/browser/run.mjs
"""
import json
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run as R  # noqa: E402

ZEBRA_PORT = int(os.environ.get("ZECRET_BROWSER_ZEBRA_PORT", "18332"))
NEAR_PORT = int(os.environ.get("ZECRET_BROWSER_NEAR_PORT", "3130"))
CONTAINER = "zecret-browser-zebra"

R.ZEBRA = f"http://127.0.0.1:{ZEBRA_PORT}"
R.NEAR_PORT = NEAR_PORT
R.NEAR = f"http://127.0.0.1:{NEAR_PORT}"
R.WORK = os.path.join(HERE, "work", "browser")
R.EVID = os.path.join(HERE, "evidence", "browser")


def log(msg):
    print(f"[chains {time.strftime('%H:%M:%S')}] {msg}", file=sys.stderr, flush=True)


def start_zebra():
    subprocess.run(["docker", "rm", "-f", CONTAINER], capture_output=True)
    conf = os.path.join(HERE, "zebra", "zebrad-nu7.toml")
    subprocess.run(["docker", "run", "-d", "--name", CONTAINER, "-e", "CONFIG_FILE_PATH=/etc/zebrad/zebrad.toml",
                    "-v", f"{conf}:/etc/zebrad/zebrad.toml:ro", "-p", f"127.0.0.1:{ZEBRA_PORT}:18232",
                    "zfnd/zebra:7.0.0-rc.0"], check=True, capture_output=True)
    for _ in range(90):
        try:
            R.zrpc("getblockchaininfo")
            break
        except Exception:
            time.sleep(1)
    R.zrpc("generate", [3])
    info = R.zrpc("getblockchaininfo")
    R.state["nu7"] = 2
    R.state["branch"] = info["consensus"]["nextblock"]
    log(f"zebra up on {R.ZEBRA}: tip {info['blocks']} branch {R.state['branch']}")


def main():
    os.makedirs(R.WORK, exist_ok=True)
    if not os.environ.get("ZECRET_SKIP_BUILD"):
        R.build()
    start_zebra()
    R.start_near()
    R.stop_miner.clear()
    threading.Thread(target=R.miner, daemon=True).start()
    out = {"zebra": R.ZEBRA, "near": R.NEAR, "nu7_height": 2, "branch": R.state["branch"],
           "root_key": os.path.join(R.WORK, "near", "validator_key.json"), "maker_key": R.state["maker_key"],
           "bin": R.BIN, **R.state["net"]}
    print(json.dumps(out), flush=True)
    try:
        # Commands on stdin, one per line; each answers one JSON line on stdout.
        #   fund <ua>   mine one block paying its coinbase to <ua>, under the miner's lock
        #               (a concurrent `generate` would make one of the two blocks stale)
        for line in sys.stdin:
            parts = line.split()
            if len(parts) == 2 and parts[0] == "fund":
                try:
                    with R.mine_lock:
                        h = R.zrpc("generatetoaddress", [1, parts[1]])
                    print(json.dumps({"ok": True, "blocks": h}), flush=True)
                except Exception as e:
                    print(json.dumps({"ok": False, "error": str(e)}), flush=True)
            else:
                print(json.dumps({"ok": False, "error": f"unknown command {line.strip()!r}"}), flush=True)
    finally:
        R.stop_miner.set()
        R.stop_near()
        subprocess.run(["docker", "rm", "-f", CONTAINER], capture_output=True)
        log("chains stopped")


if __name__ == "__main__":
    main()
