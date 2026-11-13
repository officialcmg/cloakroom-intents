# Submission to NEAR HACKATHON

Everything the owner needs to submit Zecret. Nothing here has been sent. The rules are in
`HACKATHON.md` of the event (live copy: <https://nearhackathon.com/HACKATHON.md>; read it
again before submitting, because dates and rules may change).

**Deadline:** development ends and submissions close at 2026-11-14T23:59:00Z. Outside
the window the API answers 403.

**One attempt.** A repository can be submitted once. A mistake is fixed by a change
request at <https://nearhackathon.com/request-changes>, never by a second submission.
The payload is sent once, only after the owner has read it in full and said yes.

## 1. Before submitting

| Step | Who | State on 2026-11-13 |
|---|---|---|
| Make `github.com/officialcmg/zecret` public (the rules require it by the deadline, and it must stay public through judging). | Owner | Not done: the page answered 404 without login. |
| The baseline commit `2f8e2f7` is on the remote as the branch `pre-hackathon-baseline`, and the README's diff link (`compare/pre-hackathon-baseline...main`) uses it. The tag `pre-hackathon` exists only in the build clone, because the build environment's git proxy refuses to push tags. Optionally push that tag, and tag the submitted commit `hackathon-submission` (rule Q-003). | Owner | Branch pushed: `git ls-remote origin` shows `main` and `pre-hackathon-baseline`, no tags. |
| Update the status table in `README.md` (one table, marked with a comment) to what `STATUS.md` says at submission time. | Owner or agent | Written for the state at the time of writing. |
| Decide on the sentence about AI coding assistance in `README.md`, "Libraries, templates and tools" (see section 5). | Owner | Open. |
| Record the demo video and publish the post on X (`docs/DEMO_SCRIPT.md`). | Owner | Not done. |
| Create or choose the NEAR mainnet account for the prize. | Owner | Not done. |
| If the owner sends another project to this hackathon: name it in this README, and name Zecret in that project's README (rule Q-015). | Owner | Unknown. |

## 2. The fields

From "What the human decides" in the rules. "OWNER" means only the owner can answer. Do
not guess, generate or fill in any OWNER value.

| Field | Rule | Value |
|---|---|---|
| Project name (`projectName`) | 2-80 characters | Proposed: `Zecret` |
| Team lead's name (`leadName`) | 2-80 characters | OWNER |
| Contact email (`email`) | valid, at most 160 characters; at most 2 submissions per email | OWNER |
| Team size (`size`) | one of `1`, `2 - 4`, `4 - 6`, `6 - 8`, `8+`, exactly as written | OWNER |
| Tracks (`tracks`) | ids from the Tracks table, each once | Proposed: `["defi", "privacy"]` (reasons below) |
| NEAR mainnet account for the payout (`nearAddress`) | a named account such as `team.near`, or a 64-character implicit account; lowercase | OWNER. The owner creates it in a NEAR wallet. Nobody creates one for the owner. |
| Repository link (`projectUrl`) | public http(s) URL, at most 300 characters, submitted once | Proposed: `https://github.com/officialcmg/zecret` (the `origin` of this clone). It must be public first. |
| Demo post link (`demoUrl`) | public http(s) URL, at most 300 characters | OWNER, after the post is published |
| What they built (`idea`) | 20-2000 characters: the problem, the product, how NEAR fits in | Proposed texts below |
| Consent (`consent`) | `true` only after the owner agrees | OWNER. Read this to the owner verbatim: "I agree that the organizers may use these details to review our submission and contact us about this hackathon." |

### Tracks

| Track | Proposed | Why |
|---|---|---|
| `defi` (Unstoppable DeFi) | Yes | The track asks for exchanges "that settle across chains through NEAR Intents", with "no custodian". Zecret is an exchange with no custodian: an escrow contract on NEAR holds the maker's USDC, and the exit to another chain goes through NEAR Intents. |
| `privacy` (Privacy & Private AI) | Yes | The track names "private finance". The ZEC stays in Zcash's shielded pool, the swap messages are encrypted memos, and a new NEAR key per swap, funded by the maker, leaves no gas payment that links the user to the swap. |
| `open` (Open Innovation) | No | Zecret fits `defi` and `privacy` directly. |
| `realworld` (Real-World Money) | No | Zecret is not built for businesses' payments. |

### "What they built": short version (384 characters)

```text
Zecret swaps shielded ZEC for tokens on other chains with no custodian. A maker locks USDC in an escrow on NEAR; the user's ZEC goes to a shielded address whose key is split between user and maker. Claiming the USDC reveals the user's key half; a refund reveals the maker's. The USDC lands in NEAR Intents under the user's own key, and one checked signature sends it to another chain.
```

