import { useEffect, useRef, useState } from 'react';
import type { SwapState } from './engine';
import { appStore, useApp } from './app/useZecret';
import { useExit } from './app/useExit';
import { groupsFor, nowCopy } from './app/stageCopy';
import { Icon } from './ui/Icon';
import { Notice, Spinner } from './ui/bits';
import { SwapForm } from './screens/SwapForm';
import { PayScreen, RecoveryStep } from './screens/Pay';
import { ProgressScreen } from './screens/Progress';
import { DoneScreen, RefundedScreen } from './screens/Finished';
import { RestoreScreen } from './screens/Restore';
import { ConfigOverrideBanner, DemoBanner, EngineWarnings, Footer, Header, NetworkBanner } from './screens/Shell';

type View = 'auto' | 'restore' | 'recovery';

/** Stages in which closing the tab can cost the swap: from the commit until the claim is final. */
const KEEP_OPEN = new Set(['ready', 'committing', 'claiming']);

function useDocumentTitle(swap: SwapState | null) {
  useEffect(() => {
    if (!swap) {
      document.title = 'Zecret: swap shielded ZEC';
      return;
    }
    const groups = groupsFor(swap);
    const i = groups.findIndex((g) => g.stages.includes(swap.stage));
    const step = i >= 0 && swap.stage !== 'done' && swap.stage !== 'refunded' ? `(${i + 1}/${groups.length}) ` : '';
    document.title = `${step}${nowCopy(swap).title} · Zecret`;
  }, [swap]);
}

function useLeaveGuard(swap: SwapState | null) {
  const stage = swap?.stage;
  useEffect(() => {
    if (!stage || !KEEP_OPEN.has(stage)) return;
    const onLeave = (e: BeforeUnloadEvent) => {
      e.preventDefault();
      e.returnValue = '';
    };
    window.addEventListener('beforeunload', onLeave);
    return () => window.removeEventListener('beforeunload', onLeave);
  }, [stage]);
}

/** Keeps the screen awake while a swap runs, where the browser allows it. */
function useWakeLock(active: boolean) {
  useEffect(() => {
    if (!active || !('wakeLock' in navigator)) return;
    let lock: WakeLockSentinel | undefined;
    let cancelled = false;
    const take = () =>
      navigator.wakeLock.request('screen').then(
        (l) => {
          if (cancelled) void l.release();
          else lock = l;
        },
        () => undefined,
      );
    void take();
    const onVisible = () => document.visibilityState === 'visible' && void take();
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      cancelled = true;
      document.removeEventListener('visibilitychange', onVisible);
      void lock?.release();
    };
  }, [active]);
}

export default function App() {
  const app = useApp();
  const [view, setView] = useState<View>('auto');
  const swap = app.swap;
  const exit = useExit(appStore.engine, swap);
  const main = useRef<HTMLElement>(null);

  useDocumentTitle(swap);
  useLeaveGuard(swap);
  useWakeLock(!!swap && swap.stage !== 'awaiting_deposit' && swap.stage !== 'done' && swap.stage !== 'refunded');

  // Move focus to the new screen's heading when the screen changes, for keyboard and screen-reader users.
  const screenKey = screenFor(app.phase, view, swap);
  const firstRender = useRef(true);
  useEffect(() => {
    if (firstRender.current) {
      firstRender.current = false;
      return;
    }
    const h = main.current?.querySelector('h1');
    if (h) {
      h.setAttribute('tabindex', '-1');
      h.focus({ preventScroll: false });
    }
    window.scrollTo({ top: 0 });
  }, [screenKey]);


  let content: React.ReactNode;
  if (app.phase === 'loading') {
    content = (
      <div className="loading" role="status">
        <Spinner />
        <p>Starting Zecret in this browser…</p>
      </div>
    );
  } else if (app.phase === 'other_tab') {
    content = (
      <div className="stack-lg narrow">
        <section className="card card-main stack" aria-labelledby="other-tab-title" data-testid="other-tab">
          <span className="seal-icon" aria-hidden="true">
            <Icon name="lock" size={22} />
          </span>
          <h1 id="other-tab-title" className="h1">
            This swap is open in another tab
          </h1>
          <p>Zecret runs a swap in one tab at a time, so that two tabs never act on it at once. This tab does nothing while the other one is open.</p>
          <p>Use the other tab, or close it: this tab then continues the swap by itself.</p>
          {app.failure && (
            <>
              <Notice tone="warn" role="alert">
                {app.failure.message}
              </Notice>
              <button type="button" className="btn btn-secondary" onClick={() => location.reload()}>
                <Icon name="refresh" /> Reload this tab
              </button>
            </>
          )}
        </section>
      </div>
    );
  } else if (app.phase === 'failed') {
    content = (
      <div className="stack-lg narrow">
        <section className="card card-main stack" aria-labelledby="fail-title">
          <h1 id="fail-title" className="h1">
            {app.failure?.kind === 'directory' ? 'The maker’s details did not load' : app.failure?.kind === 'config' ? 'The network settings did not load' : 'The swap engine could not start'}
          </h1>
          <Notice tone="danger" role="alert">
            {app.failure?.message}
          </Notice>
          <p>
            {app.failure?.kind === 'directory'
              ? 'The page reads the maker’s address and price from a small public file. Check your connection and try again.'
              : 'This browser may block Web Workers, WebAssembly or storage, for example in some private modes. Try another browser or a normal window.'}
          </p>
          <button type="button" className="btn btn-primary" onClick={() => appStore.retry()}>
            <Icon name="refresh" /> Try again
          </button>
        </section>
      </div>
    );
  } else if (view === 'restore' && !swap) {
    content = <RestoreScreen onCancel={() => setView('auto')} onRestored={() => setView('auto')} />;
  } else if (!swap) {
    content = <SwapForm onCreated={() => setView('recovery')} onRestore={() => setView('restore')} />;
  } else if (swap.stage === 'awaiting_deposit') {
    content = view === 'recovery' ? <RecoveryStep swap={swap} onDone={() => setView('auto')} /> : <PayScreen swap={swap} />;
  } else if (swap.stage === 'done') {
    content = <DoneScreen swap={swap} />;
  } else if (swap.stage === 'refunded') {
    content = <RefundedScreen swap={swap} />;
  } else {
    content = <ProgressScreen swap={swap} exit={exit} />;
  }

  return (
    <div className="app">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <Header init={app.init} network={app.terms?.network} onHome={() => !swap && setView('auto')} />
      <DemoBanner settings={app.settings} init={app.init} swap={swap} />
      <ConfigOverrideBanner choice={app.configOverride} />
      <NetworkBanner network={app.network} />
      <main id="main" ref={main} className={`main main-${screenKey}`}>
        <EngineWarnings warnings={app.warnings} restartedAt={app.restartedAt} />
        {content}
      </main>
      <Footer />
    </div>
  );
}

function screenFor(phase: string, view: View, swap: SwapState | null): string {
  if (phase !== 'ready') return phase;
  if (!swap) return view === 'restore' ? 'restore' : 'form';
  if (swap.stage === 'awaiting_deposit') return view === 'recovery' ? 'recovery' : 'pay';
  if (swap.stage === 'done' || swap.stage === 'refunded') return swap.stage;
  return 'progress';
}
