// The landing page at /. It says what Cloakroom Intents does, who holds the money, what is private,
// what happens on a failure and how far the project is. It loads no engine: the swap app
// at /swap starts the engine when the user opens it.
//
// Every claim here is backed by README.md, docs/DESIGN.md ("The design in plain words"),
// docs/TESTNET.md, harness/BROWSER_RESULTS.md, STATUS.md or public/config/testnet.json.
// Change the text only together with those sources. The chain count comes from the chain
// table (src/lib/chainNames.ts, checked against the 1Click token list by a unit test).
import { useEffect, type ReactNode } from 'react';
import { APP_PATH, hasMarkedSwap, onLinkClick } from '../app/route';
import { DESTINATION_CHAIN_NAMES } from '../lib/chainNames';
import { MAINNET_BUILD, NETWORK_SITES } from '../lib/netconfig';

/** True for this build: whether a mainnet site is configured (VITE_ZECRET_MAINNET_URL). */
const MAINNET_STATE = NETWORK_SITES.mainnet ? 'Mainnet runs on a separate site.' : 'Mainnet is not deployed.';
import { ChainIcon } from '../ui/ChainIcon';
import { Icon, type IconName } from '../ui/Icon';
import { Logo } from '../ui/bits';
import { TokenAvatar } from '../ui/TokenAvatar';
import { useReveal } from '../ui/reveal';
import { DESIGN_URL, Footer, Header, PAPER_URL, REPO_URL } from './Chrome';

const DEMO_PATH = `${APP_PATH}?demo=1`;
const FAUCET_URL = 'https://fauzec.com';
const CHAIN_COUNT = DESTINATION_CHAIN_NAMES.length;

/** Loads the swap app's code early when the user is about to open it (the engine still starts only on /swap). */
const prefetchApp = () => void import('../App');

function AppLink({ href = APP_PATH, className, children }: { href?: string; className: string; children: ReactNode }) {
  return (
    <a href={href} className={className} onClick={onLinkClick} onPointerEnter={prefetchApp} onFocus={prefetchApp}>
      {children}
    </a>
  );
}

function Out({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a href={href} rel="noopener noreferrer">
      {children}
    </a>
  );
}

