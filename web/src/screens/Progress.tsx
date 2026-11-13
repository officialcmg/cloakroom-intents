import { useState } from 'react';
import type { SwapState, TxRef } from '../engine';
import { remainingMs } from '../engine/timeline';
import { appStore, useApp } from '../app/useZecret';
import type { ExitControl } from '../app/useExit';
import { destAmount, groupState, groupsFor, nowCopy, receiveLine, refundReason, stageLabel, formatUsdcAmount } from '../app/stageCopy';
import { validateZcashReturnAddress } from '../lib/address/zcash';
import { formatZec } from '../lib/units';
import { Icon } from '../ui/Icon';
import { formatClock, formatDuration, Hash, Notice, Spinner, TimedProgress, useNow } from '../ui/bits';
import { chainName } from '../ui/TokenPicker';
import { saveRecoveryFile } from './Pay';

const SAFETY_ICON = { safe: 'check', come_back: 'clock', keep_open: 'alert', action: 'alert' } as const;

export function useEta(swap: SwapState): { leftMs?: number; doneBy?: number } {
  const { init } = useApp();
  const now = useNow(1000);
  const scale = init?.timeScale ?? 1;
  const real = remainingMs(swap.stage, (now - swap.stageAt) * scale, {
    confirmations: swap.funding?.confirmations,
  });
  // While the user must act, a countdown would mislead.
  if (real === undefined || swap.needs) return {};
  const leftMs = real / scale;
  return { leftMs, doneBy: now + leftMs };
}

export function ProgressScreen({ swap, exit }: { swap: SwapState; exit: ExitControl }) {
  const copy = nowCopy(swap);
  const eta = useEta(swap);
  const { network } = useApp();
  const refund = swap.stage === 'refunding' || swap.stage === 'recovering' || swap.stage === 'recovered';

  return (
    <div className="progress-layout">
      <div className="progress-top">
        <p className="summary-line summary-line-lg">
          <span>{formatZec(BigInt(swap.params.amountZats))} ZEC</span>
          <Icon name="arrow" size={18} />
          <span>{receiveLine(swap, network?.near.tokenSymbol)}</span>
        </p>
        {eta.leftMs !== undefined && (
          <p className="eta" data-testid="eta">
            <Icon name="clock" size={16} /> About {formatDuration(eta.leftMs)} left{eta.doneBy ? `, done around ${formatClock(eta.doneBy)}` : ''}
          </p>
        )}
      </div>

      <section className={`card now now-${copy.safety}`} aria-labelledby="now-title" aria-live="polite">
        <p className="now-kicker">{refund ? 'Getting your ZEC back' : 'Now'}</p>
        <h1 id="now-title" className="h1" data-testid="now-title">
          {copy.title}
        </h1>
        {refund && swap.refund?.reason && <RefundWhy swap={swap} />}
        {copy.body && <p className="lede">{copy.body}</p>}
        {swap.activity && <TimedProgress key={`${swap.activity.label}-${swap.activity.startedAt}`} startedAt={swap.activity.startedAt} estimatedMs={swap.activity.estimatedMs} label={swap.activity.label} />}
        {swap.stage === 'exit_retry' && <ExitDecision swap={swap} exit={exit} />}
        {swap.stage === 'exiting' && <ExitTrack swap={swap} />}
        {swap.needs === 'return_address' && <ReturnAddressForm swap={swap} />}
        {copy.safetyText && (
          <div className={`safety safety-${copy.safety}`} data-testid="safety">
            <Icon name={SAFETY_ICON[copy.safety]} size={18} />
            <span>{copy.safetyText}</span>
          </div>
        )}
      </section>

      <section className="card steps-card" aria-labelledby="steps-title">
        <h2 id="steps-title" className="h2">
          Steps
        </h2>
        <StepList swap={swap} />
      </section>

      <section className="card tx-card" aria-labelledby="tx-title">
        <h2 id="tx-title" className="h2">
          Transactions
        </h2>
        <TxList swap={swap} />
        <RecoveryLink swap={swap} />
      </section>
    </div>
  );
}

