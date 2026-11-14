// The landing page at /. It says what Zecret does, who holds the money, what is private,
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
import { MAINNET_BUILD } from '../lib/netconfig';
import { Icon, type IconName } from '../ui/Icon';
import { Logo } from '../ui/bits';
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
    document.title = 'Zecret: swap shielded ZEC with no custodian';
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
              Pay once from your own Zcash wallet. Your browser runs the swap with the maker, and NEAR Intents sends the tokens on to the chain you choose. No company or server
              holds your money or your keys.
            </p>
            <p className="hero-note appear">
              {MAINNET_BUILD
                ? 'This site runs on Zcash mainnet and NEAR mainnet with real money. It is hackathon software with no security audit. The maker takes only small swaps. Pay only what you can afford to lose.'
                : 'This runs on Zcash testnet and NEAR testnet today, with test money. Mainnet is not deployed. On testnet the tokens arrive in your NEAR testnet account, and the price shown for other chains is a mainnet preview.'}
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
          <p className="marquee-words" aria-hidden="true">
            <span>
              Shielded ZEC <i>·</i> Escrow on NEAR <i>·</i> USDC <i>·</i> 30+ chains <i>·</i> Shielded ZEC <i>·</i> Escrow on NEAR <i>·</i> USDC <i>·</i> 30+ chains <i>·</i>
            </span>
          </p>
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
            <SectionHead id="status" eyebrow="Status" title={<>Project <em className="serif">status</em></>} align="start">
              {MAINNET_BUILD
                ? 'This site runs on Zcash mainnet and NEAR mainnet with real USDC. The results below are from the test networks. No audit was done.'
                : 'Runs on Zcash testnet and NEAR testnet with a test USDC token that has no value. Mainnet is not deployed.'}
            </SectionHead>
            <ul className="check-list">
              <li>Tested in Chromium only. Firefox and Safari are not tested.</li>
              <li>The swap form offers tokens on {CHAIN_COUNT} chains through NEAR Intents, with live mainnet prices.</li>
            </ul>
            <p className="status-origin">
              Zecret builds on Cloakroom, my earlier protocol for shielded ZEC and ERC-20 tokens on Base. Read the <Out href={PAPER_URL}>Cloakroom paper</Out>, the{' '}
              <Out href={DESIGN_URL}>design document</Out> and the <Out href={REPO_URL}>source code</Out>.
            </p>
          </div>
          <ul className="stat-cards" data-reveal>
            <Stat tag="Testnet · website" value="5" title="Swaps through this website" featured>
              With the hosted maker, 2026-11-13 and 14. For example: 0.05 testnet ZEC for 1.4877 test USDC in 493 s, 0.01 for 0.2877 in 392 s, and one paid by the faucet with no wallet.
            </Stat>
            <Stat tag="Testnet · command line" value="0.1 → 3" title="One swap with the command-line programs">
              0.1 testnet ZEC for 3 test USDC, end to end on the public test networks.
            </Stat>
            <Stat tag="Local networks" value="5" title="Browser scenarios pass">
              A full swap, a refund, reloads during a swap, a restore from the recovery file and a price below the minimum.
            </Stat>
            <Stat tag="Reviews" value="3" title="Security reviews done">
              A fourth read of the source by a second reviewer found nothing new. Open issues are in the README. One of them: the browser trusts a single NEAR RPC endpoint.
            </Stat>
          </ul>
        </section>

        <section id="faq" className="section faq-section" aria-labelledby="faq-title">
          <SectionHead id="faq" eyebrow="Questions" title={<>Questions and <em className="serif">answers</em></>} />
          <div className="faq" data-reveal>
            <Faq q="Which wallets can pay?">
              A wallet that can send shielded ZEC to a unified address on the same network. On testnet, use testnet ZEC. In the tests, the payment came from the project’s own
              command-line wallet. No wallet app has been tested yet.{' '}
              {MAINNET_BUILD ? 'Zodl, Zingo, Zkool, Edge and Cake pay into the Ironwood pool per their release notes.' : 'On testnet, the Fauzec faucet can also pay 1 test ZEC to the swap’s address.'}
            </Faq>
            <Faq q="How long does a swap take?">
              The one swap on the public test networks took 222 s from the start to the USDC, and 322 s with the exit (2026-11-13, command-line programs). Zcash testnet blocks come
              every 20 s on average, but some took up to 538 s. In the browser, the engine prepares its keys once per visit (about 20 s), and each Zcash proof takes about 30 s
              (measured in Chromium on local test networks).
            </Faq>
            <Faq q="What is the recovery file?">
              A file with the keys of your swap: the temporary wallet key, your key half, the NEAR key and the swap settings. With it, any browser can finish the swap or take your ZEC
              back. It is not encrypted: keep it private, and delete it when the swap is done.
            </Faq>
            <Faq q="What does it cost?">
              You pay four Zcash network fees (three sends from the temporary wallet, and one spend from the shared address) and the maker’s premium. The maker’s rate and the NEAR
              Intents step set how much you receive. The swap form shows each amount before you pay. The maker’s stipend pays your gas on NEAR.
            </Faq>
            <Faq q="Do I need a NEAR account?">
              {MAINNET_BUILD
                ? 'No. The USDC lands in NEAR Intents under a new key that your browser makes, and NEAR Intents sends the tokens to your address on the chain you chose.'
                : 'On testnet, yes: the tokens go to a NEAR testnet account that is registered with the test USDC token. The swap itself uses a new NEAR key that your browser makes.'}
            </Faq>
          </div>
        </section>

        <section className="final on-dark" aria-labelledby="final-title" data-reveal>
          <p className="badge">
            <span className="badge-dot" aria-hidden="true" />
            {MAINNET_BUILD ? 'Mainnet' : 'Testnet'}
          </p>
          <h2 id="final-title" className="section-title">
            Try it on <em className="serif">{MAINNET_BUILD ? 'mainnet' : 'testnet'}</em>
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

function Stat({ tag, value, title, featured, children }: { tag: string; value: string; title: string; featured?: boolean; children: ReactNode }) {
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
            <li key={n}>{n}</li>
          ))}
        </ul>
        <ul className="chains-list" aria-hidden="true">
          {DESTINATION_CHAIN_NAMES.map((n) => (
            <li key={n}>{n}</li>
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
        <p className="tag">Key halves</p>
        <p className="float-big">2</p>
        <p className="float-text">One half alone cannot spend</p>
      </div>
      <div className="float-card float-c appear-float">
        <p className="tag">Escrow on NEAR</p>
        <p className="float-title">No owner, no upgrade</p>
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
      <div className="ring ring-3" />
      <div className="ring ring-2" />
      <div className="ring ring-1" />
      <div className="privacy-core">
        <Icon name="shield" size={40} />
      </div>
      <span className="ring-chip ring-chip-a">
        <Icon name="wallet" size={14} /> Your wallet
      </span>
      <span className="ring-chip ring-chip-b">
        <Icon name="lock" size={14} /> Encrypted memos
      </span>
      <span className="ring-chip ring-chip-c">
        <Icon name="key" size={14} /> New NEAR key
      </span>
      <span className="ring-chip ring-chip-d">
        <Icon name="eye" size={14} /> No link back
      </span>
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
