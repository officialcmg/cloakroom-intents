import { useState } from 'react';
import type { SwapState } from '../engine';
import { appStore, useApp } from '../app/useZecret';
import { parsePaymentUri } from '../lib/zip321';
import { formatZec } from '../lib/units';
import { Icon } from '../ui/Icon';
import { CopyButton, Notice, TimedProgress } from '../ui/bits';
import { QrCode } from '../ui/QrCode';
import { receiveLine } from '../app/stageCopy';

/** Downloads the recovery file. Returns the file name, or throws. */
export async function saveRecoveryFile(): Promise<string> {
  const file = await appStore.engine!.exportRecovery();
  const blob = new Blob([file.text], { type: file.mimeType });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = file.fileName;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
  return file.fileName;
}

export function Steps({ current }: { current: 1 | 2 | 3 }) {
  const names = ['Details', 'Recovery file', 'Pay'];
  return (
    <ol className="wizard" aria-label="Steps before the swap starts">
      {names.map((n, i) => {
        const k = i + 1;
        const state = k < current ? 'done' : k === current ? 'current' : 'upcoming';
        return (
          <li key={n} className={`wizard-step is-${state}`} aria-current={state === 'current' ? 'step' : undefined}>
            <span className="wizard-dot">{state === 'done' ? <Icon name="check" size={14} /> : k}</span>
            <span className="wizard-name">{n}</span>
          </li>
        );
      })}
    </ol>
  );
}

function SwapSummary({ swap }: { swap: SwapState }) {
  const { network } = useApp();
  return (
    <p className="summary-line">
      <span>{formatZec(BigInt(swap.params.amountZats))} ZEC</span>
      <Icon name="arrow" size={16} />
      <span>{receiveLine(swap, network?.near.tokenSymbol)}</span>
    </p>
  );
}

function EnginePrep() {
  const { init } = useApp();
  if (!init || init.phase !== 'building_keys') return null;
  return (
    <div className="prep">
      <TimedProgress startedAt={init.startedAt} estimatedMs={init.estimatedMs} label="Preparing the engine in this browser (proving keys, once per visit)" />
    </div>
  );
}

export function RecoveryStep({ swap, onDone }: { swap: SwapState; onDone: () => void }) {
  const [saved, setSaved] = useState<string>();
  const [error, setError] = useState<string>();
  const save = async () => {
    setError(undefined);
    try {
      setSaved(await saveRecoveryFile());
    } catch (e) {
      setError((e as Error).message);
    }
  };
  return (
    <div className="stack-lg narrow">
      <Steps current={2} />
      <section className="card card-main stack" aria-labelledby="rec-title">
        <span className="seal-icon" aria-hidden="true">
          <Icon name="key" size={22} />
        </span>
        <h1 id="rec-title" className="h1">
          Save your recovery file
        </h1>
        <p className="lede">This browser just made the keys for your swap. If it loses its data, the recovery file lets you finish the swap or get your ZEC back.</p>
        <SwapSummary swap={swap} />
        {saved ? (
          <Notice tone="ok" title="Recovery file saved">
            Saved as <span className="mono">{saved}</span>. Check that it is in your downloads.
          </Notice>
        ) : null}
        {error && (
          <Notice tone="danger" title="The file could not be made" role="alert">
            {error}
          </Notice>
        )}
        <div className="actions">
          {saved ? (
            <button type="button" className="btn btn-primary btn-lg btn-block" onClick={onDone} autoFocus>
              Continue to payment <Icon name="arrow" />
            </button>
          ) : (
            <button type="button" className="btn btn-primary btn-lg btn-block" onClick={save} autoFocus>
              <Icon name="down" /> Save recovery file
            </button>
          )}
          {saved ? (
            <button type="button" className="btn btn-link" onClick={save}>
              Save it again
            </button>
          ) : (
            <button type="button" className="btn btn-link" onClick={onDone}>
              Continue without saving
            </button>
          )}
        </div>
        <p className="fineprint">Keep the file private: anyone who has it can move this swap’s funds. You can delete it once the swap is done.</p>
      </section>
      <EnginePrep />
    </div>
  );
}

/** Wallet apps with a released version that pays into the Ironwood pool (docs/WEB.md, "Wallets"). */
const WALLETS = 'Zodl, Zingo, Zkool, Edge or Cake';
/** Of those, the ones that offer a Zcash testnet wallet in the app. */
const TESTNET_WALLETS = 'Zkool or Zingo';
const FAUCET_URL = 'https://fauzec.com';

