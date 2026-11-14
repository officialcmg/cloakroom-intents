// The page frame shared by the landing page and the swap app: header, footer, theme and
// the one-line banner with a details disclosure. No engine import, so the landing page
// stays small and starts nothing.
import { useEffect, useId, useState, type ReactNode } from 'react';
import type { InitStatus } from '../engine/api';
import { usePref } from '../app/prefs';
import { onLinkClick } from '../app/route';
import { Icon } from '../ui/Icon';
import { Logo } from '../ui/bits';
import { MAINNET_BUILD, NETWORK_SITES } from '../lib/netconfig';

type Theme = 'system' | 'light' | 'dark';

export const REPO_URL = 'https://github.com/officialcmg/cloakroom-intents';
export const DESIGN_URL = `${REPO_URL}/blob/main/docs/DESIGN.md`;
export const PAPER_URL = `${REPO_URL}/blob/main/Cloakroom_Yellow_Paper.pdf`;

export function useTheme(): [Theme, () => void] {
  const [theme, setTheme] = usePref<Theme>('theme', 'system');
  useEffect(() => {
    const root = document.documentElement;
    if (theme === 'system') root.removeAttribute('data-theme');
    else root.setAttribute('data-theme', theme);
  }, [theme]);
  const systemDark = typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: dark)').matches;
  const effective = theme === 'system' ? (systemDark ? 'dark' : 'light') : theme;
  return [theme, () => setTheme(effective === 'dark' ? 'light' : 'dark')];
}

const SECTIONS: [string, string][] = [
  ['#steps', 'How it works'],
  ['#custody', 'Custody'],
  ['#privacy', 'Privacy'],
  ['#faq', 'Questions'],
];

/**
 * Testnet | Mainnet. Each build serves one network, so this is navigation: the other side
 * is a plain link to the other deployment (VITE_ZECRET_TESTNET_URL, VITE_ZECRET_MAINNET_URL),
 * at `path`, with no query. When the other deployment is not set, that side is shown off,
 * with the reason.
 */
export function NetworkSwitch({ current, path, sites = NETWORK_SITES }: { current: 'testnet' | 'mainnet'; path: string; sites?: { testnet?: string; mainnet?: string } }) {
  const items: ['testnet' | 'mainnet', string, string][] = [
    ['testnet', 'Testnet', 'Test'],
    ['mainnet', 'Mainnet', 'Main'],
  ];
  return (
    <nav className="net-switch" aria-label="Network" data-testid="network-switch">
      {items.map(([net, label, short]) => {
        const text = (
          <>
            <span className="net-long">{label}</span>
            <span className="net-short" aria-hidden="true">
              {short}
            </span>
          </>
        );
        if (net === current)
          return (
            <a key={net} href={path} className={`net-item is-current net-${net}`} aria-current="page" aria-label={label} onClick={onLinkClick}>
              {text}
            </a>
          );
        const site = sites[net];
        if (!site)
          return (
            <span key={net} className="net-item is-off" title={`${label} is not deployed yet`} data-testid={`net-${net}-off`}>
              <span aria-hidden="true">{text}</span>
              <span className="sr-only">{label} is not deployed yet</span>
            </span>
          );
        return (
          <a key={net} href={`${site}${path}`} className={`net-item net-${net}`} aria-label={`${label} (opens the ${label.toLowerCase()} site)`} rel="noopener">
            {text}
          </a>
        );
      })}
    </nav>
  );
}

