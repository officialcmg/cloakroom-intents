# Writing checklist for external text

Use this list for the final edit of the README, the website copy and the demo post.
Read the text once. Then go through the items in order and tick each one. Each item has
a reason and a short example. "Before" marks text to remove.

Two limits apply to every edit:

- Keep every fact, number, date, link and command. An edit changes the wording, never
  the claim.
- Do not add a fact that the repository does not support. If a sentence needs a number
  that you do not have, flag it for the owner.

Some examples use made-up values (an address, a timeout, a port) to show the shape of a
sentence. Take the real values from the code and the evidence files. Sources are at the end.

## 1. Patterns to remove

- [ ] Inflated or promotional words.
  Reason: they make a claim larger without a fact behind it.
  Words to check: seamless, robust, powerful, cutting-edge, revolutionary, leverage,
  empower, unlock, crucial, pivotal, key (as an adjective), showcase, delve.
  Before: "A seamless, robust swap experience."
  After: "One swap on test networks took 222 s (one recorded run)."
- [ ] Stock openers.
  Reason: the first sentence must carry the first fact.
  Before: "In today's rapidly evolving privacy landscape, ..." or "Let's dive in."
  After: "Cloakroom Intents swaps shielded ZEC for a token on another chain with no custodian."
- [ ] Stock closers and empty end summaries.
  Reason: a last line that repeats the section adds nothing. End on the last fact.
  Before: "In short, Cloakroom Intents makes private swaps simple and safe." After: (delete the line)
- [ ] Rule-of-three lists and forced parallelism.
  Reason: a list of three that the meaning does not need sounds made up.
  Before: "Private, trustless, and unstoppable."
  After: "The ZEC stays in the shielded pool for the whole swap."
- [ ] "Not X but Y" contrasts.
  Reason: the first half denies a claim that nobody made. Keep a contrast only when
  the reader really believes X.
  Before: "This isn't just a bridge. It's a new way to move value."
  After: "The swap uses an escrow contract on NEAR and a split Zcash key."
  Keep: "Inside NEAR Intents, ZEC is a NEP-141 token, not shielded ZEC." (Readers
  assume it is shielded.)
- [ ] Excessive hedging.
  Reason: stacked qualifiers hide what you know. State the claim at the level of
  certainty that the evidence gives, once.
  Before: "This could potentially help to possibly reduce exposure."
  After: "The swap publishes no transparent Zcash address."
- [ ] Signposting that announces instead of saying.
  Reason: "here is what you need to know" delays the fact.
  Before: "It's worth noting that the maker can refund." After: "The maker can refund."
- [ ] Em dashes and other punctuation tics.
  Reason: dashes everywhere replace a decision about how two clauses connect. Use a
  period, comma, colon or parentheses. Also check for "--", ellipses and semicolon
  chains.
  Before: "The key is split — neither side can spend alone."
  After: "The key is split. Neither half can spend alone."
- [ ] Vague attributions.
  Reason: "experts say" gives no source that the reader can check. Name the source or
  cut the claim.
  Before: "Many in the community see this as a major gap."
  After: "The Cloakroom paper (October 2026) states that every route out of shielded
  Zcash passes through a transparent address."
- [ ] Uniform sentence rhythm.
  Reason: many sentences of the same length and shape read as generated. Vary length by
  meaning. Do not chop sentences into fragments to fake rhythm.
  Before: "The maker locks USDC. The taker checks it. The taker sends ZEC. The maker
  sets Ready."
  After: "The maker locks the USDC. When the taker has checked the escrow on NEAR, it
  sends the ZEC to the shared address."
- [ ] Headings that are slogans.
  Reason: a heading must say what the section holds. Use sentence case.
  Before: "## Privacy, Reimagined" After: "## How one swap runs"
- [ ] Emoji and exclamation marks.
  Reason: they add tone without information, and screen readers read emoji names
  aloud.
  Before: "Swap complete! 🎉" After: "Swap complete. 41.2 USDC arrived at 0x3f...a9 on Base."
- [ ] Over-bolding.
  Reason: when every line has bold, nothing stands out. Bold at most one phrase in a
  paragraph, and only when a reader who skims must see it. Remove bold labels from
  list items that carry no extra information.
  Before: "**Fast**, **private** and **secure** swaps."
  After: "Swaps that keep the ZEC shielded."
- [ ] Passive voice that hides who does what.
  Reason: in a two-party protocol the reader must know which party acts.
  Before: "The funds are released after the key is revealed."
  After: "The maker takes the ZEC after the taker's claim publishes key half `a`."
