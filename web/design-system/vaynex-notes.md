# Style reference: Vaynex (Framer template)

Measured on 2026-11-13 from the live preview https://vaynex.framer.website/ (home and
`/faq`), headless Chromium, viewports 1440 and 390 px, after scrolling the whole page so
that every appear animation ran. Values are computed styles. The template is a style
reference only: no image, icon, text or code of it is used in this repository.

## Type

| Role | Family | Size / line height | Weight | Tracking | Colour |
|---|---|---|---|---|---|
| Hero headline | General Sans | 84 / 84 px | 600 | -3.78 px (-0.045em) | #fff |
| Hero italic word | Instrument Serif italic | 84 / 84 px | 400 | -0.84 px (-0.01em) | #fff |
| Section heading | General Sans | 52 / 55.1 px | 600 | -1.82 px (-0.035em) | #5f706a (first words), #0b2a22 (italic part) |
| Section heading, large | General Sans | 68 / 69.4 px | 600 | -2.72 px (-0.04em) | white 74% |
| Stat number | General Sans | 60 / 60 px | 600 | -2.7 px (-0.045em) | #0b2a22 |
| Price | General Sans | 52 / 52 px | 600 | -2.08 px (-0.04em) | #0b2a22 |
| Card title | General Sans | 20 / 25 px | 500 | -0.3 px (-0.015em) | #0b2a22 |
| FAQ question | General Sans | 20 / 25 px | 500 | -0.3 px | #0b2a22 |
| Footer tagline | General Sans | 30 / 34.5 px | 600 | -0.75 px | #0b2a22 |
| Wordmark | General Sans | 19 / 19 px | 600 | +0.38 px | #0b2a22 |
| Lede | Inter | 18 / 27.9 px | 400 | -0.18 px | #5f706a, or white 78% on green |
| Body in cards | Inter | 15 / 24 px | 400 | 0 | #5f706a |
| Small | Inter | 13 / 19.5 px | 400 | 0 | #5f706a |
| Nav link, button | Inter | 15 / 15 px | 500 | -0.15 px | ink 70% (nav), #fff or #0b2a22 (button) |
| Eyebrow, labels | Geist Mono | 12 / 14.4 px | 500 | +0.96 px (0.08em), uppercase | #0e5c48 |

Headlines mix the sans (first words) with one or two words in the serif italic.

## Colour

| Token | Value |
|---|---|
| Ink (text) | #0b2a22 |
| Muted text | #5f706a |
| Brand green (eyebrows, links) | #0e5c48 |
| Primary button | linear-gradient(180deg, #17735a, #0d4f3e) |
| Logo tile | linear-gradient(135deg, #2a8f72, #0d4a3a) |
| Dark button | linear-gradient(180deg, #1a2e28, #0b1a16) |
| Hero background | linear-gradient(180deg, #0a3a2f 0%, #11523f 34%, #3e8a70 62%, #cfe5dc 84%, #fff 100%) |
| Hero glow | radial-gradient(55% 45% at 50% 40%, rgba(160,230,200,.35), transparent 75%) |
| Light section | linear-gradient(180deg, #f5faf7, #eaf3ee) |
| Dark section (CTA) | linear-gradient(160deg, #13604b, #0b4134 55%, #072b23) |
| Dark card | linear-gradient(155deg, #17735a, #0a3a2f) |
| Light card tints | linear-gradient(155deg, #f7fcfa, #d3ece2); white |
| Soft fills | rgba(14,92,72,.02 to .10) for rows, icon tiles and chips |
| Accent glow | rgba(75,240,192,.12 to .55) |
| Page | #fff |

Light theme only. No dark variant exists in the reference.

## Shape

| Element | Radius | Other |
|---|---|---|
| Section panels | 44 px (40 px for split panels) | full width minus 62 px each side at 1440 |
| Large cards (pricing, footer card) | 30 px | padding 32 to 56 px |
| Cards | 26 px | padding 24 to 28 px |
| Floating cards, FAQ items | 20 to 22 px | FAQ item 820 x 80 px |
| Rows inside cards | 14 to 16 px | |
| Icon tiles | 13 to 14 px | 40 to 44 px square |
| Logo tile | 9 px | 30 px square |
| Buttons, badges, chips | 100 px | button padding 15 x 26 px, height 45 px |
| Nav bar | 22 px | 1180 x 66 px, 14 px from the top |

## Surfaces and depth

- Nav: rgba(255,255,255,.72), backdrop-filter blur(22px), shadow 0 20px 50px -20px rgba(0,0,0,.6).
- Floating cards: rgba(255,255,255,.78 to .85), blur(16 to 18px), shadow 0 30px 60px -20px rgba(0,0,0,.6) on green.
- Cards on white: shadow 0 40px 70px -30px rgba(6,8,12,.35), or 0 40px 80px -50px.
- Primary button: inset 0 1px 0 rgba(255,255,255,.22), 0 10px 24px -10px rgba(13,79,62,.55).
- White button on green: 0 1px 2px rgba(11,42,34,.06), 0 10px 24px -12px rgba(11,42,34,.25).
- Glass button on green: rgba(255,255,255,.12), inset 0 1px 0 rgba(255,255,255,.25), blur(14px).
- Featured card: ring 0 0 0 6px rgba(20,95,75,.08) and 0 40px 80px -30px rgba(10,58,47,.55).
- Big arcs: circles of 1600 to 2000 px with box-shadow 0 -40px 120px rgba(200,255,230,.22) cut by the section edge.
- Badge: white pill, padding 7 12 7 10 px, 6 px dot in brand green before the text.

## Layout

- Content width 1152 px (144 px side margins at 1440); hero text column about 640 px.
- Hero: padding-top 168 px; badge, headline, lede (24 px gap), buttons (28 px gap), then a phone mockup with four floating cards, cut by a white fade.
- Sections alternate: open white sections and rounded light-green panels; 110 px top padding, 170 px bottom.
- Section header: centred badge, heading, lede of at most 2 lines; 72 to 100 px to the content.
- Split sections: two columns of about 528 and 609 px with a 24 px gap.
- Feature lists: icon tile, title 20 px, text 15 px, 20 x 22 px padding per row.
- FAQ: one column of 820 px, cards 10 px apart, a 36 px round plus button at the right.
- Footer: a white card (radius 30, padding 56 52 36) on a gradient from white to deep green, with a huge wordmark under it, cut at the bottom.
- 390 px: one column, 16 to 24 px side padding, hero headline about 44 px, sections radius 32 px.

## Motion

- Appear on load: opacity 0 to 1 and translateY 16 to 30 px (phone: scale .94 and 120 px). About 700 ms per element, a spring ease with a small overshoot, staggered by 150 to 300 ms.
- Appear on scroll: the same, opacity 0 and translateY 28 px.
- Stat cards: rotated -9 to +9 deg and offset 90 to 160 px down before they settle.
- A large word marquee runs sideways (SPEND · PAY · SAVE ...).
- FAQ: the answer opens below the question; the plus button turns.

## Fonts and licences

| Reference | Licence | Used here |
|---|---|---|
| General Sans (Indian Type Foundry, Fontshare) | ITF Free Font License: no subsetting, no format conversion, no distribution through a repository or public server | Not used. Hanken Grotesk (SIL OFL 1.1) instead, the closest open match measured side by side at 84 px, weight 600, tracking -0.045em |
| Inter | SIL OFL 1.1 | Inter, variable weight, Latin subset |
| Instrument Serif italic | SIL OFL 1.1 | Instrument Serif italic, Latin subset |
| Geist Mono | SIL OFL 1.1 | Geist Mono 500, Latin subset |
