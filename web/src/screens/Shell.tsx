import { useEffect, useRef, useState } from 'react';
import type { InitStatus, SwapState } from '../engine';
import { appStore, usePref } from '../app/useZecret';
import { FAILURES, SPEEDS, settingsUrl, type PageSettings } from '../app/settings';
import { Icon } from '../ui/Icon';
import { Logo, Notice } from '../ui/bits';

type Theme = 'system' | 'light' | 'dark';

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

export function Header({ init, network, onHome }: { init?: InitStatus; network?: string; onHome: () => void }) {
  const [theme, toggle] = useTheme();
  const systemDark = typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: dark)').matches;
  const dark = theme === 'dark' || (theme === 'system' && systemDark);
  return (
    <header className="topbar">
      <div className="topbar-inner">
        <a
          href="./"
          className="home-link"
          onClick={(e) => {
            e.preventDefault();
            onHome();
          }}
          aria-label="Zecret, home"
        >
          <Logo />
        </a>
        <div className="topbar-right">
          {network && <span className={`pill pill-${network === 'mainnet' ? 'main' : 'test'}`}>{network === 'mainnet' ? 'Mainnet' : `Zcash ${network}`}</span>}
          {init?.engine === 'simulated' && <span className="pill pill-demo">Demo</span>}
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
      <p>
        Zecret runs in your browser. It holds the swap’s keys here, never on a server. The maker is the other side of the trade and never holds your ZEC alone.
      </p>
      <p className="muted">Prices for the delivery come from NEAR Intents. Amounts round down.</p>
    </footer>
  );
}

export function DemoBanner({ settings, init, swap }: { settings: PageSettings; init?: InitStatus; swap: SwapState | null }) {
  if (init?.engine !== 'simulated') return null;
  const speed = settings.sim.speed;
  const fail = FAILURES.find((f) => f.id === (settings.sim.fail ?? settings.pageFailure));
  return (
    <div className="demo-banner" role="note">
      <Icon name="info" size={16} />
      <span>
        Demo with a simulated engine: no real funds move.
        {speed === 1 ? ' Real-time: a swap takes about 10 minutes.' : ` Running ${speed}× faster than real time.`}
        {fail ? ` Failure on: ${fail.label.toLowerCase()}.` : ''}
      </span>
      {speed === 1 && (
        <a className="demo-banner-link" href={settingsUrl({ speed: 20 })}>
          Run 20× faster
        </a>
      )}
      <DevPanel settings={settings} swap={swap} />
    </div>
  );
}

/** Controls for the simulated engine: speed, failures, skipping a wait, a cleared browser. */
export function DevPanel({ settings, swap }: { settings: PageSettings; swap: SwapState | null }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [msg, setMsg] = useState<string>();
  const current = settings.sim.fail ?? settings.pageFailure ?? '';
  const act = async (fn: () => Promise<unknown>, done: string) => {
    try {
      await fn();
      setMsg(done);
    } catch (e) {
      setMsg((e as Error).message);
    }
  };
  return (
    <>
      <button
        type="button"
        className="btn btn-secondary btn-sm devpanel-toggle"
        onClick={() => {
          setMsg(undefined);
          dialog.current?.showModal();
        }}
        aria-haspopup="dialog"
      >
        <Icon name="settings" size={16} /> Demo controls
      </button>
      <dialog ref={dialog} className="dialog devpanel" aria-labelledby="devpanel-title" onClick={(e) => e.target === dialog.current && dialog.current?.close()}>
        <div className="dialog-head">
          <h2 id="devpanel-title" className="dialog-title">
            Demo controls
          </h2>
          <button type="button" className="btn btn-ghost btn-icon" onClick={() => dialog.current?.close()} aria-label="Close">
            <Icon name="close" />
          </button>
        </div>
        <div className="devpanel-body">
          <p className="hint">These change the simulated engine only. Speed and failure apply after the page reloads; a failure is fixed when a swap starts.</p>
          <label className="label" htmlFor="dev-speed">
            Speed
          </label>
          <select id="dev-speed" className="input" value={settings.sim.speed} onChange={(e) => location.assign(settingsUrl({ speed: Number(e.target.value) }))}>
            {[...new Set([...SPEEDS, settings.sim.speed])].map((s) => (
              <option key={s} value={s}>
                {s === 1 ? 'Real time (about 10 min)' : `${s}× faster`}
              </option>
            ))}
          </select>
          <label className="label" htmlFor="dev-fail">
            Failure for the next swap
          </label>
          <select id="dev-fail" className="input" value={current} onChange={(e) => location.assign(settingsUrl({ fail: e.target.value || null }))}>
            <option value="">None: the normal path</option>
            {FAILURES.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
          <div className="devpanel-actions">
            <button type="button" className="btn btn-secondary btn-sm" disabled={!swap} onClick={() => act(() => appStore.engine!.sim.skip(), 'Skipped to the end of the current wait.')}>
              Skip this wait
            </button>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={!swap}
              onClick={() =>
                act(async () => {
                  await appStore.engine!.sim.clearLocal();
                  location.reload();
                }, 'Cleared.')
              }
            >
              Simulate a cleared browser
            </button>
          </div>
          {msg && (
            <Notice tone="neutral" role="status">
              {msg}
            </Notice>
          )}
        </div>
      </dialog>
    </>
  );
}