- [ ] Undefined jargon and abbreviations.
  Reason: a judge may not know NEP-141, "Verifier" or "stipend". Define a term the first
  time, or link to the Terms table.
  Before: "The stipend covers gas."
  After: "The maker sends 0.03 NEAR (the stipend) to pay the gas for the taker's claim."
- [ ] Fancy verbs in place of "is" and "has".
  Reason: "serves as", "boasts" and "features" are longer and say less.
  Before: "The escrow serves as a trustless vault and boasts two exits."
  After: "The escrow holds the USDC and has two exits."
- [ ] Leftover chat phrases.
  Reason: lines such as "I hope this helps" or "Let me know if" show that the text was
  pasted from a conversation.
  Before: "Feel free to reach out with any questions!" After: "Questions: open an issue."
- [ ] Text that describes itself.
  Reason: "This section explains" and "The table below shows" tell the reader what they
  can already see.
  Before: "Below, we outline the security considerations."
  After: (start the section with the first security fact)

## 2. Rules to apply

- [ ] Lead with the fact the reader needs.
  Reason: judges and screeners read the first lines and stop.
  Example: the README's first paragraph says what Cloakroom Intents does and for whom.
- [ ] Give numbers with units, a date and a source.
  Reason: a number without a unit or a source cannot be checked.
  Before: "Swaps are fast." After: "222 s from start to the taker's USDC, one run on testnet
  (`testnet/EVIDENCE.json`)."
- [ ] One idea in each sentence.
  Reason: short sentences are easier to check and translate. As a guide, keep steps
  under 20 words and descriptions under 25 words (ASD-STE100 limits).
  Before: "The maker, which runs in our backend, locks USDC, after which it sends a
  stipend, so the taker can claim."
  After: "The maker locks the USDC. Then it sends the stipend to the taker's NEAR key."
- [ ] Use the same word for the same thing.
  Reason: a new word makes the reader think of a new thing.
  Before: "key half", then "key share", then "secret fragment".
  After: "key half" every time, as in the Terms table.
- [ ] Write steps as numbered imperative sentences with the expected result.
  Reason: the reader must know whether the step worked.
  Before: "1. The tests can be run with cargo."
  After: "1. Run `cargo test -p near-escrow`. Expect `57 passed`."
- [ ] State limits plainly, close to the claim they limit.
  Reason: a limit that the reader finds later makes every other claim look doubtful.
  Example: "Done, once. Happy path only. The browser taker was not used."
- [ ] Define a term once, then use it without a new definition.
  Reason: two definitions can drift apart.
  Example: define "Maker" in the Terms table and link to that table.
- [ ] Put statements in positive form when you can.
  Reason: the reader then has one meaning to decode, not two.
  Before: "The taker does not lack a way to recover."
  After: "The taker can recover the ZEC with the recovery file."
- [ ] Cut words that you can cut.
  Reason: each extra word costs the reader time.
  Before: "In order to be able to start the swap" After: "To start the swap"
- [ ] Use descriptive link text.
  Reason: "here" says nothing when the reader scans links.
  Before: "See [here](docs/TESTNET.md)." After: "See the [testnet run](docs/TESTNET.md)."
- [ ] Read the text aloud. Reason: you hear stiff phrasing that you miss on screen.

## 3. Texts in this project

### README for judges and automated screeners

- [ ] The first paragraph says what the project does, on which chains, and what is
  new at this hackathon.
- [ ] The status table is the only place that says how far each part works. Other
  sections link to it and do not restate it.
- [ ] Each "try it" block is "do X, see Y": one command or click per step, then the
  output the reader will see.
  Example: "2. Open `http://localhost:5173`. The quote screen shows one price in USDC."
- [ ] Each claim of "done" or "works" links to its evidence file.
- [ ] Words that a screener matches literally (track names, chain names, "testnet",
  "mainnet") are spelled the same as in the hackathon rules.
- [ ] Nothing says "production-ready", "secure" or "audited" unless a file shows it.
  Write what was reviewed and what is still open.

### Website microcopy for a money-moving flow

- [ ] Each error message answers three questions in this order: what happened, where
  the money is now, and what the user can do next.
  Before: "Something went wrong. Please try again."
  After: "The maker did not lock the USDC within 10 minutes. Your ZEC is still in your
  temporary wallet. Download the recovery file, then start a new swap or send the ZEC
  back to your own wallet."
- [ ] Say "safe" only when it is true at that moment, and say why.
  Example: "Your ZEC is safe: the maker cannot spend it without your key half."