export function Landing() {
  useEffect(() => {
    document.title = 'Cloakroom Intents: swap shielded ZEC with no custodian';
  }, []);
  useReveal();
  const resume = hasMarkedSwap();
  const startLabel = resume ? 'Open your swap' : 'Start a swap';
  return (
    <div className="app app-landing">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <Header sections>
        <AppLink className="btn btn-primary btn-sm topbar-cta">{startLabel}</AppLink>
      </Header>
      <main id="main" className="landing">
        <section className="hero on-dark" aria-labelledby="hero-title">
          <div className="hero-text">
            {resume && (
              <div className="hero-resume" role="status">
                <Icon name="clock" className="hero-resume-icon" />
                <div>
                  <p className="hero-resume-title">This browser holds a swap</p>
                  <p>Open it to see where it is. The swap continues while the app is open.</p>
                </div>
                <AppLink className="btn btn-light btn-sm">
                  Open your swap <Icon name="arrow" size={16} />
                </AppLink>
              </div>
            )}
            <p className="badge appear">
              <span className="badge-dot" aria-hidden="true" />
              {MAINNET_BUILD ? 'Mainnet · real money · unaudited' : 'Testnet · test money only'}
            </p>
            <h1 id="hero-title" className="hero-title appear">
              Swap from fully shielded ZEC <em className="serif">to 30+ chains</em>
            </h1>
            <p className="hero-lede appear">
              To swap ZEC today, your coins first leave the shielded pool for a transparent address, so the amount and the timing become public. Here your ZEC stays shielded on its way to the maker, and no
              custodian holds it. The USDC lands in NEAR Intents under a key made in your browser, and Intents sends it on to the chain you choose.
            </p>
            <p className="hero-note appear">
              {MAINNET_BUILD
                ? 'This site runs on Zcash mainnet and NEAR mainnet with real money. It is hackathon software with no security audit. The maker takes only small swaps. Pay only what you can afford to lose.'
                : `This runs on Zcash testnet and NEAR testnet, with test money. ${MAINNET_STATE} On testnet the tokens arrive in your NEAR testnet account, and the price shown for other chains is a mainnet preview.`}
            </p>
            <div className="hero-ctas appear">
              <AppLink className="btn btn-light btn-lg">
                {startLabel} <Icon name="arrow" />
              </AppLink>
              <AppLink href={DEMO_PATH} className="btn btn-glass btn-lg">
                Try the simulated demo
              </AppLink>
            </div>
            {!MAINNET_BUILD && (
            <p className="hero-hint appear">
              On testnet you need no Zcash wallet for a swap of up to 1 ZEC: the{' '}
              <a href={FAUCET_URL} target="_blank" rel="noreferrer noopener">
                Fauzec faucet
              </a>{' '}
              pays 1 test ZEC to the swap’s address. The demo moves no money.
            </p>
            )}
          </div>
          <HeroVisual />
        </section>

        <ChainStrip />

        <section id="steps" className="panel panel-light steps-panel" aria-labelledby="steps-title">
          <SectionHead id="steps" eyebrow="How it works" title={<>How a swap <em className="serif">works</em></>}>
            Five steps. You act in the first two, and your browser does the rest.
          </SectionHead>
          <ol className="step-cards" data-reveal>
            <Step n={1} title="Choose the amount and the destination">
              Enter how much ZEC you pay and where the tokens go. The page shows one price: the USDC the maker pays, and what NEAR Intents delivers for it.
            </Step>
            <Step n={2} title="Pay once from your own Zcash wallet">
              Save the recovery file. Then pay the exact amount to the shielded address on the screen, by QR code or by copying it.
            </Step>
            <Step n={3} title="Your browser does the rest" featured>
              It agrees the firm price with the maker in encrypted Zcash memos, checks the escrow on NEAR, sends your ZEC to the shared address and claims the USDC.
            </Step>
            <Step n={4} title="The USDC arrives under your key">
              The escrow moves the USDC into NEAR Intents, under a key that only your browser holds.
            </Step>
            <Step n={5} title="Out to another chain">
              One signature from your browser lets NEAR Intents send the USDC on as the token and chain you chose. On testnet there is no 1Click, so the tokens go to your NEAR testnet
              account instead.
            </Step>
          </ol>
        </section>

        <section id="custody" className="split" aria-labelledby="custody-title">
          <div className="split-visual on-dark" data-reveal>
            <FlowDiagram />
          </div>
          <div className="split-card" data-reveal>
            <SectionHead id="custody" eyebrow="Custody" title={<>Who holds <em className="serif">the money</em></>} align="start">
              No company or server holds your money at any step. Your ZEC is under your browser’s key, then at an address that needs two key halves. The maker’s USDC is in an escrow
              contract, then in NEAR Intents under your browser’s key.
            </SectionHead>
            <ul className="feature-list">
              <Feature icon="key" title="Two key halves">
                The shared Zcash address is spent with two key halves. Your browser holds one, the maker holds the other. One half alone cannot spend.
              </Feature>
              <Feature icon="lock" title="The escrow has two exits">
                Your claim publishes your key half: you get the USDC, and the maker can take the ZEC. The maker’s refund publishes its key half: you take your ZEC back.
              </Feature>
              <Feature icon="shield" title="No owner, no upgrade">
                The escrow contract on NEAR has no owner, no upgrade method and no access keys. Nobody can change its code.
              </Feature>
            </ul>
          </div>
        </section>

        <section id="privacy" className="section" aria-labelledby="privacy-title">
          <SectionHead id="privacy" eyebrow="Privacy" title={<>What stays <em className="serif">private</em> and what is <em className="serif">public</em></>}>
            The ZEC stays in the shielded pool for the whole swap. The steps on NEAR and the delivery on the destination chain are public.
          </SectionHead>
          <div className="privacy-grid">
            <PrivacyVisual />
            <div className="privacy-card" data-reveal>
              <div className="privacy-card-head">
                <span className="icon-tile">
                  <Icon name="shield" />
                </span>
                <h3 className="card-title">Private</h3>
                <span className="chip chip-status">Shielded</span>
              </div>
              <ul className="check-list">
                <li>Your Zcash wallet, its balance and its history. The ZEC stays in the shielded pool for the whole swap.</li>
                <li>Which Zcash wallet paid. Nothing on NEAR or on the destination chain points back to it.</li>
                <li>The messages between your browser and the maker. They are encrypted memos in shielded Zcash transactions.</li>
                <li>
                  Your NEAR gas. The swap uses a new NEAR key. Its gas comes from 0.06 NEAR that the maker sends to that key (the stipend), so no account of yours pays gas on NEAR.
                </li>
              </ul>
            </div>
            <div className="privacy-card" data-reveal>
              <div className="privacy-card-head">
                <span className="icon-tile">
                  <Icon name="eye" />
                </span>
                <h3 className="card-title">Public</h3>
                <span className="chip chip-status chip-status-public">On chain</span>
              </div>
              <ul className="check-list check-list-dot">
                <li>On NEAR: the escrow, the USDC amount, the time of each step and the new key.</li>
                <li>On the destination chain: the amount and the address that receives it.</li>
                <li>The maker knows that a swap of this size happened, and when. It does not learn your Zcash wallet.</li>
                <li>Timing. The website adds no delay. When few swaps run, the times of the steps can narrow down which payment belongs to which swap.</li>
                <li>Your IP address, to the Zcash light-wallet server, the NEAR RPC and NEAR Intents, unless you use Tor or a VPN.</li>
              </ul>
            </div>
          </div>
        </section>

        <section id="failures" className="panel panel-light" aria-labelledby="failures-title">
          <SectionHead id="failures" eyebrow="Failures" title={<>If something <em className="serif">goes wrong</em></>}>
            The progress screen names the case and what you can do.
          </SectionHead>
          <ul className="case-cards" data-reveal>
            <Case icon="clock" title="The maker does not answer">
              The swap stops before the premium is paid, and your ZEC goes back to you, less network fees. On testnet the page waits up to 10 minutes for the price.
            </Case>
            <Case icon="return" title="The maker refunds instead of finishing">
              Its refund publishes its key half, and your browser takes your ZEC back. The premium stays with the maker.
            </Case>
            <Case icon="key" title="The maker disappears after your ZEC reached the shared address">
              After a deadline, your browser claims the USDC anyway.
            </Case>
            <Case icon="shield" title="The maker disappears after your claim">
              Your browser moves the USDC to your key in NEAR Intents itself, with the NEAR that the maker sent at the start. On testnet, the last transfer to your NEAR account
              goes through the maker’s relay; while the maker is away, the USDC waits under your key.
            </Case>
            <Case icon="refresh" title="The delivery to the other chain fails">
              The USDC stays in NEAR Intents under your key. You try again, or accept a lower price. Nothing is signed below your minimum unless you accept it.
            </Case>
            <Case icon="file" title="You close the tab, or the browser loses its data">
              The swap is stored in this browser and continues when you come back. The recovery file restores it in any browser. During the claim, keep the tab open for about a
              minute.
            </Case>
          </ul>
        </section>

        <section id="status" className="section status" aria-labelledby="status-title">
          <div className="status-text" data-reveal>
            <SectionHead id="status" eyebrow="Status" title="Project status" align="start">
              {MAINNET_BUILD
                ? 'This site runs on Zcash mainnet and NEAR mainnet with real USDC. The results below are from the test networks. No audit was done.'
                : `Runs on Zcash testnet and NEAR testnet with a test USDC token that has no value. ${MAINNET_STATE}`}
            </SectionHead>
            <ul className="check-list">
              <li>Tested in Chromium only. Firefox and Safari are not tested.</li>
              <li>The swap form offers tokens on {CHAIN_COUNT} chains through NEAR Intents, with live mainnet prices.</li>
            </ul>
            <p className="status-origin">
              Cloakroom Intents is Cloakroom, my earlier protocol for shielded ZEC and ERC-20 tokens on Base, rebuilt on NEAR with NEAR Intents as the exit. Read the <Out href={PAPER_URL}>Cloakroom paper</Out>, the{' '}
              <Out href={DESIGN_URL}>design document</Out> and the <Out href={REPO_URL}>source code</Out>.
            </p>
          </div>
          <ul className="stat-cards" data-reveal>
            <Stat tag="Testnet · website" value="37" title="Swaps through this website" featured>
              With the hosted maker, 2026-11-13 and 14. Most took 6 to 9 minutes. Two ran at the same time, and one that was abandoned on purpose was refunded in 5 minutes.
            </Stat>
            <Stat
              tag="Testnet · no website"
              value={
                <span className="stat-pair" role="img" aria-label="0.1 ZEC to 3 USDC">
                  <TokenAvatar symbol="ZEC" blockchain="zec" size={34} />
                  <span>0.1</span>
                  <Icon name="arrow" size={22} />
                  <TokenAvatar symbol="USDC" blockchain="near" size={34} />
                  <span>3</span>
                </span>
              }
              title="0.1 ZEC became 3 USDC without the website"
            >
              The same swap also runs from the command line: a maker program and a taker program, on the public test networks, with test coins.
            </Stat>
            <Stat tag="Local networks" value="6" title="Browser scenarios pass">
              A full swap, a refund, reloads during a swap, a restore from the recovery file, a price below the minimum, and the maker stopping right after the claim.
            </Stat>
            <Stat tag="Reviews" value="6" title="Security reviews done">
              Each confirmed finding has a fix report. Open issues are in the README. No outside audit was done.
            </Stat>
          </ul>
        </section>

        <section id="faq" className="section faq-section" aria-labelledby="faq-title">
          <SectionHead id="faq" eyebrow="Questions" title="Questions and answers" />
          <div className="faq" data-reveal>
            <Faq q="What if the swap does not finish?">
              You get the USDC, or you get your ZEC back, less network fees. If the maker goes quiet, the page takes the ZEC back by itself. If you close the page, open it again
              in the same browser, or load the recovery file in any browser.
            </Faq>
            <Faq q="What is the recovery file?">
              A file that holds the keys of your swap. With it, any browser can finish the swap or take your ZEC back. It is not encrypted: keep it private, and delete it when
              the swap is done.
            </Faq>
            <Faq q="How long does a swap take?">
              6 to 9 minutes for most swaps on testnet. Most of that is waiting for Zcash blocks: they come every 20 s on average, but some took up to 538 s. Your browser also
              prepares its keys once per visit (about 20 s) and needs about 30 s for each Zcash proof.
            </Faq>
            <Faq q="What does it cost?">
              Four Zcash network fees and the maker’s premium. The swap form shows every amount before you pay. You pay no gas on NEAR: the maker’s stipend covers it.
            </Faq>
            <Faq q="Which wallets can pay?">
              Any wallet that sends shielded ZEC to a unified address.{' '}
              {MAINNET_BUILD
                ? 'Zodl, Zingo, Zkool, Edge and Cake pay into the Ironwood pool per their release notes.'
                : 'On testnet, use testnet ZEC, or let the Fauzec faucet pay 1 test ZEC to the swap’s address.'}{' '}
              In the tests, the payment came from the project’s own command-line wallet. No wallet app has been tested yet.
            </Faq>
            <Faq q="Do I need a NEAR account?">
              {MAINNET_BUILD
                ? 'No. The USDC lands in NEAR Intents under a new key that your browser makes, and NEAR Intents sends the tokens to your address on the chain you chose.'
                : 'On testnet, yes: the test USDC goes to a NEAR testnet account that is registered with the test USDC token. The swap itself uses a new NEAR key that your browser makes, so your account signs nothing.'}
            </Faq>
          </div>
        </section>

        <section className="final on-dark" aria-labelledby="final-title" data-reveal>
          <p className="badge">
            <span className="badge-dot" aria-hidden="true" />
            {MAINNET_BUILD ? 'Mainnet' : 'Testnet'}
          </p>
          <h2 id="final-title" className="section-title">
            Try it on {MAINNET_BUILD ? 'mainnet' : 'testnet'}
          </h2>
          <p className="final-hint">Opening the app downloads the swap engine (about 9 MB). Your browser then prepares its keys for about 20 s.</p>
          <div className="hero-ctas">
            <AppLink className="btn btn-light btn-lg">
              {startLabel} <Icon name="arrow" />
            </AppLink>
            <AppLink href={DEMO_PATH} className="btn btn-glass btn-lg">
              Try the simulated demo
            </AppLink>
          </div>
        </section>
      </main>
      <Footer full />
    </div>
  );
}

