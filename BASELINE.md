# Baseline

This commit is the starting point of Zecret. It is a copy of selected folders from
[`officialcmg/cloakroom`](https://github.com/officialcmg/cloakroom) at commit
`bad9fa37f6fb82475ed222bd300f1a3ffc7e08a9` (2026-10-28), by the same author, under the
MIT licence. Cloakroom was built for another hackathon, ZECATHON, and trades shielded
ZEC against an ERC-20 token on Base.

The commit is tagged `pre-hackathon`. Nothing in it was written for NEAR HACKATHON.

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