### "What they built": long version (1,549 characters)

Update the "Status" paragraph if `STATUS.md` changes before submission.

```text
The problem: every route out of shielded Zcash passes through a transparent address held by an exchange, a bridge or an intent network, which publishes amounts and timing and can hold funds.

Zecret swaps shielded ZEC for a token on another chain with no custodian. A maker locks USDC in an escrow contract on NEAR. The user's ZEC goes to a shielded address whose spending key is split between the user and the maker. Claiming the USDC publishes the user's key half, so the maker can take the ZEC; a refund publishes the maker's half, so the user takes the ZEC back. The ZEC never leaves the shielded pool.

How NEAR fits in: the escrow is a Rust contract on NEAR with no access keys. Each swap uses a new ed25519 implicit account that the maker funds with a 0.03 NEAR stipend, so the user sends her own claim with no relayer and no gas payment that links her to the swap. NEAR's fast finality shrinks the protocol's safety windows from hours (on Base) to minutes. The USDC is paid into NEAR Intents under the user's key, and one signature, checked before signing, sends it to a token on another chain through 1Click.

Status: the escrow and the native maker and taker pass 19 two-party scenarios on local chains, and one swap ran on Zcash testnet and NEAR testnet. The website runs against a simulated engine while the browser engine is finished. The exit with real funds on mainnet is not yet verified. Zecret builds on the author's earlier Cloakroom protocol (ZECATHON, Base), disclosed as baseline commit 2f8e2f7 (branch pre-hackathon-baseline).
```

Sources for each statement: the problem sentence is from the abstract of
`Cloakroom_Yellow_Paper.pdf`; the rest is in `README.md`, `docs/DESIGN.md`,
`docs/TESTNET.md`, `harness/RESULTS.md` and `STATUS.md`.

## 3. The payload

The exact shape from the rules. Send exactly these fields and no others. In particular
do not add `website`: any non-empty value rejects the submission. Every `<...>` is a
placeholder that must be replaced; `OWNER` values come only from the owner.

```json
{
  "requestId": "<UUID: generate once, keep it, reuse it on a retry>",
  "projectName": "Zecret",
  "leadName": "<OWNER: team lead's name>",
  "email": "<OWNER: contact email>",
  "size": "<OWNER: one of 1 | 2 - 4 | 4 - 6 | 6 - 8 | 8+>",
  "tracks": ["defi", "privacy"],
  "nearAddress": "<OWNER: mainnet account, for example name.near>",
  "projectUrl": "https://github.com/officialcmg/zecret",
  "demoUrl": "<OWNER: link to the published X post>",
  "idea": "<the short or the long text above, as a JSON string>",
  "consent": <OWNER: true only after agreeing to the contact notice>
}
```

Field checks before sending (from the rules' Submission API table):

- `requestId`: a UUID. Generate it once with `uuidgen | tr A-Z a-z` or
  `node -e 'console.log(crypto.randomUUID())'`. A retry after a network error or a 503
  reuses the same value.
- `nearAddress`: 2-64 characters, lowercase, matching
  `^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$`.
- `idea`: 20-2000 characters. The long text uses `\n\n` between paragraphs inside the
  JSON string.
- The whole body: at most 12,000 bytes.
- Do not commit the payload. It contains personal data.

## 4. How to send it

Only after the owner has confirmed the full printed payload:

```sh
curl -sS -X POST https://nearhackathon.com/api/applications \
  -H 'Content-Type: application/json' \
  -d @payload.json
```

| Answer | Meaning | What to do |
|---|---|---|
| 201 | Saved | Tell the owner. Done. |
| 400 | A field failed | Fix the fields named in `fields`; resend with the same `requestId`. |
| 403 | Submissions closed, or the request looked cross-site | Tell the owner. Do not retry. |
| 409 | This repository was already submitted, or the email has 2 submissions | Ask the owner. Do not change the link or the email to get around it. |
| 413 | Body too large | Use the short `idea` text. |
| 503 | Storage unavailable | Wait a minute; resend with the same `requestId`. |

The same fields can be entered by hand at <https://nearhackathon.com/#apply>.

## 5. The sentence about AI coding assistance

Rule Q-005 asks to list libraries, templates and boilerplate in the README, "including
own pre-hackathon libs and AI code". This repository's own rule (`CLAUDE.md`) forbids
naming AI tools in any file. `README.md`, section "Libraries, templates and tools", has
one neutral sentence that names no product:

> Parts of the code and documentation were written with AI coding assistance, and were
> reviewed and tested as described in this file and in `docs/`.

The owner decides whether to keep it, reword it or remove it.