export function Header({ init, network, sections, children }: { init?: InitStatus; network?: string; sections?: boolean; children?: ReactNode }) {
  // The landing page has no network loaded: the build decides (MAINNET_BUILD).
  const current = network === 'mainnet' || (!network && MAINNET_BUILD) ? 'mainnet' : network === undefined || network === 'testnet' ? 'testnet' : undefined;
  const path = sections ? '/' : '/swap';
  const [theme, toggle] = useTheme();
  const systemDark = typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: dark)').matches;
  const dark = theme === 'dark' || (theme === 'system' && systemDark);
  return (
    <header className="topbar">
      <div className="topbar-inner">
        <a href="/" className="home-link" onClick={onLinkClick} aria-label="Cloakroom Intents, home">
          <Logo />
        </a>
        {sections && (
          <nav className="topbar-nav" aria-label="Sections of this page">
            {SECTIONS.map(([href, label]) => (
              <a key={href} href={href}>
                {label}
              </a>
            ))}
          </nav>
        )}
        <div className="topbar-right">
          {/* The switch belongs to the app. The landing page is the same for both networks' visitors. */}
          {current && !sections ? (
            <NetworkSwitch current={current} path={path} />
          ) : (
            network && (
              <span className="pill pill-test">
                <span className="pill-long">Zcash&nbsp;</span>
                {network}
              </span>
            )
          )}
          {init?.engine === 'simulated' && <span className="pill pill-demo">Demo</span>}
          <button type="button" className="btn btn-ghost btn-icon theme-toggle" onClick={toggle} aria-label={dark ? 'Use light theme' : 'Use dark theme'} title={dark ? 'Light theme' : 'Dark theme'}>
            <Icon name={dark ? 'sun' : 'moon'} />
          </button>
          {children}
        </div>
      </div>
    </header>
  );
}

const SITE_URL = 'https://cloakroom.chrismg.dev';

export function Footer({ full }: { full?: boolean }) {
  if (!full) {
    return (
      <footer className="footer">
        <p>Cloakroom Intents runs in your browser. The swap’s keys stay in this browser, never on a server. The maker never holds your ZEC alone.</p>
        <p className="muted">Delivery prices come from NEAR Intents. Amounts round down.</p>
        <nav aria-label="About Cloakroom Intents" className="footer-links">
          <a href="/" onClick={onLinkClick}>
            How it works
          </a>
          <a href={REPO_URL} rel="noopener noreferrer">
            Source code
          </a>
          <a href={DESIGN_URL} rel="noopener noreferrer">
            Design
          </a>
        </nav>
      </footer>
    );
  }
  return (
    <footer className="site-footer">
      <div className="site-footer-card">
        <div className="site-footer-grid">
          <div className="site-footer-about">
            <Logo />
            <p className="site-footer-tagline">Shielded ZEC to tokens on other chains, with no custodian.</p>
            <p>Cloakroom Intents runs in your browser. The swap’s keys stay in this browser, never on a server. The maker never holds your ZEC alone.</p>
          </div>
          <nav className="site-footer-col" aria-labelledby="footer-use">
            <p id="footer-use" className="site-footer-head">
              Use
            </p>
            <a href="/swap" onClick={onLinkClick}>
              Start a swap
            </a>
            <a href="/swap?demo=1" onClick={onLinkClick}>
              Simulated demo
            </a>
            <a href="#steps">How it works</a>
            <a href="#faq">Questions</a>
          </nav>
          <nav className="site-footer-col" aria-labelledby="footer-read">
            <p id="footer-read" className="site-footer-head">
              Read
            </p>
            <a href={DESIGN_URL} rel="noopener noreferrer">
              Design
            </a>
            <a href={PAPER_URL} rel="noopener noreferrer">
              Cloakroom paper
            </a>
            <a href={REPO_URL} rel="noopener noreferrer">
              Source code
            </a>
          </nav>
        </div>
        <div className="site-footer-note">
          <p>
            {MAINNET_BUILD ? 'Mainnet: real money, unaudited software.' : NETWORK_SITES.mainnet ? 'Testnet: test money only.' : 'Testnet only. Mainnet is not deployed.'} Delivery prices come from
            NEAR Intents. Amounts round down.
          </p>
          <a href={SITE_URL}>cloakroom.chrismg.dev</a>
        </div>
      </div>
    </footer>
  );
}

/**
 * One short line, and the full facts behind a "Details" button. The facts stay on the
 * page for anyone who opens them; the line does not crowd a phone screen.
 */
export function Banner({ lead, children, testId, tone = 'note' }: { lead: ReactNode; children: ReactNode; testId?: string; tone?: 'note' | 'demo' }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <div className={`site-banner site-banner-${tone}`} role="note" data-testid={testId}>
      <div className="site-banner-row">
        <Icon name="info" size={16} className="site-banner-icon" />
        <p className="site-banner-lead">{lead}</p>
        <button type="button" className="site-banner-toggle" aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)}>
          Details
          <Icon name="chevron" size={16} className="site-banner-chevron" />
        </button>
      </div>
      <div id={id} className="site-banner-more" hidden={!open}>
        {children}
      </div>
    </div>
  );
}
