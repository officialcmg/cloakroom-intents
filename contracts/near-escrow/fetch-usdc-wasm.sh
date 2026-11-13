#!/usr/bin/env bash
# Downloads the code of Circle's USDC on NEAR mainnet for the sandbox tests in
# tests/usdc.rs, and checks it against the code hash that the escrow's analysis
# (docs/security/nv02-usdc.md, section 3.2) recorded.
#
#   ./fetch-usdc-wasm.sh [OUT_DIR]
#
# OUT_DIR defaults to $ZECRET_WASM_DIR (../../.tools/wasm if unset). That folder is not
# in git. The file is written as OUT_DIR/usdc.near.wasm only if its hash matches.
set -euo pipefail

USDC=17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1
EXPECTED=DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q
RPC=${NEAR_MAINNET_RPC:-https://free.rpc.fastnear.com}
here=$(cd "$(dirname "$0")" && pwd)
out_dir=${1:-${ZECRET_WASM_DIR:-$here/../../.tools/wasm}}
mkdir -p "$out_dir"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -sS --fail "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,
  "method":"query","params":{"request_type":"view_code","finality":"final",
  "account_id":"'"$USDC"'"}}' -o "$tmp/resp.json"

python3 -I - "$tmp/resp.json" "$tmp/usdc.wasm" "$EXPECTED" <<'EOF'
import base64, hashlib, json, sys
resp, out, expected = sys.argv[1], sys.argv[2], sys.argv[3]
r = json.load(open(resp))
if "result" not in r:
    sys.exit(f"RPC error: {r.get('error')}")
code = base64.b64decode(r["result"]["code_base64"])
alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
n = int.from_bytes(hashlib.sha256(code).digest(), "big")
h = ""
while n:
    n, rem = divmod(n, 58)
    h = alphabet[rem] + h
print(f"block {r['result']['block_height']}: {len(code)} bytes, code hash {h}")
if h != expected:
    sys.exit(f"code hash {h} is not the expected {expected}; not written")
open(out, "wb").write(code)
EOF
mv "$tmp/usdc.wasm" "$out_dir/usdc.near.wasm"
echo "wrote $out_dir/usdc.near.wasm"
