# Build plan

How the long build runs once the owner approves the design. Deadline: 2026-11-14 23:59 UTC.

## What "done" means

A public repository that a judge can use without installing anything:

1. A website where a user swaps shielded ZEC for a token on another chain.
2. A maker running in our backend.
3. An escrow contract on NEAR.
4. A README with "do X, see Y" steps, a baseline disclosure and a link to the diff.
5. Automatic tests that run one full swap locally.

## Work packages

Packages in the same wave run in parallel. A wave starts when the wave before it passes
its check.

| Wave | Package | Output | Check that must pass |
|---|---|---|---|
| 0 | Repository setup | Cloakroom imported and tagged `pre-hackathon`; setup script for Docker, sandbox 2.14.1 and tools; `STATUS.md` | Setup script runs on a clean machine; Cloakroom client builds |
| 1 | A. Escrow contract | Rust contract with every transition of the state diagram, the stipend and the Intents payout | Unit tests for every transition and every wrong-time or wrong-caller case; sandbox test with the live `intents.near` code |
| 1 | B. Browser spike | The Zcash wallet code in a browser worker: scan blocks, decrypt a memo, build and send a shielded transaction | A page receives a payment and sends a memo on local regtest |
| 1 | C. Shared-code cleanup | No `cast`, no blocking HTTP, no sleep loops in shared code; a small network interface with a browser and a server implementation | Cloakroom's existing local scenarios still pass on the maker side |
| 2 | D. NEAR adapter | `near.rs` with the functions `evm.rs` had; ed25519 taker key; stipend; taker sends commit and claim | Maker and taker complete one swap as two programs: Zebra regtest plus NEAR sandbox |
| 2 | E. Taker engine in WebAssembly | The taker state machine in a worker, with storage and a recovery file | The same swap completes with the taker in headless Chromium |
| 3 | F. Website | Amount, token and chain; one price; QR code; progress; refund and recovery screens | Browser test: a full swap driven through the page |
| 3 | G. Exit through Intents | 1Click quote, combined price, signed exit after the claim | Sandbox: USDC leaves Intents by signed message. Live: quotes for 5 destination chains |
| 3 | H. Failure paths | Maker refund, taker recovery of ZEC, punish path, page reload in the middle of a swap | One scenario test for each |
| 4 | I. Testnet deployment | Escrow on NEAR testnet, maker in the backend, website hosted | A swap on Zcash testnet and NEAR testnet through the hosted site |
| 4 | J. Timing | Measured `D` and `F`, final window parameters | Parameters satisfy the paper's inequalities |
| 5 | K. Documentation | README, baseline disclosure, diff link, demo script, draft of the demo post | A fresh agent follows the README and gets the same results |
| 5 | L. Final wording review | All text a person reads outside the code: README, website copy, demo post | See "Final wording review" below |

If package B fails, the fallback is: the taker ships as a program, and the website shows
it running in a hosted sandbox. Packages A, C, D, G, H and J do not change.

## How agents are used

- Opus 5.5 subagents do the packages. At most 3 run at the same time, to spread usage.
- Each agent gets one package, the design section it needs and the check it must pass.
  It works in its own git worktree and returns a branch.
- The main session reviews each branch, runs the check itself and merges. An agent's
  report is not proof; the check is.
- A failed check goes back to an agent with the exact failure output.
- Nothing is skipped for convenience. If something cannot be done, `STATUS.md` says
  what, why and what was tried.

## Usage and wake-ups

The main session cannot read the account's usage meter. It manages usage in these ways:

- **A status file.** `STATUS.md` in the project repository is updated and pushed after
  every merged package: what is done, what is running, what is next. Any session can
  resume from it.
- **An hourly scheduled task into this session.** Its message is: "Read `STATUS.md`. If
  the build is not finished, continue it." If the session stopped because usage ran
  out, the first firing after the reset restarts the work with nobody awake.
- **A cap on parallel agents** (3), and small packages, so a stop loses little work.
- **Commit and push often**, because the machine is temporary.

The scheduled task is removed when the build is finished.

### When this machine runs out

Owner's instruction (2026-11-13): if the message "This session is out of disk, memory
or time. Start a new session to get a fresh machine." appears, start a new cloud
session to continue the task. Do not stop and wait.

1. Push everything first, including `STATUS.md`.
2. Create a new cloud session on the project repository. Its prompt says: read
   `STATUS.md`, `DESIGN.md` and `BUILD_PLAN.md`, run the setup script, continue the
   build, and follow this same rule.
3. Move the hourly scheduled task to the new session, so only one session builds.
4. Write the new session's ID in `STATUS.md`.

To hand over before the machine fails, the main session checks free disk space after
every merged package and starts the hand-over when less than 5 GB is free. It deletes
build output it no longer needs first.

## Final wording review

The last step. It covers the README, the website text and the demo post. It does not
touch code.

1. An agent collects advice on plain, human-sounding writing from well-used public
   writing skills and style guides, and writes a short checklist with its sources.
2. A second agent edits every external text against that checklist.
3. The main session checks that no fact changed in the edit.

## What needs the owner

| Item | Why |
|---|---|
| An empty repository, attached to this session | The session cannot create repositories. |
| A project name | A placeholder works until the owner picks one. |
| A funded NEAR mainnet account and approval to spend | The real Intents exit only runs on mainnet. |
| Approval to deploy the maker and the website | Hosting is public and outward-facing. |
| The demo post on X and the submission form | Only the owner may post or submit. |
