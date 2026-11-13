# Running the maker as a service

The maker daemon is `zecret maker --dir D` from `crates/zecret-node` (binary `zecret`).
It runs until stopped. It keeps its wallet key, its NEAR key, its swap records and its
settings in the folder `D`:

| File | What it is |
|---|---|
| `config.json` | `MakerConfig` (`crates/zecret-core/src/config.rs`, `docs/ENGINE.md`) |
| `near_key.json` | the maker account's NEAR key as one string `ed25519:...` (not JSON) |
| other `*.json` | wallet key, RFQ key, swap records (written by the daemon) |

The folder holds private keys. Keep it on an encrypted volume, mode 0700, and back it
up: a lost folder in the middle of a swap means the maker cannot take the ZEC or refund.

## 1. Build the image

From the repository root:

```sh
docker build -f deploy/maker/Dockerfile -t zecret-maker .
```

`Dockerfile.dockerignore` keeps everything except `crates/` (and its `target/` folders)
out of the build context. Build arguments: `RUST_VERSION` (default 1.97),
`DEBIAN_RELEASE` (bookworm), `CRATES_DIR` (crates), `PACKAGE` (zecret-node), `BIN`
(zecret).

Behind a proxy that re-signs TLS, give cargo the proxy's CA bundle as a build secret:

```sh
docker build --network host --secret id=cacert,src=/path/to/ca.crt \
  --build-arg HTTPS_PROXY=$HTTPS_PROXY -f deploy/maker/Dockerfile -t zecret-maker .
```

The image runs as user `zecret` (uid 10001) with `tini` as PID 1, entrypoint `zecret`,
default command `maker --dir /data`.

## 2. Prepare the maker folder

```sh
D=/srv/zecret-maker
# The NEAR key of the maker account that zecret-deploy created:
deploy/target/debug/zecret-deploy maker-key --config deploy/configs/testnet.toml --out $D
```

`maker-key` writes `$D/near_key.json` (mode 600) and prints the values for the `chains`
section and `near_account`. Write `$D/config.json`. Example for testnet (the timing and
window values are examples; choose them from `docs/DESIGN.md` §7 and the measurements in
`docs/DEPLOY.md`):

```json
{
  "chains": {
    "network": "test",
    "lightwalletd": "https://zcash-testnet.chainsafe.dev",
    "birthday": 0,
    "near_rpc": "https://rpc.testnet.fastnear.com",
    "escrow": "escrow2.zecret-3f109eea1e.testnet",
    "token": "usdc.zecret-3f109eea1e.testnet",
    "intents": "intents.zecret-3f109eea1e.testnet"
  },
  "timing": { "inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2,
              "margin_secs": 0, "fund_secs": 1800, "k_z": 3 },
  "near_account": "maker.zecret-3f109eea1e.testnet",
  "price_per_zec": "940000000",
  "min_zats": 1000000,
  "max_zats": 200000000,
  "premium_zats": 450000,
  "premium_check": {
    "yocto_near_per_zec": "10000000000000000000000000",
    "costs_per_request_yocto": "15000000000000000000000"
  },
  "stipend": "30000000000000000000000",
  "windows": { "t0_secs": 3600, "t1_secs": 1800, "t2_secs": 1800, "reveal_secs": 600 }
}
```

The escrow is `escrow2.zecret-3f109eea1e.testnet`, the build with the NV-02 fix.
`escrow.zecret-3f109eea1e.testnet` is retired (`docs/DEPLOY.md`); a taker refuses it,
because its code hash is not the expected one.

### The premium check

Each accepted request costs the maker the stipend (0.03 NEAR) and some gas and storage.
The premium is the taker's payment for that. `premium_check` makes the maker refuse to
start when the premium is worth less than these costs (security review 2, NV2-02;
`MakerConfig::check_premium` in `crates/zecret-core/src/config.rs`). The rule:

```text
premium_zats × yocto_near_per_zec / 10^8  ≥  stipend + costs_per_request_yocto
```