function RefundWhy({ swap }: { swap: SwapState }) {
  const r = refundReason(swap.refund?.reason, swap);
  return (
    <Notice tone="info" title={r.title} icon="return">
      {r.body}
    </Notice>
  );
}

export function StepList({ swap }: { swap: SwapState }) {
  const groups = groupsFor(swap);
  const seen = new Map(swap.history.map((h) => [h.stage, h.at]));
  return (
    <ol className="steps">
      {groups.map((g) => {
        const st = groupState(g, swap);
        const reached = g.stages.filter((s) => seen.has(s) && s !== 'exit_retry');
        return (
          <li key={g.id} className={`step is-${st}`} aria-current={st === 'current' ? 'step' : undefined}>
            <span className="step-dot" aria-hidden="true">
              {st === 'done' ? <Icon name="check" size={14} /> : st === 'stopped' ? <Icon name="close" size={12} /> : null}
            </span>
            <div className="step-body">
              <p className="step-title">
                {g.title(swap)}
                <span className="sr-only">{st === 'done' ? ' (done)' : st === 'current' ? ' (in progress)' : st === 'stopped' ? ' (stopped)' : ''}</span>
              </p>
              {(st === 'current' || st === 'done' || st === 'stopped') && reached.length > 0 && (
                <ul className="substeps">
                  {reached.map((s) => (
                    <li key={s} className={s === swap.stage && st === 'current' ? 'is-now' : undefined}>
                      <span>{stageLabel(s, swap)}</span>
                      <time className="substep-time">{formatClock(seen.get(s)!)}</time>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

const CHAIN_LABEL: Record<TxRef['chain'], string> = { zcash: 'Zcash', near: 'NEAR', destination: '' };

export function TxList({ swap }: { swap: SwapState }) {
  const simulated = swap.engine === 'simulated';
  if (swap.txs.length === 0) return <p className="hint">Transactions show here as they happen.</p>;
  return (
    <>
      <ul className="txs">
        {[...swap.txs].reverse().map((t) => (
          <li key={t.id} className="tx">
            <span className={`chip chip-${t.chain}`}>{t.chain === 'destination' ? chainName(swap.params.destination.chain) : CHAIN_LABEL[t.chain]}</span>
            <span className="tx-label">{t.label}</span>
            <Hash value={t.hash} href={simulated ? undefined : t.url} label={`${t.label} transaction`} />
          </li>
        ))}
      </ul>
      <p className="fineprint">
        {simulated
          ? 'Simulated transactions: the hashes are random and have no explorer page.'
          : 'Zcash transactions are shielded: an explorer shows that they exist, not what they contain.'}
      </p>
    </>
  );
}

function RecoveryLink({ swap }: { swap: SwapState }) {
  const [state, setState] = useState<'idle' | 'saved' | 'error'>('idle');
  return (
    <p className="recovery-link">
      <button
        type="button"
        className="btn btn-link"
        onClick={() =>
          saveRecoveryFile().then(
            () => setState('saved'),
            () => setState('error'),
          )
        }
      >
        <Icon name="down" size={16} /> {swap.recoveryExportedAt ? 'Save the recovery file again' : 'Save a recovery file'}
      </button>
      {state === 'saved' && <span className="hint"> Saved.</span>}
      {state === 'error' && <span className="hint text-danger"> It could not be saved.</span>}
    </p>
  );
}

function ExitDecision({ swap, exit }: { swap: SwapState; exit: ExitControl }) {
  const code = swap.lastError?.code;
  const usdc = swap.payout ? formatUsdcAmount(swap.payout.usdc) : 'The USDC';
  const offered = swap.exit?.offered;
  return (
    <div className="stack">
      {code === 'EXIT_BELOW_MINIMUM' && offered ? (
        <>
          <p className="lede">
            The price for the delivery moved. NEAR Intents now offers about {destAmount(swap, offered.amountOut)}, at least {destAmount(swap, offered.minAmountOut)}. Your minimum is{' '}
            {destAmount(swap, swap.exit?.approvedMinimum ?? swap.params.minDestinationAmount)}. Nothing was signed.
          </p>
          <p>
            {usdc} is in NEAR Intents under the key made in this browser. Nobody else can move it. Prices often come back within minutes: you can get a new price, or accept the lower amount.
          </p>
          <div className="row-actions">
            <button type="button" className="btn btn-primary" onClick={exit.retry} disabled={exit.busy}>
              {exit.busy ? <Spinner /> : <Icon name="refresh" size={16} />} Get a new price
            </button>
            <button type="button" className="btn btn-secondary" onClick={exit.acceptLower} disabled={exit.busy}>
              Accept at least {destAmount(swap, offered.minAmountOut)}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="lede">{exitProblem(code, swap)}</p>
          <p>
            {usdc} is in NEAR Intents under the key made in this browser. Nobody else can move it. Trying again starts with a new firm price.
          </p>
          <div className="row-actions">
            <button type="button" className="btn btn-primary" onClick={exit.retry} disabled={exit.busy}>
              {exit.busy ? <Spinner /> : <Icon name="refresh" size={16} />} Try the delivery again
            </button>
          </div>
        </>
      )}
      {exit.error && (
        <Notice tone="danger" role="alert">
          {exit.error}
        </Notice>
      )}
    </div>
  );
}

function exitProblem(code: string | undefined, swap: SwapState): string {
  const chain = chainName(swap.params.destination.chain);
  if (code === 'EXIT_AMOUNT_TOO_LOW') return `NEAR Intents says this amount is too small to send to ${chain}. ${swap.lastError?.message ?? ''}`;
  if (code === 'EXIT_REFUSED') return 'The engine refused the delivery request from NEAR Intents, because it did not match what you approved. Nothing was signed.';
  return `NEAR Intents could not complete the delivery to ${chain} and returned the USDC.`;
}

const EXIT_STEPS: { phase: string; label: string }[] = [
  { phase: 'quoting', label: 'Firm price' },
  { phase: 'signing', label: 'Check and sign' },
  { phase: 'submitted', label: 'Submitted' },
  { phase: 'delivering', label: 'Delivering' },
];

function ExitTrack({ swap }: { swap: SwapState }) {
  const current = EXIT_STEPS.findIndex((x) => x.phase === swap.exit?.phase);
  return (
    <ol className="exit-track" aria-label="Delivery steps">
      {EXIT_STEPS.map((x, i) => {
        const st = i < current ? 'done' : i === current ? 'current' : 'upcoming';
        return (
          <li key={x.phase} className={`exit-step is-${st}`} aria-current={st === 'current' ? 'step' : undefined}>
            {x.label}
          </li>
        );
      })}
    </ol>
  );
}

export function ReturnAddressForm({ swap }: { swap: SwapState }) {
  const [value, setValue] = useState('');
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const r = validateZcashReturnAddress(value, swap.network);
    if (!r.ok) {
      setError(r.reason);
      return;
    }
    setBusy(true);
    try {
      await appStore.engine!.setReturnAddress(r.address);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="return-form stack-sm" onSubmit={submit} noValidate>
      <label className="label" htmlFor="return-address">
        Your Zcash address
      </label>
      <input
        id="return-address"
        className="input input-mono"
        value={value}
        onChange={(e) => {
          setValue(e.target.value);
          setError(undefined);
        }}
        placeholder={swap.network === 'mainnet' ? 'u1…' : 'utest1…'}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={!!error}
        aria-describedby="return-address-hint"
      />
      <p className="hint" id="return-address-hint">
        {error ? <span className="text-danger">{error}</span> : 'A shielded unified address from your wallet.'}
      </p>
      <button type="submit" className="btn btn-primary" disabled={busy}>
        {busy && <Spinner />} Send my ZEC here
      </button>
    </form>
  );
}