function SectionHead({ id, eyebrow, title, align = 'center', children }: { id: string; eyebrow: string; title: ReactNode; align?: 'center' | 'start'; children?: ReactNode }) {
  return (
    <header className={`section-head section-head-${align}`} data-reveal={align === 'center' ? '' : undefined}>
      <p className="badge badge-soft">
        <span className="badge-dot" aria-hidden="true" />
        {eyebrow}
      </p>
      <h2 id={`${id}-title`} className="section-title">
        {title}
      </h2>
      {children && <p className="section-lede">{children}</p>}
    </header>
  );
}

function Step({ n, title, featured, children }: { n: number; title: string; featured?: boolean; children: ReactNode }) {
  return (
    <li className={`step-card${featured ? ' is-featured' : ''}`}>
      <div className="step-card-top">
        <span className="tag">Step</span>
        <span className="step-card-n" aria-hidden="true">
          0{n}
        </span>
      </div>
      <h3 className="card-title">{title}</h3>
      <p>{children}</p>
    </li>
  );
}

function Feature({ icon, title, children }: { icon: IconName; title: string; children: ReactNode }) {
  return (
    <li className="feature">
      <span className="icon-tile">
        <Icon name={icon} />
      </span>
      <div>
        <h3 className="card-title">{title}</h3>
        <p>{children}</p>
      </div>
    </li>
  );
}