export function PayScreen({ swap }: { swap: SwapState }) {
  const app = useApp();
  const simulated = app.init?.engine === 'simulated';
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const expected = BigInt(swap.deposit.expectedZats);
  const received = BigInt(swap.deposit.receivedZats);
  const owed = expected - received;
  const partial = received > 0n && owed > 0n;
  const parsed = parsePaymentUri(swap.deposit.uri);
  const amountText = parsed?.amount ?? formatZec(owed);
  const headAmount = formatZec(owed);
  const testnet = swap.network !== 'mainnet';

  const cancel = async () => {
    setBusy(true);
    try {
      await appStore.engine!.forget();
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  };
  const simulatePay = async (fraction?: number) => {
    setError(undefined);
    try {
      await appStore.engine!.sim.pay(fraction ? ((owed * BigInt(Math.round(fraction * 100))) / 100n).toString() : undefined);
    } catch (e) {
      setError((e as Error).message);
    }
  };

  return (
    <div className="stack-lg">
      <Steps current={3} />
      <section className="card card-main pay" aria-labelledby="pay-title">
        <div className="pay-head">
          <h1 id="pay-title" className="h1">
            {partial ? `Send ${headAmount} ZEC more` : `Send exactly ${headAmount} ZEC`}
          </h1>
          <SwapSummary swap={swap} />
        </div>
        <div className="pay-grid">
          <div className="qr-plate">
            <QrCode text={swap.deposit.uri} label={`Payment request: ${amountText} ZEC to the swap's address`} />
          </div>
          <div className="pay-details stack">
            <div className="kv">
              <span className="label">Amount</span>
              <span className="kv-value">
                <span className="amount-big">{amountText} ZEC</span>
                <CopyButton text={amountText} label="amount" compact />
              </span>
            </div>
            <div className="kv">
              <span className="label">To this address</span>
              <span className="address-box mono" data-testid="deposit-address">
                {swap.deposit.address}
              </span>
              <CopyButton text={swap.deposit.address} label="address" />
            </div>
            <a className="btn btn-secondary btn-block" href={swap.deposit.uri}>
              <Icon name="wallet" /> Open in a wallet app
            </a>
            <p className="hint">
              {testnet
                ? `Pay with testnet ZEC from a wallet set to testnet, such as ${TESTNET_WALLETS}. Scan the code, or copy the address and the amount.`
                : `Use a shielded wallet such as ${WALLETS}. Scan the code, or copy the address and the amount.`}
            </p>
            {swap.network === 'testnet' && owed <= 100_000_000n && (
              <p className="hint" data-testid="faucet-hint">
                No wallet? The{' '}
                <a href={FAUCET_URL} target="_blank" rel="noreferrer noopener">
                  Fauzec faucet
                </a>{' '}
                pays 1 test ZEC to the address above. The swap uses the amount shown, and the rest goes to your return address.
              </p>
            )}
          </div>
        </div>

        {swap.restoredAt !== undefined && (
          <Notice tone="info" title="Restored from a recovery file">
            The address and the code above were computed again in this browser from the wallet key in the file.
          </Notice>
        )}

        {partial && (
          <Notice tone="warn" title={`Received ${formatZec(received)} ZEC of ${formatZec(expected)} ZEC`}>
            The swap starts when the full amount has arrived. The code above asks for the rest.
          </Notice>
        )}

        <div className="waiting" role="status" aria-live="polite">
          <span className="pulse" aria-hidden="true" />
          <div>
            <p className="waiting-title">Waiting for your payment</p>
            <p className="hint">It shows here a minute or two after you send. You can close this tab: the swap waits for you.</p>
          </div>
        </div>

        {simulated && (
          <div className="demo-box">
            <p className="demo-box-title">
              <Icon name="settings" size={16} /> Demo: the engine is simulated
            </p>
            <p className="hint">Do not send real ZEC to this address. Pretend that your wallet paid:</p>
            <div className="row-actions">
              <button type="button" className="btn btn-secondary btn-sm" onClick={() => simulatePay()}>
                Simulate the payment
              </button>
              <button type="button" className="btn btn-ghost btn-sm" onClick={() => simulatePay(0.4)}>
                Pay only 40%
              </button>
            </div>
          </div>
        )}

        {error && (
          <Notice tone="danger" role="alert">
            {error}
          </Notice>
        )}
      </section>

      {!swap.recoveryExportedAt && <RecoveryReminder />}

      <div className="center">
        {!confirmCancel ? (
          <button type="button" className="btn btn-link" onClick={() => setConfirmCancel(true)} disabled={received > 0n}>
            Change the swap
          </button>
        ) : (
          <Notice tone="warn" title="Start over?">
            <p>Only do this if you have not sent anything. ZEC sent to the old address after you start over is hard to recover without the recovery file.</p>
            <div className="row-actions">
              <button type="button" className="btn btn-secondary btn-sm" onClick={cancel} disabled={busy}>
                Yes, start over
              </button>
              <button type="button" className="btn btn-ghost btn-sm" onClick={() => setConfirmCancel(false)}>
                Keep this swap
              </button>
            </div>
          </Notice>
        )}
      </div>
      <EnginePrep />
    </div>
  );
}

export function RecoveryReminder() {
  const [saved, setSaved] = useState(false);
  if (saved) return null;
  return (
    <Notice tone="info" title="You have not saved a recovery file">
      <p>If this browser loses its data, the file lets you finish the swap or get your ZEC back.</p>
      <button
        type="button"
        className="btn btn-secondary btn-sm"
        onClick={async () => {
          await saveRecoveryFile();
          setSaved(true);
        }}
      >
        <Icon name="down" size={16} /> Save recovery file
      </button>
    </Notice>
  );
}
