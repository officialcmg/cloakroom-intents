# Intents sandbox: payout to an unfunded key, then a gasless exit

**Question.** Can the escrow pay a taker whose key has never touched NEAR, and can that
taker then move the funds without holding any NEAR for gas?

**Result: yes.** The test runs the contract code deployed on `intents.near` (fetched from
mainnet on 2026-11-12) in a local sandbox:

1. A payer deposits tokens into the Verifier with `msg` set to the hex of a fresh
   ed25519 public key. The Verifier credits that account.
2. That account does not exist on-chain and holds no NEAR.
3. The key signs one `ft_withdraw` intent off-chain (`raw_ed25519` standard). A third
   party submits it with `execute_intents` and pays the gas (10.7 Tgas).
4. The tokens arrive at the destination account.

What this does not cover: the 1Click API and solvers, which deliver to other chains.
That path exists only on mainnet.

## Second test: a gas stipend for the taker's key

**Question.** If the maker sends a little NEAR to the account named by the taker's fresh
key, can the taker then send her own transactions with that key?

**Result: yes** (`tests/stipend.rs`). A transfer of 0.01 NEAR to the hex of the public
key creates the account, and the key can sign transactions straight away. So the taker
needs no relayer and no NEAR account of her own: her gas comes from the counterparty,
who already knows about the swap.

## Run

```sh
./fetch-wasm.sh    # downloads intents.near and wrap.near code from mainnet
NEAR_SANDBOX_BIN_PATH=/path/to/near-sandbox-2.14.1 cargo test -- --nocapture
```

The sandbox must be nearcore 2.14.1 or newer. Version 2.9.0 fails to load the live
contract code.
