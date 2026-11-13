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
    "escrow": "escrow.zecret-3f109eea1e.testnet",
    "token": "usdc.zecret-3f109eea1e.testnet",
    "intents": "intents.zecret-3f109eea1e.testnet"
  },
  "timing": { "inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2,
              "margin_secs": 0, "fund_secs": 1800, "k_z": 3 },
  "near_account": "maker.zecret-3f109eea1e.testnet",
  "price_per_zec": "940000000",
  "min_zats": 1000000,
  "max_zats": 200000000,
  "premium_zats": 20000,
  "stipend": "30000000000000000000000",
  "windows": { "t0_secs": 3600, "t1_secs": 1800, "t2_secs": 1800, "reveal_secs": 600 }
}
```

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

- The maker's NEAR balance: each swap costs about 0.04 NEAR (0.03 stipend, gas).
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

Not working yet, and not fixable from this folder: **the `zecret` binary cannot reach
`https://` endpoints.** `crates/zecret-node/Cargo.toml` turns reqwest's default features
off and enables only `blocking`, so reqwest has no TLS backend. Every public endpoint
the maker needs is HTTPS (the NEAR RPC, the light-wallet server). In the container,
`zecret balance` against `https://zcash-testnet.chainsafe.dev` failed: reqwest sent the
`https://` URL as a plain-HTTP proxy request instead of opening a TLS tunnel. The fix
belongs in `crates/zecret-node`: add a TLS feature to reqwest (for example
`rustls-tls`) and rebuild the image. The daemon has therefore not run against testnet.

Not checked: the systemd unit; a maker run with real swaps on testnet.
