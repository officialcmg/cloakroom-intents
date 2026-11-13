import { useState } from 'react';
import type { SwapState } from '../engine';
import { appStore } from '../app/useZecret';
import { destAmount, exitAmount, formatUsdcAmount, refundReason, shortAddr } from '../app/stageCopy';
import { formatZec } from '../lib/units';
import { Icon } from '../ui/Icon';
import { Hash, Notice } from '../ui/bits';
import { chainName } from '../ui/TokenPicker';
import { ReturnAddressForm, StepList, TxList } from './Progress';

function NewSwapActions() {
  const [error, setError] = useState<string>();
  const startOver = async () => {
    try {
      await appStore.engine!.forget();
    } catch (e) {
      setError((e as Error).message);
    }
  };
  return (
    <div className="actions">
      <button type="button" className="btn btn-primary btn-lg btn-block" onClick={startOver}>
        Start a new swap
      </button>
      <p className="hint center">This deletes the finished swap and its keys from this browser.</p>
      {error && (
        <Notice tone="danger" role="alert">
          {error}
        </Notice>
      )}
    </div>
  );
}

function Revealed({ swap }: { swap: SwapState }) {
  const chain = chainName(swap.params.destination.chain);
  return (
    <section className="card" aria-labelledby="revealed-title">
      <h2 id="revealed-title" className="h2">
        What this swap revealed
      </h2>
      <div className="privacy privacy-done">
        <div>
          <h3 className="h3">
            <Icon name="shield" size={16} /> Stayed private
          </h3>
          <ul className="plain-list">
            <li>Your Zcash wallet, its balance and its history. Your payment went into the shielded pool and stayed there.</li>
            <li>Which Zcash wallet paid for this swap. Nothing on NEAR or {chain} points back to it.</li>
            <li>The messages between this browser and the maker. They were encrypted memos inside shielded transactions.</li>
          </ul>
        </div>
        <div>
          <h3 className="h3">
            <Icon name="eye" size={16} /> Public
          </h3>
          <ul className="plain-list">
            <li>On NEAR: the escrow, the USDC amount, the times of each step and the new key made for this swap.</li>
            <li>On {chain}: the amount of {swap.params.destination.symbol} and the address that received it.</li>
            <li>The maker knows that a swap of this size happened, and when. It does not learn your Zcash wallet.</li>
            <li>The Zcash light-wallet server, the NEAR RPC and NEAR Intents saw this browser’s IP address, unless you used Tor or a VPN.</li>
          </ul>
        </div>
      </div>
    </section>
  );
}

export function DoneScreen({ swap }: { swap: SwapState }) {
  const d = swap.params.destination;
  const delivered = swap.exit?.deliveredAmount ?? swap.exit?.amountOut;
  const delivery = swap.txs.find((t) => t.id === 'delivery');
  const simulated = swap.engine === 'simulated';
  const direct = swap.exit?.direct;
  return (
    <div className="stack-lg">
      <section className="card card-main done" aria-labelledby="done-title">
        <span className="done-seal" aria-hidden="true">
          <Icon name="check" size={30} />
        </span>
        <p className="now-kicker">Swap complete</p>
        <h1 id="done-title" className="display-sm" data-testid="done-title">
          {direct
            ? `You received ${delivered ? exitAmount(swap, delivered) : direct.symbol} on NEAR`
            : `You received ${delivered ? destAmount(swap, delivered) : d.symbol} on ${chainName(d.chain)}`}
        </h1>
        <p className="lede">
          Sent to <span className="mono">{direct ? direct.nearAccount : shortAddr(d.address)}</span>.
          {direct && ` On this test network the exit has no 1Click: the engine sent ${direct.symbol} from NEAR Intents to your NEAR account. No ${d.symbol} was delivered on ${chainName(d.chain)}.`}
        </p>
        {delivery && (
          <div className="kv kv-inline">
            <span className="label">{direct ? 'NEAR' : chainName(d.chain)} transaction</span>
            <Hash value={delivery.hash} href={simulated ? undefined : delivery.url} label="delivery transaction" />
          </div>
        )}
        <dl className="rows rows-compact">
          <div className="row">
            <dt>You paid</dt>
            <dd>{formatZec(BigInt(swap.deposit.receivedZats))} ZEC</dd>
          </div>
          {swap.quote && (
            <div className="row">
              <dt>The maker paid</dt>
              <dd>{formatUsdcAmount(swap.quote.usdc)}</dd>
            </div>
          )}
          {!direct && (
            <div className="row">
              <dt>Your guaranteed minimum was</dt>
              <dd>{destAmount(swap, swap.params.minDestinationAmount)}</dd>
            </div>
          )}
          {swap.claim?.punishPath && (
            <div className="row">
              <dt>Claim</dt>
              <dd>After the deadline, because the maker went quiet</dd>
            </div>
          )}
        </dl>
        {swap.leftover && (
          <Notice tone={swap.leftover.toAddress ? 'ok' : 'info'} title={`${formatZec(BigInt(swap.leftover.amountZats))} ZEC was left over`}>
            {swap.leftover.toAddress ? `It went back to ${shortAddr(swap.leftover.toAddress)}.` : 'You paid more than needed. Enter a Zcash address to get it back.'}
          </Notice>
        )}
        {swap.needs === 'return_address' && <ReturnAddressForm swap={swap} />}
        <NewSwapActions />
      </section>
      <Revealed swap={swap} />
      <details className="card details-card">
        <summary className="h2">All steps and transactions</summary>
        <StepList swap={swap} />
        <TxList swap={swap} />
      </details>
    </div>
  );
}

export function RefundedScreen({ swap }: { swap: SwapState }) {
  const r = refundReason(swap.refund?.reason, swap);
  const tx = swap.txs.find((t) => t.id === 'refund');
  const simulated = swap.engine === 'simulated';
  return (
    <div className="stack-lg">
      <section className="card card-main done done-refund" aria-labelledby="refunded-title">
        <span className="done-seal done-seal-refund" aria-hidden="true">
          <Icon name="return" size={28} />
        </span>
        <p className="now-kicker">Swap stopped, ZEC returned</p>
        <h1 id="refunded-title" className="display-sm" data-testid="refunded-title">
          {swap.refund?.amountZats ? `${formatZec(BigInt(swap.refund.amountZats))} ZEC is on its way back` : 'Your ZEC is on its way back'}
        </h1>
        {swap.refund?.toAddress && (
          <p className="lede">
            To <span className="mono">{shortAddr(swap.refund.toAddress)}</span>. It shows in your wallet after the next Zcash block, in a minute or two.
          </p>
        )}
        <Notice tone="info" title={r.title} icon="info">
          {r.body}
        </Notice>
        {tx && (
          <div className="kv kv-inline">
            <span className="label">Refund transaction</span>
            <Hash value={tx.hash} href={simulated ? undefined : tx.url} label="refund transaction" />
          </div>
        )}
        <NewSwapActions />
      </section>
      <details className="card details-card">
        <summary className="h2">All steps and transactions</summary>
        <StepList swap={swap} />
        <TxList swap={swap} />
      </details>
    </div>
  );
}
