# Documents

What each document in `docs/` is for and who should read it. Start with the repository's
[`README.md`](../README.md): what Zecret is, its status, and how to try it.

## The product and the build

| Document | What it is | Read it if you are |
|---|---|---|
| [`DESIGN.md`](DESIGN.md) | The design. Its first section, "The design in plain words", is always current and has the log of decisions. The numbered sections give the detail: system map, one swap, escrow, gas trail, browser taker, timing, networks, tests, risks. | anyone; read this first |
| [`BUILD_PLAN.md`](BUILD_PLAN.md) | How the hackathon build was organized: work packages, the check each must pass, what needs the owner. | a reviewer of the process |
| [`ENGINE.md`](ENGINE.md) | The interface of the shared swap code (`crates/zecret-core`): network, storage and time, the taker's configuration, stages and events, the maker's rules. | an engineer who hosts the engine or reads the code |
| [`WEB.md`](WEB.md) | The website: screens, states, decisions, accessibility, tests, security headers. | a reviewer of the website |
| [`INTENTS.md`](INTENTS.md) | The exit through NEAR Intents: the 1Click flow, the check before signing, what needs an API key, measured quote behaviour, errors, what is not verified. | a reviewer of the NEAR Intents integration |
| [`DEPLOY.md`](DEPLOY.md) | The NEAR deployment: what is deployed on testnet, steps and costs for testnet and mainnet, how to check a deployment, measured transaction times. | an operator, or a judge checking the testnet contracts |
| [`TESTNET.md`](TESTNET.md) | The swap on Zcash testnet and NEAR testnet: what is real and what is a stand-in, commands, every transaction, timings, balances, problems met. | a judge checking the testnet evidence |

## Submission

| Document | What it is | Read it if you are |
|---|---|---|
| [`SUBMISSION.md`](SUBMISSION.md) | The hackathon submission fields, proposed values, the payload shape and how to send it once. | the owner |
| [`DEMO_SCRIPT.md`](DEMO_SCRIPT.md) | A script for the demo video and two drafts of the post on X. | the owner |

## Security

| Document | What it is |
|---|---|
| [`security/review-1.md`](security/review-1.md) | Review 1: the escrow contract and the Intents library. Findings F-01, F-02. |
| [`security/fixes-1.md`](security/fixes-1.md) | Fixes for review 1, with regression tests. |
| [`security/nv02-usdc.md`](security/nv02-usdc.md) | The escrow's payout with Circle's USDC code: the double-payment risk, reproduced in a sandbox. |
| [`security/fixes-2.md`](security/fixes-2.md) | The two-step payout that fixes NV-02, its tests and residual risks. |
| [`security/review-2.md`](security/review-2.md) | Review 2: the swap engines (`zecret-core`, `zecret-node`). Findings F2-01 to F2-04. |
| [`security/fixes-2-core.md`](security/fixes-2-core.md) | Fixes for the maker side of review 2, the premium check, the RPC trust statement, and what is still open on the taker side. |
| `security/findings-1.json`, `security/findings-2.json` | Machine-readable records of both reviews. |
| `security/poc/`, `security/poc2/` | Proof-of-concept tests of the findings. |

## Measurements

Write-ups of short experiments made before and during the build. The code that made
them is not in this repository.

| Document | Question |
|---|---|
| [`experiments/gas-spike.md`](experiments/gas-spike.md) | Does the key-half check fit in NEAR gas? |
| [`experiments/intents-sandbox.md`](experiments/intents-sandbox.md) | Can a key with no NEAR account own funds in NEAR Intents and move them with a signature? Does a stipend make a new key usable? |
| [`experiments/near-timing.md`](experiments/near-timing.md) | How long do NEAR testnet transactions take to be included and final? |
| [`experiments/wasm-prover.md`](experiments/wasm-prover.md) | Can the Zcash prover run in a browser, and how fast? |
| [`experiments/grpc-web.md`](experiments/grpc-web.md) | Can a browser read Zcash blocks? |
| [`experiments/browser-wallet.md`](experiments/browser-wallet.md) | The full wallet in a browser worker: receive, read a memo, build and send. |

## Diagrams

| File | What it shows |
|---|---|
| [`diagrams/1-system.png`](diagrams/1-system.png) | The parts of the system and how they talk. |
| [`diagrams/2-happy-path.png`](diagrams/2-happy-path.png) | One swap, step by step. |
| [`diagrams/3-escrow-states.png`](diagrams/3-escrow-states.png) | The escrow's states and exits. |
| [`swap-sequence.md`](swap-sequence.md), `swap-sequence.png` | One Cloakroom swap on Base (baseline, from before the hackathon). |

The `.mmd` files next to the diagrams are their Mermaid sources.

## Elsewhere in the repository

| File | What it is |
|---|---|
| [`../STATUS.md`](../STATUS.md) | Where the build is: done, running, next. |
| [`../BASELINE.md`](../BASELINE.md) | What was imported from Cloakroom: baseline commit `2f8e2f7`, branch `pre-hackathon-baseline` (tag `pre-hackathon` in the build clone). |
| [`../harness/RESULTS.md`](../harness/RESULTS.md) | Results of the 19 two-party scenarios on local chains. |
| [`../harness/BROWSER_RESULTS.md`](../harness/BROWSER_RESULTS.md) | Results of the scenarios with the taker in a browser. |
| [`../contracts/near-escrow/README.md`](../contracts/near-escrow/README.md) | The escrow's interface, deployment rules, gas and tests. |
| [`../crates/zecret-core/README.md`](../crates/zecret-core/README.md), [`../crates/zecret-node/README.md`](../crates/zecret-node/README.md), [`../crates/zecret-intents/README.md`](../crates/zecret-intents/README.md) | The Rust crates. |
| [`../deploy/README.md`](../deploy/README.md), [`../deploy/maker/README.md`](../deploy/maker/README.md) | The deployment tool and the maker as a service. |
| [`../web/README.md`](../web/README.md), [`../web/ENGINE_API.md`](../web/ENGINE_API.md) | Building and testing the website; its interface to the engine. |
