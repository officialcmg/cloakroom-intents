// The page frame shared by the landing page and the swap app: header, footer, theme and
// the one-line banner with a details disclosure. No engine import, so the landing page
// stays small and starts nothing.
import { useEffect, useId, useState, type ReactNode } from 'react';
import type { InitStatus } from '../engine/api';
import { usePref } from '../app/prefs';
import { onLinkClick } from '../app/route';
import { Icon } from '../ui/Icon';
import { Logo } from '../ui/bits';

type Theme = 'system' | 'light' | 'dark';

export const REPO_URL = 'https://github.com/officialcmg/zecret';
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

export function Header({ init, network, children }: { init?: InitStatus; network?: string; children?: ReactNode }) {
  const [theme, toggle] = useTheme();
  const systemDark = typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: dark)').matches;
  const dark = theme === 'dark' || (theme === 'system' && systemDark);
  return (
    <header className="topbar">
      <div className="topbar-inner">
        <a href="/" className="home-link" onClick={onLinkClick} aria-label="Zecret, home">
          <Logo />
        </a>
        <div className="topbar-right">
          {network && <span className={`pill pill-${network === 'mainnet' ? 'main' : 'test'}`}>{network === 'mainnet' ? 'Mainnet' : `Zcash ${network}`}</span>}
          {init?.engine === 'simulated' && <span className="pill pill-demo">Demo</span>}
          {children}
          <button type="button" className="btn btn-ghost btn-icon" onClick={toggle} aria-label={dark ? 'Use light theme' : 'Use dark theme'} title={dark ? 'Light theme' : 'Dark theme'}>
            <Icon name={dark ? 'sun' : 'moon'} />
          </button>
        </div>
      </div>
    </header>
  );
}

export function Footer() {
  return (
    <footer className="footer">
      <p>Zecret runs in your browser. The swap’s keys stay in this browser, never on a server. The maker never holds your ZEC alone.</p>
      <p className="muted">Delivery prices come from NEAR Intents. Amounts round down.</p>
      <nav aria-label="About Zecret" className="footer-links">
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
