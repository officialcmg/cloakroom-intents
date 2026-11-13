// The landing page at /. It says what Zecret does, who holds the money, what is private,
// what happens on a failure and how far the project is. It loads no engine: the swap app
// at /swap starts the engine when the user opens it.
//
// Every claim here is backed by README.md, docs/DESIGN.md ("The design in plain words"),
// docs/TESTNET.md, harness/BROWSER_RESULTS.md, STATUS.md or public/config/testnet.json.
// Change the text only together with those sources.
import { useEffect, type ReactNode } from 'react';
import { APP_PATH, hasMarkedSwap, onLinkClick } from '../app/route';
import { Icon, type IconName } from '../ui/Icon';
import { DESIGN_URL, Footer, Header, PAPER_URL, REPO_URL } from './Chrome';

const DEMO_PATH = `${APP_PATH}?demo=1`;

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
  const resume = hasMarkedSwap();
  return (
    <div className="app">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <Header>
        <AppLink className="btn btn-secondary btn-sm topbar-cta">{resume ? 'Open your swap' : 'Start a swap'}</AppLink>
      </Header>
      <main id="main" className="main main-landing">
        {resume && (
          <div className="notice notice-info landing-resume" role="status">
            <Icon name="clock" className="notice-icon" />
            <div className="notice-body">
              <p className="notice-title">This browser holds a swap</p>
              <div className="notice-text">
                <p>Open it to see where it is. The swap continues while the app is open.</p>
                <AppLink className="btn btn-primary btn-sm">
                  Open your swap <Icon name="arrow" size={16} />
                </AppLink>
              </div>
            </div>
          </div>
        )}

        <header className="landing-hero">
          <p className="landing-kicker">
            <span className="pill pill-test">Testnet</span> Zcash testnet and NEAR testnet, test tokens only
          </p>
          <h1 className="display landing-title">Swap shielded ZEC for USDC, with no custodian</h1>
          <p className="lede landing-lede">
            Zecret swaps shielded ZEC for USDC on NEAR. Your browser runs the swap with the maker, and NEAR Intents can then send the USDC on as a token on another chain. No server holds
            your keys.
          </p>
          <div className="landing-ctas">
            <AppLink className="btn btn-primary btn-lg">
              {resume ? 'Open your swap' : 'Start a swap'} <Icon name="arrow" />
            </AppLink>
            <AppLink href={DEMO_PATH} className="btn btn-secondary btn-lg">
              Try the simulated demo
            </AppLink>
          </div>
          <p className="hint">The demo moves no money and needs no wallet. Mainnet is not deployed.</p>
        </header>

        <Section id="steps" title="How a swap works">
          <ol className="landing-steps">
            <Step n={1} title="Choose the amount and the destination">
              Enter how much ZEC you pay and where the tokens go. The page shows one price: the USDC the maker pays, and what NEAR Intents delivers for it.
            </Step>
            <Step n={2} title="Pay once from your own Zcash wallet">
              Save the recovery file. Then pay the exact amount to the shielded address on the screen, by QR code or by copying it.
            </Step>
            <Step n={3} title="Your browser does the rest">
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
        </Section>

        <Section id="custody" title="Who holds the money">
          <p className="landing-intro">
            No company or server holds your money at any step. Your ZEC is under your browser’s key, then at an address that needs two key halves. The maker’s USDC is in an escrow
            contract, then in NEAR Intents under your browser’s key.
          </p>
          <FlowDiagram />
          <ul className="landing-facts">
            <Fact icon="key" title="Two key halves">
              The shared Zcash address is spent with two key halves. Your browser holds one, the maker holds the other. One half alone cannot spend.
            </Fact>
            <Fact icon="lock" title="The escrow has two exits">
              Your claim publishes your key half: you get the USDC, and the maker can take the ZEC. The maker’s refund publishes its key half: you take your ZEC back.
            </Fact>
            <Fact icon="shield" title="No owner, no upgrade">
              The escrow contract on NEAR has no owner, no upgrade method and no access keys. Nobody can change its code.
            </Fact>
          </ul>
        </Section>

        <Section id="privacy" title="What stays private and what is public">
          <div className="privacy landing-privacy">
            <div>
              <h3 className="h3">
                <Icon name="shield" size={16} /> Private
              </h3>
              <ul className="plain-list">
                <li>Your Zcash wallet, its balance and its history. The ZEC stays in the shielded pool for the whole swap.</li>
                <li>Which Zcash wallet paid. Nothing on NEAR or on the destination chain points back to it.</li>
                <li>The messages between your browser and the maker. They are encrypted memos in shielded Zcash transactions.</li>
                <li>
                  Your NEAR gas. The swap uses a new NEAR key. Its gas comes from 0.03 NEAR that the maker sends to that key (the stipend), so no account of yours pays gas on NEAR.
                </li>
              </ul>
            </div>
            <div>
              <h3 className="h3">
                <Icon name="eye" size={16} /> Public
              </h3>
              <ul className="plain-list">
                <li>On NEAR: the escrow, the USDC amount, the time of each step and the new key.</li>
                <li>On the destination chain: the amount and the address that receives it.</li>
                <li>The maker knows that a swap of this size happened, and when. It does not learn your Zcash wallet.</li>
                <li>Timing. The website adds no delay. When few swaps run, the times of the steps can narrow down which payment belongs to which swap.</li>
                <li>Your IP address, to the Zcash light-wallet server, the NEAR RPC and NEAR Intents, unless you use Tor or a VPN.</li>
              </ul>
            </div>
          </div>
        </Section>

        <Section id="failures" title="If something goes wrong">
          <dl className="landing-cases">
            <Case title="The maker does not answer">
              The swap stops before the premium is paid, and your ZEC goes back to you, less network fees. On testnet the page waits up to 10 minutes for the price.
            </Case>
            <Case title="The maker refunds instead of finishing">
              Its refund publishes its key half, and your browser takes your ZEC back. The premium stays with the maker.
            </Case>
            <Case title="The maker disappears after your ZEC reached the shared address">After a deadline, your browser claims the USDC anyway.</Case>
            <Case title="The delivery to the other chain fails">
              The USDC stays in NEAR Intents under your key. You try again, or accept a lower price. Nothing is signed below your minimum unless you accept it.
            </Case>
            <Case title="You close the tab, or the browser loses its data">
              The swap is stored in this browser and continues when you come back. The recovery file restores it in any browser. During the claim, keep the tab open for about a minute.
            </Case>
          </dl>
        </Section>

        <Section id="status" title="Status">
          <ul className="plain-list landing-status">
            <li>Runs on Zcash testnet and NEAR testnet with a test USDC token that has no value. Mainnet is not deployed.</li>
            <li>
              Two swaps ran end to end on the public test networks. One used the command-line programs: 0.1 testnet ZEC for 3 test USDC. One went through this website with
              the hosted maker: 0.05 testnet ZEC for 1.4877 test USDC, 493 s from the payment to the end (2026-11-13).
            </li>
            <li>In the browser, five scenarios pass on local test networks: a full swap, a refund, reloads during a swap, a restore from the recovery file and a price below the minimum.</li>
            <li>Tested in Chromium only. Firefox and Safari are not tested.</li>
            <li>
              Three security reviews are done. Open issues are listed in the README. One of them: the browser trusts a single NEAR RPC endpoint.
            </li>
          </ul>
          <p className="landing-origin">
            Zecret builds on Cloakroom, my earlier protocol for shielded ZEC and ERC-20 tokens on Base. Read the <Out href={PAPER_URL}>Cloakroom paper</Out>, the{' '}
            <Out href={DESIGN_URL}>design document</Out> and the <Out href={REPO_URL}>source code</Out>.
          </p>
        </Section>

        <Section id="faq" title="Questions">
          <div className="faq">
            <Faq q="Which wallets can pay?">
              A wallet that can send shielded ZEC to a unified address on the same network. On testnet, use testnet ZEC. In the tests, the payment came from the project’s own
              command-line wallet. No wallet app has been tested yet.
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
              You pay four Zcash network fees (three sends from the temporary wallet, and one spend from the shared address) and the maker’s premium. The maker’s rate and the NEAR Intents step set how much you receive. The swap form shows each amount before
              you pay. The maker’s stipend pays your gas on NEAR.
            </Faq>
            <Faq q="Do I need a NEAR account?">
              On testnet, yes: the tokens go to a NEAR testnet account that is registered with the test USDC token. The swap itself uses a new NEAR key that your browser makes.
            </Faq>
          </div>
        </Section>

        <section className="landing-final" aria-labelledby="final-title">
          <h2 id="final-title" className="h2">
            Try it on testnet
          </h2>
          <p className="hint">Opening the app downloads the swap engine (about 9 MB). Your browser then prepares its keys for about 20 s.</p>
          <div className="landing-ctas">
            <AppLink className="btn btn-primary btn-lg">
              {resume ? 'Open your swap' : 'Start a swap'} <Icon name="arrow" />
            </AppLink>
            <AppLink href={DEMO_PATH} className="btn btn-secondary btn-lg">
              Try the simulated demo
            </AppLink>
          </div>
        </section>
      </main>
      <Footer />
    </div>
  );
}

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section className="landing-section" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="landing-h2">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Step({ n, title, children }: { n: number; title: string; children: ReactNode }) {
  return (
    <li className="landing-step">
      <span className="landing-step-n" aria-hidden="true">
        {n}
      </span>
      <div>
        <h3 className="h3">{title}</h3>
        <p>{children}</p>
      </div>
    </li>
  );
}

function Fact({ icon, title, children }: { icon: IconName; title: string; children: ReactNode }) {
  return (
    <li className="how-item">
      <span className="how-icon">
        <Icon name={icon} />
      </span>
      <div>
        <h3 className="h3">{title}</h3>
        <p>{children}</p>
      </div>
    </li>
  );
}

function Case({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="landing-case">
      <dt>{title}</dt>
      <dd>{children}</dd>
    </div>
  );
}

function Faq({ q, children }: { q: string; children: ReactNode }) {
  return (
    <details className="faq-item">
      <summary>
        <span>{q}</span>
        <Icon name="chevron" size={18} className="faq-chevron" />
      </summary>
      <p>{children}</p>
    </details>
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
      <figcaption id="flow-caption" className="hint">
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
