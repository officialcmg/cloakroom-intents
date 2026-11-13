# Demo video and post

A script for a 60 to 90 second video, and two drafts of the post on X. The owner records
and posts. Nothing here has been posted.

The rules for the post (NEAR HACKATHON, "The demo post"): it says the project was built
at NEAR HACKATHON, tags @NEARProtocol and #NEARHackathon (optionally @NearLegion and
@NearBuilders), and has the video or screenshots attached. A good post leads with the
user problem, shows one complete moment from a user action to a visible result,
explains NEAR's role concretely, and gives a next step.

## What to record depends on the browser engine

The taker engine in the browser was still being built when this was written (see the
status table in `README.md`). Two versions of the middle part of the video follow.

- **Version A, now:** the website with the simulated engine, plus the recorded swap on
  Zcash testnet and NEAR testnet. Every shot of the simulated engine must show its banner
  ("Demo with a simulated engine: no real funds move.") and the voice-over must say so.
- **Version B, when `STATUS.md` lists package E as done:** the website with the real
  engine. Re-record the shots marked **[RE-RECORD IF ENGINE DONE]**. Use the local or
  testnet setup in `web/README.md`. Say which network it is; the page shows a banner with
  the network's name.

Shots that do not change: the opening, the NEAR explorer shots of the recorded testnet
swap, and the close.

## Preparation

- Website, version A: `cd web && npm install && npm run dev`, then open
  `http://localhost:5173/?speed=20`. At speed 20 the full swap takes about 30 s.
- Have these NEAR testnet explorer pages open in tabs (hashes from `docs/TESTNET.md`):
  - escrow `escrow2.zecret-3f109eea1e.testnet`;
  - stipend `CTza5f9XvaCXYuXSoGsjSgiA3k3RGRxjNehfp76TXHbf` (maker to the taker key);
  - `claim` `DGShdiHZfLWRyUyWX69dDCt14JV86FFHJq1Twowmk3Dz` (signed by the taker key
    `15968c4a…a9067`);
  - `withdraw` `DodbZPEToNesVSF5G9yQUogm1zN9FqF3hdTdwEGPepL6` (payout into the Verifier).
- Address for the form: `0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed` (a public test
  address, used by the website's own tests).

## Script (about 85 s)

| Time | Picture | Words (voice-over or captions) |
|---|---|---|
| 0:00-0:08 | Title card: "Leave shielded Zcash without a custodian." | "Every way out of shielded ZEC today goes through an exchange, a bridge or a transparent address. Each one sees your amount, and can hold your funds." |
| 0:08-0:15 | The Zecret form, empty. **[RE-RECORD IF ENGINE DONE]** | "Zecret swaps shielded ZEC for a token on another chain. Nobody holds both sides." |
| 0:15-0:25 | Type `1` ZEC, keep USDC on Base, paste the address. The price line appears: "You pay 1.00 ZEC. You receive at least … USDC on Base." **[RE-RECORD IF ENGINE DONE]** | "I enter an amount and an address. I get one price: what the maker pays in USDC, and what NEAR Intents delivers on Base." |
| 0:25-0:33 | Continue, save the recovery file, the QR code. **[RE-RECORD IF ENGINE DONE]** | "My browser makes the keys for this swap. I save a recovery file and pay once, from any shielded wallet." |
| 0:33-0:48 | Press "Simulate the payment" (version A) or pay from a wallet (version B). The six progress groups fill in; the step "You claim the USDC". Then the done screen: "You received … USDC on Base" and "What this swap revealed". **[RE-RECORD IF ENGINE DONE]** | Version A: "This is the demo engine; no money moves here. The same steps ran with real transactions on testnet." Version B: name the network. Both: "The maker locks USDC on NEAR. My ZEC goes to an address whose key is split between me and the maker. When I claim the USDC, my half of the key becomes public, and only then can the maker take the ZEC." |
| 0:48-1:05 | NEAR testnet explorer: the stipend transfer, then the `claim` signed by the taker key, then the `withdraw` into the Verifier. | "Here is that swap on NEAR testnet. The maker sent 0.03 NEAR to a brand-new key, and that key sent the claim itself. No relayer, and no gas payment from me. The claim was final about ten seconds after the commit, so the safety windows are minutes, not hours as on Base." |
| 1:05-1:15 | The done screen again, or the "Delivery" track. | "The USDC lands in NEAR Intents under my key. One signature sends it on. Before signing, the app checks that the payload moves exactly the amount I approved to the quoted address, and nothing else." |
| 1:15-1:25 | End card: repository link, "Built at NEAR HACKATHON", "Testnet works. Mainnet exit next." | "The escrow, the tests and the testnet swap are in the repository. Tell us where you would exit your ZEC to." |

What the video must not say, because it is not verified (see `README.md`, "Status"):
that a swap ran on mainnet, that tokens were delivered to another chain, or that the
website is hosted.

## Post, draft 1 (problem first; 277 characters as X counts them)

```text
Every way out of shielded ZEC uses an exchange, a bridge or a transparent address.

Zecret swaps shielded ZEC with no custodian: a NEAR escrow, a split Zcash key, exit to 30+ chains via NEAR Intents.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon
https://github.com/officialcmg/zecret
```

Reply under it (the next step):

```text
It works on Zcash testnet and NEAR testnet today; the mainnet exit is next. If you hold shielded ZEC: which chain would you exit to first?
```

## Post, draft 2 (NEAR's role first; 272 characters as X counts them)

```text
How do you leave shielded Zcash without trusting anyone?

A NEAR escrow holds the USDC. A 0.03 NEAR stipend lets a fresh key send your claim. NEAR Intents moves it on with one signature.

Zecret, built at NEAR HACKATHON @NEARProtocol #NEARHackathon
https://github.com/officialcmg/zecret
```

Reply under it:

```text
The escrow, the tests and a recorded testnet swap are in the repo. Feedback on the escrow and the check before signing is welcome.
```

Notes:

- X counts every link as 23 characters. The counts above use that rule. An account
  without X Premium can post at most 280 characters.
- Optional tags: @NearLegion and @NearBuilders. Adding them to draft 1 or 2 goes over
  280 characters; put them in the reply.
- Attach the video. If the video is not ready, the rules allow screenshots, but say so:
  judges cannot open a local demo, so a video is better (rule Q-012).
- "30+ chains": the 1Click token list had 230 tokens on 39 chains on 2026-11-13, and the
  website offers 35 of those chains. The exit to another chain has not run with real
  funds; the reply says "the mainnet exit is next".
- If a hosted demo exists by then, add its link to the reply. It must stay up until at
  least 2026-11-18.
