# Baseline

This commit is the starting point of Zecret. It is a copy of selected folders from
[`officialcmg/cloakroom`](https://github.com/officialcmg/cloakroom) at commit
`bad9fa37f6fb82475ed222bd300f1a3ffc7e08a9` (2026-10-28), which I also wrote, under the
MIT licence. Cloakroom was built for another hackathon, ZECATHON, and trades shielded
ZEC against an ERC-20 token on Base.

The commit is `2f8e2f7`. It is pushed as the branch `pre-hackathon-baseline`. In the
build clone it is also tagged `pre-hackathon`; the git proxy of the build environment
refuses to push tags, so the tag is not on GitHub; the branch is. Nothing in the commit was written for
NEAR HACKATHON. The diff of the hackathon work:
<https://github.com/officialcmg/zecret/compare/pre-hackathon-baseline...main>.

| Folder | What it is |
|---|---|
| `node/` | Rust maker and taker client, Zcash light wallet, encrypted memo messages |
| `contracts/` | The Solidity escrow for Base and its Pallas library, with tests and audit notes |
| `harness/` | Two-party scenario runner on Zebra regtest and anvil |
| `formal/` | TLA+ model and TLC logs |
| `app/`, `design-system/` | Local web UI and its design tokens and components |
| `docs/` | Swap sequence diagram |
| `Cloakroom_Yellow_Paper.pdf` | The protocol paper |

Left out of the copy: `archive/`, `demo/`, `measurements/`, `testnet/`, `paper/` and
`harness/evidence/`.
