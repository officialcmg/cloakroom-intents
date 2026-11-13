# Working in this repository

Zecret swaps shielded ZEC for tokens on other chains with no custodian. Read
`docs/DESIGN.md` first; its opening section, "The design in plain words", is always
current. `docs/BUILD_PLAN.md` says how the build runs and `STATUS.md` says where it is.

## Rules

- **Baseline.** Commit `2f8e2f7` holds the code imported from `officialcmg/cloakroom`
  (see `BASELINE.md`). It is tagged `pre-hackathon` in the build clone and pushed as the
  branch `pre-hackathon-baseline` (the build environment's git proxy refuses to push
  tags). Never move or delete that tag or that branch.
- **Commits.** Author is already configured for this clone; do not change it. No
  co-author trailers and no mention of AI tools in commit messages, code comments,
  READMEs or any other file.
- **Quality bar.** Working code with tests, not placeholders. No `todo!()`, no stubbed
  functions, no skipped or disabled tests to get a green run. If something cannot be
  done, write what, why and what was tried in `STATUS.md`.
- **A report is not proof.** A package is done when its check in `docs/BUILD_PLAN.md`
  passes and the output is shown.
- **Design changes.** Update "The design in plain words" and its decisions log in the
  same commit, in Simplified Technical English: short sentences, one idea each, active
  voice, the same word for the same thing.
- **Plain writing.** Text a person reads (README, site copy) states facts and steps.
  No marketing language.
- **Secrets.** Keys go in environment variables or untracked files, never in git.
- **Never without the owner:** deploy to mainnet, spend real funds, post on social
  networks, submit the hackathon form.

## Environment

```sh
./scripts/setup-env.sh && source .tools/env.sh
```

- NEAR sandbox tests: `LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test ...` (the sandbox
  must be nearcore 2.14.1 or newer; `NEAR_SANDBOX_BIN_PATH` points at it).
- Live `intents.near` and `wrap.near` contract code is in `$ZECRET_WASM_DIR`.
- Contracts build with `cargo build --target wasm32-unknown-unknown --release`, and
  need `--cfg near` in `RUSTFLAGS` for that target (see
  `contracts/near-escrow/.cargo/config.toml`). Without it, contract panics lose their
  messages.
- Local Zcash node: `harness/zebra/` (Docker, Zebra 7.0.0-rc.0 regtest).
- WebAssembly: build `orchard` and `zcash_primitives` without their `multicore`
  feature. Do not test with `node:wasi`; it crashes. Use a browser (Playwright and
  Chromium are installed) or a hand-written shim.
- Measured facts and how they were measured are in `docs/experiments/`.
