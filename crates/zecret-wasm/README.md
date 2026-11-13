# zecret-wasm

The taker engine for the browser. It puts `zecret-core`'s taker and the exit through
NEAR Intents (`zecret-intents`) behind wasm-bindgen, for the Web Worker in
`web/src/engine/real/`. That worker implements the website's `Engine` interface
(`web/ENGINE_API.md`) on top of it.

| File | What |
|---|---|
| `src/session.rs` | The taker over a `MemStore`: open from stored records, `start`, `tick`, `reconcile`, chain facts (balance, scan height, funding height), escrow state, Verifier and token balances, `return_leftover`. The worker writes the changed records (`dirty`) to IndexedDB and waits for the commit before it calls again. |
| `src/exit.rs` | The exit. 1Click (mainnet): firm quote and check, then generate, check, sign and submit, then status. Direct (local and testnet): plan one `ft_withdraw` to a NEAR account, sign it and hand it to the maker's relay. Errors carry the website's codes. |
| `src/transport.rs` | The core's POST transport as `zecret-intents`' `HttpTransport`. |
| `src/web.rs` | wasm32 only: the wasm-bindgen interface and the synchronous `XMLHttpRequest` transport (POST and GET). |
| `examples/sandbox_account.rs` | Test helper: a NEAR sandbox account registered with a token. |

## Build

```sh
source .tools/env.sh
crates/zecret-wasm/build.sh      # release wasm + wasm-bindgen glue into web/public/engine/
```

`orchard` and `zcash_primitives` build without `multicore`; the getrandom browser
backends come from `zecret-core` and from the `getrandom` 0.3 entry here, with the cfg in
`crates/.cargo/config.toml`. The `.wasm` is about 9 MB (no `wasm-opt`).

## Tests

```sh
cd crates
cargo test -p zecret-wasm --lib                         # error codes, account ids, stored plan
cargo test -p zecret-wasm --test oneclick_recorded      # 1Click steps against recorded live answers
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test -p zecret-wasm --test sandbox -- --nocapture --test-threads=1
ZECRET_GAS_SWEEP=1 LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test -p zecret-wasm --test sandbox gas_sweep -- --nocapture
```

The sandbox tests use the live `intents.near` and `wrap.near` code. They show that the
maker's relay executes the direct exit (the receiver gets the whole balance; a second
submission sends nothing; a key that is not the maker's taker is refused), that what is
left of the stipend after commit and claim (0.0281 NEAR) cannot send `execute_intents`
with the 40 Tgas an `ft_withdraw` needs (`NotEnoughBalance`; 30 Tgas alone was checked at 0.0309 NEAR),
and that an account that does not exist or is not registered with the token is refused
before anything is signed. `gas_sweep` measured: 30 and 35 Tgas "Exceeded the prepaid
gas", 40 and 50 Tgas deliver, about 0.001 NEAR burnt.

The whole engine runs in a browser in `harness/browser/run.mjs`.