function Case({ icon, title, children }: { icon: IconName; title: string; children: ReactNode }) {
  return (
    <li className="case-card">
      <span className="icon-tile">
        <Icon name={icon} />
      </span>
      <h3 className="card-title">{title}</h3>
      <p>{children}</p>
    </li>
  );
}

function Stat({ tag, value, title, featured, children }: { tag: string; value: ReactNode; title: string; featured?: boolean; children: ReactNode }) {
  return (
    <li className={`stat-card${featured ? ' is-featured' : ''}`}>
      <span className="tag">
        <span className="badge-dot" aria-hidden="true" />
        {tag}
      </span>
      <p className="stat-value">{value}</p>
      <h3 className="card-title">{title}</h3>
      <p className="stat-text">{children}</p>
    </li>
  );
}

function Faq({ q, children }: { q: string; children: ReactNode }) {
  return (
    <details className="faq-item">
      <summary>
        <span>{q}</span>
        <span className="faq-toggle" aria-hidden="true">
          <Icon name="plus" size={16} />
        </span>
      </summary>
      <p>{children}</p>
    </details>
  );
}

/** The destination chains as a moving row of names. The names are text, not logos. */
function ChainStrip() {
  return (
    <section className="chains" aria-labelledby="chains-title">
      <h2 id="chains-title" className="chains-title">
        {CHAIN_COUNT} destination chains in the swap form, through NEAR Intents
      </h2>
      <div className="chains-track">
        <ul className="chains-list">
          {DESTINATION_CHAIN_NAMES.map((n) => (
            <li key={n}>
              <ChainIcon name={n} />
              {n}
            </li>
          ))}
        </ul>
        <ul className="chains-list" aria-hidden="true">
          {DESTINATION_CHAIN_NAMES.map((n) => (
            <li key={n}>
              <ChainIcon name={n} />
              {n}
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

/** A drawing of the swap app on a phone with four facts around it. Decorative: every fact is also in the text. */
function HeroVisual() {
  const steps: [string, 'done' | 'now' | 'next'][] = [
    ['Your payment arrives', 'done'],
    ['The maker confirms the price', 'done'],
    ['The maker locks the USDC', 'done'],
    ['Your ZEC goes to the shared address', 'now'],
    ['You claim the USDC', 'next'],
    ['USDC arrives on Base', 'next'],
  ];
  return (
    <div className="hero-visual" aria-hidden="true">
      <div className="hero-arc" />
      <div className="phone appear-phone">
        <div className="phone-screen">
          <div className="phone-notch" />
          <div className="phone-top">
            <Logo />
            <span className="tag">Testnet</span>
          </div>
          <div className="phone-swap">
            <span className="phone-label">You pay</span>
            <span className="phone-amount">0.50 ZEC</span>
            <span className="phone-to">
              <Icon name="arrow" size={14} /> USDC on Base
            </span>
          </div>
          <p className="phone-label phone-steps-label">Steps</p>
          <ol className="phone-steps">
            {steps.map(([t, s]) => (
              <li key={t} className={`is-${s}`}>
                <span className="phone-dot">{s === 'done' && <Icon name="check" size={11} />}</span>
                {t}
              </li>
            ))}
          </ol>
        </div>
      </div>
      <div className="float-card float-a appear-float">
        <span className="icon-tile icon-tile-sm">
          <Icon name="shield" size={16} />
        </span>
        <div>
          <p className="float-title">Shielded payment</p>
          <p className="float-text">Your ZEC stays in the shielded pool</p>
        </div>
      </div>
      <div className="float-card float-b appear-float">
        <span className="icon-tile icon-tile-sm">
          <Icon name="key" size={16} />
        </span>
        <div>
          <p className="float-title">No custodian</p>
          <p className="float-text">You keep half of the key to your ZEC</p>
        </div>
      </div>
      <div className="float-card float-c appear-float">
        <span className="icon-tile icon-tile-sm">
          <Icon name="lock" size={16} />
        </span>
        <div>
          <p className="float-title">The maker pays first</p>
          <p className="float-text">USDC is locked on NEAR before your ZEC moves</p>
        </div>
      </div>
      <div className="float-card float-d appear-float">
        <p className="tag">Destinations</p>
        <p className="float-big">{CHAIN_COUNT}</p>
        <p className="float-text">chains through NEAR Intents</p>
      </div>
    </div>
  );
}

/** Concentric rings around a shield, with the things that stay private. Decorative. */
function PrivacyVisual() {
  return (
    <div className="privacy-visual on-dark" aria-hidden="true" data-reveal>
      <div className="pv-stack">
        <div className="pv-box pv-box-shielded">
          <span className="pv-tag">Shielded · Zcash</span>
          <span className="pv-title">Your Zcash wallet</span>
          <span className="pv-sub">Balance, history and which wallet paid</span>
        </div>
        <div className="pv-link pv-link-broken">
          <span className="pv-line" />
          <span className="pv-pill">
            <Icon name="close" size={12} /> No link on any chain
          </span>
        </div>
        <div className="pv-box">
          <span className="pv-tag">Public · NEAR</span>
          <span className="pv-title">A new key for this swap</span>
          <span className="pv-sub">Escrow, USDC amount and times</span>
        </div>
        <div className="pv-link">
          <span className="pv-line" />
          <span className="pv-note">NEAR Intents sends it on</span>
        </div>
        <div className="pv-box">
          <span className="pv-tag">Public · destination chain</span>
          <span className="pv-title">Your address</span>
          <span className="pv-sub">The amount it receives</span>
        </div>
      </div>
    </div>
  );
}

/** Where the money is during a swap: two lanes, Zcash and NEAR, that the claim joins. */
function FlowDiagram() {
  return (
    <figure className="flow" aria-labelledby="flow-caption">
      <div className="flow-lanes">
        <Lane
          name="Zcash, shielded"
          tone="zcash"
          nodes={[
            { title: 'Your wallet', note: 'You pay once' },
            { title: 'Temporary wallet', note: 'Key in your browser' },
            { title: 'Shared address', note: 'Needs your key half and the maker’s' },
            { title: 'Maker', note: 'Takes the ZEC after your claim' },
          ]}
        />
        <Lane
          name="NEAR"
          tone="near"
          nodes={[
            { title: 'Maker', note: 'Locks USDC first' },
            { title: 'Escrow contract', note: 'No owner, two exits' },
            { title: 'NEAR Intents', note: 'Under your browser’s key' },
            { title: 'Your address', note: 'Token on the chain you chose' },
          ]}
        />
      </div>
      <figcaption id="flow-caption" className="flow-caption">
        The maker locks the USDC before your ZEC leaves the temporary wallet. Your claim publishes your key half: you get the USDC, and only then can the maker take the ZEC.
      </figcaption>
    </figure>
  );
}

function Lane({ name, tone, nodes }: { name: string; tone: 'zcash' | 'near'; nodes: { title: string; note: string }[] }) {
  return (
    <div className={`flow-lane flow-${tone}`}>
      <p className="flow-lane-name">{name}</p>
      <ol className="flow-nodes">
        {nodes.map((n) => (
          <li key={n.title} className="flow-node">
            <span className="flow-node-title">{n.title}</span>
            <span className="flow-node-note">{n.note}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}