- `yocto_near_per_zec` is the price of 1 ZEC in yoctoNEAR. You set it. Use a low
  estimate of the real price. The value in the example, 10 NEAR per ZEC, is only an
  example, not a market price.
- `costs_per_request_yocto` is the gas and escrow storage of one accepted request,
  besides the stipend. The default is 0.015 NEAR. Measure it on your network.
- With the example values: 450,000 zats × 10 NEAR / 10^8 = 0.045 NEAR, and the costs are
  0.03 + 0.015 = 0.045 NEAR. So 450,000 zats is the smallest premium that passes;
  449,999 zats is refused (test `premium_below_cost_is_refused`).
- An earlier version of this example had `premium_zats` 20,000. That passes only if
  1 ZEC is worth at least 225 NEAR (20,000 × 225 NEAR / 10^8 = 0.045 NEAR).
- On mainnet (`chains.network` is `main`) the check is required: without
  `premium_check` the maker does not start. On testnet it runs only when it is set.
- When the check fails, the error message names the smallest premium that passes.

Takers refuse a quote whose premium is above their own `max_premium`
(`crates/zecret-node/config/testnet-taker.json` has 2,000 zats). A maker with a higher
premium needs takers configured for it.

Set `birthday` to a recent Zcash testnet height before the maker's wallet is created.
Then create the wallet and read its address:

```sh
docker run --rm -v $D:/data zecret-maker init --dir /data
docker run --rm -v $D:/data zecret-maker directory --dir /data   # the signed directory entry
```

The folder must be writable by uid 10001: `chown -R 10001 $D; chmod 700 $D`.

## 3. Run

```sh
docker run -d --name zecret-maker --restart unless-stopped -v $D:/data zecret-maker
docker logs -f zecret-maker
```

Run one maker per folder. Two processes on the same folder would both act on the same
swaps.

Or with systemd and no container:

```ini
# /etc/systemd/system/zecret-maker.service
[Unit]
Description=Zecret maker
After=network-online.target
Wants=network-online.target

[Service]
User=zecret
ExecStart=/usr/local/bin/zecret maker --dir /srv/zecret-maker
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/srv/zecret-maker
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

## What to watch

- The maker's NEAR balance: each swap costs about 0.04 NEAR (0.03 stipend, gas). Keep
  about 0.2 NEAR free besides that: NEAR checks that the signer holds the prepaid gas of
  a 200 Tgas `withdraw` at a pessimistic price (`docs/ENGINE.md`, section 4).
- Its escrow storage funds (`get_storage_funds`): the daemon tops them up from its NEAR
  (`storage_min`, `storage_topup` in `config.json`).
- Its token balance and its shielded ZEC balance (`zecret balance --dir D`).
- Log lines with `transient:` that repeat, and swaps that stay in one stage.

## Checked and not checked

Checked on 2026-11-13 on the build machine, with the `crates/` code of that time:

- The image builds (`docker build`, Rust 1.97, Debian bookworm; about 143 MB).
- `init` in the container creates a testnet wallet and prints a `utest1...` address.
- `directory` in the container signs a directory entry with the key that
  `zecret-deploy maker-key` exported. So the key file format matches what the daemon
  reads.
- `maker` with an empty folder stops with an error (no `config.json`), as expected.

At that time **the `zecret` binary could not reach `https://` endpoints**: reqwest had
no TLS backend, and in the container `zecret balance` against
`https://zcash-testnet.chainsafe.dev` failed. This was fixed later in `crates/zecret-node`:
its `Cargo.toml` now enables reqwest's `rustls-tls` and `rustls-tls-native-roots`
features. The native binary (not the container) then ran a full swap on Zcash testnet
and NEAR testnet over HTTPS (`docs/TESTNET.md`).

Not checked: this image rebuilt with the TLS fix; the systemd unit. A separate image,
`deploy/maker/railway/Dockerfile`, runs the hosted testnet maker on Railway
(`docs/HOSTING.md`).
