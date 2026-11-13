# Status

Updated: 2026-11-13 03:00 UTC. Deadline: 2026-11-14 23:59 UTC.

Any session can resume from this file. Read `docs/BUILD_PLAN.md` for the packages and
their checks.

## Done

| Package | Result |
|---|---|
| A. Escrow contract | PASS, rerun by the main session: 48 unit tests, 5 sandbox tests against the live `intents.near` and `wrap.near` code, clippy and fmt clean. `claim` 16.8 Tgas, taker withdraw into Intents 42.4 Tgas. Findings: the stipend must be 0.03 NEAR; contracts need `--cfg near`; the maker prepays swap storage. See `contracts/near-escrow/README.md`. |
| B. Browser spike | PASS, rerun by the main session. In headless Chromium the wallet in a Web Worker receives a shielded payment, reads its memo, and builds, proves and broadcasts a fully shielded transaction with a 512-byte memo on Zebra regtest. Key build about 20 s, one send about 27 s. Run: `spike/browser-wallet/run.mjs`. Findings and the recommended engine structure: `docs/experiments/browser-wallet.md`. |
| Wave 0: repository setup | Cloakroom imported and tagged `pre-hackathon`. `scripts/setup-env.sh` prepares the sandbox, the Intents contract code, the open-file shim and Docker. |

## Running

| Package | Started (UTC) | Where the work is |
|---|---|---|
| CD. Shared core crate and NEAR adapter | 02:50 | `crates/`, `node/` (uncommitted until reviewed) |
| F. Website | 02:50 | `web/` (uncommitted until reviewed) |

## Next

E. Taker engine in WebAssembly, on top of the shared core (after CD). Then G, H, I, J, K, L.

Plan change: packages C and D are merged into one (CD), because the browser spike showed the right structure is one shared core crate with no I/O. The website (F) starts now against a documented engine interface with a simulated engine, so UX work runs in parallel.

## Blocked or skipped

Nothing.

## Notes for whoever resumes

- Usage window resets at 03:10 UTC on 2026-11-13, then every 5 hours.
- An hourly scheduled task wakes the main session (trigger `trig_01Bbv8VUrvwpGeEr3NWMoRAU`).
- Main session: `session_01UpiMzVjgad99bzMjoPrgGL`.
- Agents edit separate folders and do not commit. The main session reviews, runs the
  check and commits.
- Because this machine is temporary, the main session also pushes snapshots of
  work in progress. Their commit messages start with "WIP". A package counts as done
  only when it is listed under "Done" above.
