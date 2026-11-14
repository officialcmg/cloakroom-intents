# Demo video and post

A script for a 60 to 90 second video, and two drafts of the post on X. The owner records
and posts. Nothing here has been posted.

The rules for the post (NEAR HACKATHON, "The demo post"): it says the project was built
at NEAR HACKATHON, tags @NEARProtocol and #NEARHackathon (optionally @NearLegion and
@NearBuilders), and has the video or screenshots attached. A good post leads with the
user problem, shows one complete moment from a user action to a visible result,
explains NEAR's role concretely, and gives a next step.

## What to record

The hosted website on Zcash testnet and NEAR testnet:
<https://cloakroom.chrismg.dev>. Record one real swap through it, and use
the simulated demo (`/?demo=1`) only for the failure cases. Every shot of the demo must
show its banner ("Demo. No real money moves."), and the voice-over must say so. Every
shot of the real app shows the banner "Testnet. Test money only."

A real swap takes about 7 to 8 minutes from the payment to done (493 s for 0.05 TAZ and
392 s for 0.01 TAZ in the recorded runs, `docs/TESTNET.md`). Cut the waits in the edit,
and say on screen that time was cut.

## Preparation

- Testnet ZEC in the project's command-line wallet, and the command that pays the
  page's address: `zecret pay --dir <wallet> --to <address> --zats <amount>`
  (`docs/TESTNET.md`, steps 1 and 2). No wallet app was tested as the payer.
- A NEAR testnet account registered with `usdc.zecret-3f109eea1e.testnet`. The recorded
  runs used `zecret-3f109eea1e.testnet`.
- An amount between 0.001 and 0.1 TAZ (the hosted maker's limits).
- Have these NEAR testnet explorer pages open in tabs (hashes from `docs/TESTNET.md`,
  "Hosted swap"), or the ones of your own run:
  - escrow `escrow2.zecret-3f109eea1e.testnet`;
  - `commit_claim` `8uh57oHrWRxcLDRHHDGnzXs9ayPNuyoP3DfX1TqR4Rti` and `claim`
    `D4gdMsdEYku88YMyHa9j5DKTVT9wHGo77mnM1CEEswA6`, signed by the new taker account
    `aa30f22c…843e`;
  - the signed exit `DAKZAsvRKaN9hb6JJTUhLThtVgxpx1PbCccrLsDiXxvT`, sent by the maker's
    relay.
- For the demo shots, the address `0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed` (a public
  test address, used by the website's own tests).

## Script (about 85 s)

| Time | Picture | Words (voice-over or captions) |
|---|---|---|
| 0:00-0:08 | Title card: "Leave shielded Zcash without a custodian." | "Every way out of shielded ZEC today goes through a transparent address first, so the amount and timing become public." |
| 0:08-0:15 | The landing page, then "Start a swap". | "Cloakroom Intents swaps shielded ZEC for a token on another chain. Nobody holds both sides. This is the hosted site on Zcash testnet and NEAR testnet, with test money." |
| 0:15-0:25 | The swap form: amount in TAZ, a token and chain, the NEAR testnet account. The price line. | "I enter an amount. I get one price: what the maker pays in USDC, and, on mainnet, what NEAR Intents delivers on the chain I chose. On testnet the test USDC goes to my NEAR account." |
| 0:25-0:33 | Save the recovery file, then the payment screen with the QR code. Pay from the wallet. | "My browser makes the keys for this swap. I save a recovery file and pay once." |
| 0:33-0:48 | The progress screen, step groups filling in (time cut), then "You received … test USDC on NEAR". | "The maker locks USDC on NEAR. My ZEC goes to an address whose key is split between me and the maker. When I claim the USDC, my half of the key becomes public, and only then can the maker take the ZEC. This run took about eight minutes; I cut the waiting." |
| 0:48-1:05 | NEAR testnet explorer: the `commit_claim` and `claim` signed by the new taker account, then the exit sent by the relay. | "Here it is on NEAR testnet. The maker sent 0.03 NEAR to a brand-new key, and that key sent the claim itself. No gas payment from me. The safety windows are minutes, not hours as on Base." |
| 1:05-1:15 | The simulated demo (`/?demo=1`), banner visible: "Demo controls", the maker refunds, the ZEC comes back. | "This is the simulated demo; no money moves here. If the maker refunds, its refund publishes its key half, and my browser takes the ZEC back." |
| 1:15-1:25 | End card: the site link, the repository link, "Built at NEAR HACKATHON", "Testnet works. Mainnet next." | "The site, the escrow, the tests and the testnet evidence are public. Tell us where you would exit your ZEC to." |

What the video must not say, because it is not verified (see `README.md`, "Status"):
that a swap ran on mainnet, that tokens were delivered to another chain, or that a wallet
app was used as the payer.

## Post, draft 1 (problem first; 277 characters as X counts them)

```text
Every way out of shielded ZEC goes through a transparent address first, so the amount and timing become public.

Cloakroom Intents swaps shielded ZEC with no custodian: a NEAR escrow, a split Zcash key, exit to 30+ chains via NEAR Intents.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon
https://github.com/officialcmg/cloakroom-intents
```

Reply under it (the next step):

```text
Try it on Zcash testnet and NEAR testnet: https://cloakroom.chrismg.dev (test money only; the mainnet exit is next). If you hold shielded ZEC: which chain would you exit to first?
```

## Post, draft 2 (NEAR's role first; 272 characters as X counts them)

```text
How do you leave shielded Zcash without trusting anyone?

A NEAR escrow holds the USDC. A 0.03 NEAR stipend lets a fresh key send your claim. NEAR Intents moves it on with one signature.

Cloakroom Intents, built at NEAR HACKATHON @NEARProtocol #NEARHackathon
https://github.com/officialcmg/cloakroom-intents
```

Reply under it:

```text
Testnet site: https://cloakroom.chrismg.dev. The escrow, the tests and the recorded testnet swaps are in the repo. Feedback on the escrow and the check before signing is welcome.
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
- The replies link the hosted site. It must stay up until at least 2026-11-18.
- The replies are 173 (draft 1) and 171 (draft 2) characters as X counts them.
