import { useRef, useState } from 'react';
import type { EngineErrorData, InitStatus, SwapState } from '../engine';
import { appStore } from '../app/useZecret';
import { FAILURES, SPEEDS, settingsUrl, type PageSettings } from '../app/settings';
import { Icon } from '../ui/Icon';
import { Notice } from '../ui/bits';
import { Banner } from './Chrome';
import type { ConfigChoice, NetworkConfig } from '../lib/netconfig';

export { Footer, Header, useTheme } from './Chrome';

/**
 * A development build loaded network settings that this site does not ship (review 3,
 * NV3-01). Those settings choose the escrow, the servers and the maker, so the page says so
 * above everything else.
 */
export function ConfigOverrideBanner({ choice }: { choice?: ConfigChoice }) {
  if (!choice) return null;
  return (
    <div className="config-warning" role="alert" data-testid="config-warning">
      <Icon name="alert" size={20} />
      <div>
        <strong>Test build with outside network settings.</strong> This page uses the settings from <code className="mono">{choice.name}</code>, not the ones this site ships. That file chooses
        the escrow, the servers and the maker. Do not pay into this page unless you made that file yourself.
      </div>
    </div>
  );
}

/** Problems the engine reports that are not about one call: storage, a restarted engine. */
export function EngineWarnings({ warnings, restartedAt }: { warnings: EngineErrorData[]; restartedAt?: number }) {
  return (
    <>
      {restartedAt !== undefined && (
        <Notice tone="warn" title="The swap engine restarted" role="status">
          The engine in this tab stopped because of an internal error. The page started it again from the swap stored in this browser, and the swap continues. Save the recovery file if you
          have not.
        </Notice>
      )}
      {warnings.map((w, i) =>
        w.code === 'STORAGE' && w.transient ? (
          <Notice key={i} tone="danger" title="Your swap could not be saved in this browser" role="alert">
            <span data-testid="storage-stopped">
              The engine has stopped and does nothing until this browser stores the swap again. It tries again every few seconds. Keep this tab open, free some disk space, and save the
              recovery file. ({w.message})
            </span>
          </Notice>
        ) : (
          <Notice key={i} tone="warn" title={w.code === 'STORAGE' ? 'Storage is off in this browser' : 'The engine reports a problem'} role="alert">
            {w.message}
          </Notice>
        ),
      )}
    </>
  );
}

/** What is real on a test network, for the real engine: one line, the rest under "Details". */
export function NetworkBanner({ network }: { network?: NetworkConfig }) {
  if (!network || network.network === 'mainnet') return null;
  return (
    <Banner testId="network-banner" lead={<><strong>{network.network === 'testnet' ? 'Testnet.' : 'Local test network.'}</strong> Test money with no value.</>}>
      <ul className="plain-list">
        <li>{network.label}. Real transactions, test money with no value.</li>
        {(network.honesty ?? []).map((l) => (
          <li key={l}>{l}</li>
        ))}
      </ul>
    </Banner>
  );
}

export function DemoBanner({ settings, init, swap }: { settings: PageSettings; init?: InitStatus; swap: SwapState | null }) {
  if (init?.engine !== 'simulated') return null;
  const speed = settings.sim.speed;
  const fail = FAILURES.find((f) => f.id === settings.sim.fail);
  return (
    <Banner testId="demo-banner" tone="demo" lead={<><strong>Demo.</strong> Simulated. No real money moves.</>}>
      <ul className="plain-list">
        <li>The swap engine is simulated: no chains, no funds. Prices on the form are live from NEAR Intents.</li>
        <li>
          {speed === 1 ? 'Real time: a swap takes about 10 minutes. ' : `Runs ${speed}× faster than real time. `}
          {speed === 1 && <a href={settingsUrl({ speed: 20 })}>Run 20× faster</a>}
        </li>
        {fail && <li>Failure switched on: {fail.label}.</li>}
      </ul>
      <DevPanel settings={settings} swap={swap} />
    </Banner>
  );
}

/** Controls for the simulated engine: speed, failures, skipping a wait, a cleared browser. */
export function DevPanel({ settings, swap }: { settings: PageSettings; swap: SwapState | null }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [msg, setMsg] = useState<string>();
  const current = settings.sim.fail ?? '';
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