- [ ] No blame, no "oops", no "please" in instructions, no "invalid".
  Before: "Invalid address! Please fix it."
  After: "Enter a Base address that starts with 0x and has 42 characters."
- [ ] Buttons say the action, and match the question in the title.
  Before: title "Refund your ZEC?", buttons "OK" and "Cancel".
  After: buttons "Refund ZEC" and "Keep waiting".
- [ ] Show amounts with their unit and chain every time: "0.1 ZEC", "3 USDC on NEAR".
- [ ] Waiting screens say what the app waits for and how long that usually takes.
  Example: "Waiting for 3 Zcash confirmations (about 4 minutes)."
- [ ] Error text in the field and in the error summary is the same.

### Short social post

- [ ] The first line says what the project does. No hook question and no teaser.
  Before: "What if you could swap ZEC without anyone watching? 👀"
  After: "Cloakroom Intents swaps shielded ZEC for tokens on other chains, with no custodian."
- [ ] One concrete result with a number, and a link to the proof.
  Example: "Test run: 0.1 ZEC to 3 USDC, testnet to testnet, 222 s."
- [ ] One honest limit.
  Example: "Testnet only so far. Mainnet needs a deploy."
- [ ] At most two hashtags or tags, and only the ones the hackathon requires.
- [ ] No thread of empty lines between one-sentence "punch" lines.

## 4. Final pass

- [ ] Search the text for a dash character (—, –) and for "--" used as a dash.
- [ ] Search for "!" outside code.
- [ ] Search for these words: seamless, robust, leverage, empower, unlock, crucial,
  pivotal, delve, simply, just, easy.
- [ ] Compare each number in the edited text with the number in the source file.
- [ ] Check that every link opens.
- [ ] Check that no file names a writing tool or model (a repository rule).

## Sources

Read on 2026-11-13 unless the line says otherwise. Star counts: GitHub search API, same day.

Public editing guides (skill files for coding assistants):

- <https://github.com/blader/humanizer>: 26 patterns of machine-written prose, each with
  a before and after example; about 68,500 stars. Main source for section 1.
- <https://github.com/hardikpandya/stop-slop>: short rules and quick checks (no em
  dashes, active voice, varied rhythm); about 18,900 stars.
- <https://github.com/conorbronsdon/avoid-ai-writing>: severity tiers and a rule against
  invented facts in a rewrite; about 5,600 stars. Source for the limits at the top.
- [Wikipedia: Signs of AI writing](https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing):
  the field guide of WikiProject AI Cleanup, which the first guide is based on.

Style guides:

- Federal Plain Language Guidelines: plainlanguage.gov now redirects to
  <https://digital.gov/guides/plain-language>; the original text is at
  <https://github.com/GSA/plainlanguage.gov/tree/main/_pages/guidelines>.
- [GOV.UK style guide, A to Z](https://www.gov.uk/guidance/style-guide/a-to-z): "Words to
  avoid" (deliver, empower, impact, key, leverage, robust, streamline).
- [GOV.UK Design System, error message](https://design-system.service.gov.uk/components/error-message/):
  say what happened and how to fix it; no "please", "sorry", "oops" or "invalid".
- Google developer documentation style guide: [highlights](https://developers.google.com/style/highlights),
  [voice and tone](https://developers.google.com/style/tone), [procedures](https://developers.google.com/style/procedures).
- Microsoft Writing Style Guide: [top 10 tips](https://learn.microsoft.com/en-us/style-guide/top-10-tips-style-voice);
  errors in [app writing style](https://learn.microsoft.com/en-us/windows/apps/design/style/writing-style).
- ASD-STE100: [about STE](https://www.asd-ste100.org/about.html) (one word, one meaning).
  The 20 and 25 word limits come from a search summary of the
  [Issue 9 specification](https://www.asd-ste100.org/assets/files/ASD-STE100_ISSUE9.pdf),
  not read in full.
- Orwell, [Politics and the English Language](https://www.orwellfoundation.com/the-orwell-foundation/orwell/essays-and-other-works/politics-and-the-english-language/) (1946): the six rules.
- Strunk and White, The Elements of Style, rules on active voice, positive form,
  concrete language and needless words. Checked through secondary summaries only, for
  example <https://www.npr.org/2009/04/15/103147654/excerpts-the-elements-of-style>.

README guidance:

- [Make a README](https://www.makeareadme.com/) (<https://github.com/dguo/make-a-readme>,
  about 740 stars): show usage examples with the expected output.
- [Awesome README](https://github.com/matiassingers/awesome-readme): example READMEs,
  about 21,700 stars. It lists examples and gives no writing rules.
