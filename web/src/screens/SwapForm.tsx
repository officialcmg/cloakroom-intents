import { useId, useMemo, useState } from 'react';
import type { CreateSwapParams, EngineError } from '../engine';
import { appStore, useApp, usePref } from '../app/useZecret';
import { usePrice, useTokens } from '../app/usePrice';
import { chainInfo, validateDestinationAddress } from '../lib/address/chains';
import { validateZcashReturnAddress } from '../lib/address/zcash';
import type { Token } from '../lib/oneclick';
import { formatBps, makerMarginBps, type CombinedPrice } from '../lib/price';
import { formatUnits, formatUsdc, formatZec, parseUnits, toDecimalString, ZEC_DECIMALS } from '../lib/units';
import { Icon } from '../ui/Icon';
import { Notice, Spinner, useNow } from '../ui/bits';
import { chainName, TokenBadge, TokenPicker } from '../ui/TokenPicker';

const DEFAULT_TOKEN = 'nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near';

function amountError(input: string): string | undefined {
  const r = parseUnits(input, ZEC_DECIMALS);
  if (r.ok) return r.value === 0n ? 'Enter an amount above zero.' : undefined;
  if (r.reason === 'empty') return undefined;
  if (r.reason === 'too_precise') return 'ZEC has at most 8 decimal places.';
  return 'Enter a number, for example 0.5';
}

export function SwapForm({ onCreated, onRestore }: { onCreated: () => void; onRestore: () => void }) {
  const app = useApp();
  const terms = app.terms!;
  const tokens = useTokens();
  const [amount, setAmount] = useState('');
  const [tokenId, setTokenId] = usePref<string>('token', DEFAULT_TOKEN);
  const [address, setAddress] = useState('');
  const [addressTouched, setAddressTouched] = useState(false);
  const [showReturn, setShowReturn] = useState(false);
  const [returnAddress, setReturnAddress] = useState('');
  const [returnTouched, setReturnTouched] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitFailure, setSubmitFailure] = useState<{ message: string; inputs: string }>();
  const [triedSubmit, setTriedSubmit] = useState(false);
  const ids = { amount: useId(), token: useId(), address: useId(), ret: useId() };

  const list = tokens.list?.destinations ?? [];
  const token: Token | undefined = list.find((t) => t.assetId === tokenId) ?? list.find((t) => t.assetId === DEFAULT_TOKEN) ?? list[0];

  const parsed = parseUnits(amount, ZEC_DECIMALS);
  const payZats = parsed.ok && parsed.value > 0n ? parsed.value : undefined;
  const price = usePrice(payZats, token, address, terms);

  const addrCheck = token ? validateDestinationAddress(token.blockchain, address) : undefined;
  const addrRejectedByIntents = price.status === 'error' && price.error.kind === 'recipient' && addrCheck?.ok;
  const showAddrError = (addressTouched || triedSubmit) && address.trim() !== '' && addrCheck && !addrCheck.ok;
  const retCheck = returnAddress.trim() ? validateZcashReturnAddress(returnAddress, terms.network) : undefined;
  const showRetError = (returnTouched || triedSubmit) && retCheck && !retCheck.ok;
  const amtErr = amountError(amount);

  // An error from the engine applies to the inputs it was given; editing them hides it.
  const inputs = [amount, tokenId, address, returnAddress].join('|');
  const submitError = submitFailure?.inputs === inputs ? submitFailure.message : undefined;

  const ready = price.status === 'ok' && addrCheck?.ok && price.recipientChecked && (!retCheck || retCheck.ok) && !!token;
  const blocker = !payZats
    ? 'Enter an amount.'
    : price.status === 'invalid'
      ? undefined
      : !address.trim()
        ? `Enter your ${token?.symbol ?? ''} address.`
        : addrCheck && !addrCheck.ok
          ? 'Check the receiving address.'
          : retCheck && !retCheck.ok
            ? 'Check the return address.'
            : price.status === 'loading' || (price.status === 'ok' && !price.recipientChecked)
              ? 'Getting the price…'
              : price.status === 'error'
                ? 'No price yet.'
                : undefined;

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setTriedSubmit(true);
    if (!ready || price.status !== 'ok' || !token || !addrCheck?.ok) return;
    const p = price.price;
    const params: CreateSwapParams = {
      amountZats: p.payZats.toString(),
      destination: { assetId: token.assetId, symbol: token.symbol, chain: token.blockchain, decimals: token.decimals, address: addrCheck.address },
      returnAddress: retCheck?.ok ? retCheck.address : undefined,
      minUsdc: p.usdcMin.toString(),
      minDestinationAmount: p.destMin.toString(),
      shown: { usdcExpected: p.usdcExpected.toString(), destinationExpected: p.destExpected.toString(), premiumZats: p.premiumZats.toString(), networkFeeZats: p.networkFeeZats.toString() },
    };
    setSubmitting(true);
    try {
      await appStore.engine!.createSwap(params);
      onCreated();
    } catch (err) {
      setSubmitFailure({ message: (err as EngineError).message, inputs });
    } finally {
      setSubmitting(false);
    }
  };

  const limits = `${formatZec(terms.minZats, 4)} to ${formatZec(terms.maxZats, 4)} ZEC`;
  const zecUsd = tokens.list?.zecUsd;

  return (
    <div className="stack-lg">
      <header className="hero">
        <h1 className="display">Swap shielded ZEC for a token on another chain</h1>
        <p className="lede">Pay once from your own Zcash wallet. Your browser runs the swap with the maker and sends the tokens to your address. There is no custodian and nothing to install.</p>
      </header>

      <form className="card card-main" onSubmit={submit} noValidate aria-describedby="form-status">
        <div className="field">
          <label className="label" htmlFor={ids.amount}>
            You pay
          </label>
          <div className={`amount-input${amtErr ? ' has-error' : ''}`}>
            <input
              id={ids.amount}
              className="amount-field"
              inputMode="decimal"
              autoComplete="off"
              placeholder="0.00"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              aria-invalid={!!amtErr || price.status === 'invalid'}
              aria-describedby={`${ids.amount}-hint`}
            />
            <span className="amount-unit">
              <TokenBadge symbol="ZEC" size={26} />
              ZEC
            </span>
          </div>
          <p className="hint" id={`${ids.amount}-hint`}>
            {amtErr ? (
              <span className="text-danger">{amtErr}</span>
            ) : price.status === 'invalid' ? (
              <span className="text-danger">{limitMessage(price.error, terms.network)}</span>
            ) : (
              <>
                Shielded ZEC, {limits}.{payZats && zecUsd ? ` About $${(Number(toDecimalString(payZats, 8)) * zecUsd).toLocaleString(undefined, { maximumFractionDigits: 2 })}.` : ''}
              </>
            )}
          </p>
        </div>

        <div className="field">
          <label className="label" htmlFor={ids.token}>
            You receive
          </label>
          {tokens.error ? (
            <Notice tone="danger" title="The token list did not load">
              <p>{tokens.error}</p>
              <button type="button" className="btn btn-secondary btn-sm" onClick={tokens.retry}>
                <Icon name="refresh" size={16} /> Try again
              </button>
            </Notice>
          ) : !tokens.list ? (
            <div className="token-select is-loading">
              <Spinner label="Loading tokens" />
              <span className="token-select-text">
                <span className="token-chain">Loading tokens from NEAR Intents…</span>
              </span>
            </div>
          ) : (
            <TokenPicker id={ids.token} tokens={list} value={token} onChange={(t) => setTokenId(t.assetId)} />
          )}
        </div>

        <div className="field">
          <label className="label" htmlFor={ids.address}>
            Your {token ? `${token.symbol} address on ${chainName(token.blockchain)}` : 'receiving address'}
          </label>
          <div className={`input-wrap${showAddrError || addrRejectedByIntents ? ' has-error' : ''}${addrCheck?.ok ? ' is-valid' : ''}`}>
            <input
              id={ids.address}
              className="input input-mono"
              value={address}
              onChange={(e) => setAddress(e.target.value)}
              onBlur={() => setAddressTouched(true)}
              placeholder={token ? chainInfo(token.blockchain)?.example : ''}
              autoComplete="off"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              aria-invalid={!!showAddrError || !!addrRejectedByIntents}
              aria-describedby={`${ids.address}-hint`}
            />
            {addrCheck?.ok && <Icon name="check" className="input-ok" />}
          </div>
          <p className="hint" id={`${ids.address}-hint`}>
            {showAddrError && addrCheck && !addrCheck.ok ? (
              <span className="text-danger">{addrCheck.reason}</span>
            ) : addrRejectedByIntents ? (
              <span className="text-danger">NEAR Intents does not accept this address for {token?.symbol}. Check it, or choose another token.</span>
            ) : addrCheck?.ok && addrCheck.note ? (
              addrCheck.note
            ) : (
              <>{token ? chainInfo(token.blockchain)?.hint : 'Choose a token first.'} The tokens go here. This address is public on that chain.</>
            )}
          </p>
        </div>

        <div className="field">
          {!showReturn ? (
            <button type="button" className="btn btn-link" onClick={() => setShowReturn(true)} aria-expanded="false">
              <Icon name="return" size={16} /> Add a Zcash return address (recommended)
            </button>
          ) : (
            <>
              <label className="label" htmlFor={ids.ret}>
                Zcash return address <span className="label-opt">optional</span>
              </label>
              <div className={`input-wrap${showRetError ? ' has-error' : ''}${retCheck?.ok ? ' is-valid' : ''}`}>
                <input
                  id={ids.ret}
                  className="input input-mono"
                  value={returnAddress}
                  onChange={(e) => setReturnAddress(e.target.value)}
                  onBlur={() => setReturnTouched(true)}
                  placeholder={terms.network === 'mainnet' ? 'u1…' : 'utest1…'}
                  autoComplete="off"
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  aria-invalid={!!showRetError}
                  aria-describedby={`${ids.ret}-hint`}
                />
                {retCheck?.ok && <Icon name="check" className="input-ok" />}
              </div>
              <p className="hint" id={`${ids.ret}-hint`}>
                {showRetError && retCheck && !retCheck.ok ? (
                  <span className="text-danger">{retCheck.reason}</span>
                ) : (
                  'A shielded unified address from your wallet. If the swap stops, your ZEC goes back here. So does any ZEC you overpay. Without it, the page asks you when needed.'
                )}
              </p>
            </>
          )}
        </div>

        <PriceCard price={price} token={token} payZats={payZats} zecUsd={zecUsd} />

        {submitError && (
          <Notice tone="danger" title="The swap could not start" role="alert">
            {submitError}
          </Notice>
        )}

        <div className="form-actions">
          <button type="submit" className="btn btn-primary btn-lg btn-block" disabled={submitting}>
            {submitting ? <Spinner /> : null}
            {submitting ? 'Making the swap’s keys…' : 'Continue'}
            {!submitting && <Icon name="arrow" />}
          </button>
          <p className="hint center" id="form-status" aria-live="polite">
            {triedSubmit && blocker ? blocker : 'Next: save a recovery file, then pay. No money moves before you pay.'}
          </p>
        </div>
      </form>

      <HowItWorks />

      <p className="center muted">
        <button type="button" className="btn btn-link" onClick={onRestore}>
          <Icon name="file" size={16} /> Have a recovery file? Restore a swap
        </button>
      </p>
    </div>
  );
}

function limitMessage(e: Extract<ReturnType<typeof usePrice>, { status: 'invalid' }>['error'], network: string): string {
  switch (e.kind) {
    case 'below_min':
      return `The maker takes at least ${formatZec(e.minZats, 4)} ZEC.`;
    case 'above_max':
      return `The maker takes at most ${formatZec(e.maxZats, 4)} ZEC per swap.`;
    case 'fees_exceed_amount':
      return `Fees and premium (${formatZec(e.costZats)} ZEC) are more than this amount.`;
    case 'maker_paused':
      return `The maker on ${network} is not taking swaps right now. Try again later.`;
  }
}

function Ago({ at }: { at: number }) {
  const now = useNow(1000);
  const s = Math.max(0, Math.round((now - at) / 1000));
  return <>{s < 5 ? 'just now' : `${s} s ago`}</>;
}

function PriceCard({ price, token, payZats, zecUsd }: { price: ReturnType<typeof usePrice>; token?: Token; payZats?: bigint; zecUsd?: number }) {
  const app = useApp();
  const terms = app.terms!;
  const shown: CombinedPrice | undefined = price.status === 'ok' ? price.price : price.status === 'loading' || price.status === 'error' ? price.previous : undefined;
  const stale = price.status !== 'ok';
  const margin = makerMarginBps(terms.rateMicroUsdcPerZec, zecUsd);
  const fmtDest = (v: bigint) => (token ? `${formatUnits(v, token.decimals)} ${token.symbol}` : '');
  const chain = token ? chainName(token.blockchain) : '';
  const quote = price.status === 'ok' ? price.quote : undefined;

  if (!payZats || !token || price.status === 'idle' || price.status === 'invalid') {
    return (
      <section className="price price-empty" aria-label="Price">
        <p className="price-placeholder">Enter an amount to see the price.</p>
      </section>
    );
  }

  return (
    <section className={`price${stale ? ' is-stale' : ''}`} aria-label="Price" aria-busy={price.status === 'loading'}>
      {shown && shown.payZats === payZats ? (
        <>
          <p className="price-line" data-testid="price-line">
            <span className="price-pay">
              You pay <strong>{formatZec(shown.payZats)} ZEC</strong>.
            </span>{' '}
            <span className="price-get">
              You receive at least <strong className="price-min">{fmtDest(shown.destMin)}</strong> on {chain}.
            </span>
          </p>
          <p className="price-sub">
            Expected about {fmtDest(shown.destExpected)}.{' '}
            {price.status === 'loading' ? (
              <span className="price-status">
                <Spinner /> Updating…
              </span>
            ) : quote ? (
              <span className="price-status">
                Updated <Ago at={quote.fetchedAt} />.{' '}
                <button type="button" className="btn btn-link btn-inline" onClick={price.refresh}>
                  Refresh
                </button>
              </span>
            ) : null}
          </p>
          <details className="breakdown">
            <summary>
              How this price is made <Icon name="chevron" size={16} className="breakdown-chevron" />
            </summary>
            <dl className="rows">
              <div className="row">
                <dt>Zcash network fees, 3 transactions</dt>
                <dd>{formatZec(shown.networkFeeZats)} ZEC</dd>
              </div>
              <div className="row">
                <dt>Maker’s premium ({formatBps(Number(terms.premiumBps))}, at least {formatZec(terms.premiumMinZats)} ZEC)</dt>
                <dd>{formatZec(shown.premiumZats)} ZEC</dd>
              </div>
              <div className="row">
                <dt>ZEC sold to the maker</dt>
                <dd>{formatZec(shown.swapZats)} ZEC</dd>
              </div>
              <div className="row">
                <dt>
                  Maker’s rate
                  {margin !== undefined && zecUsd && (
                    <span className="row-note">
                      {margin >= 0 ? `${formatBps(margin)} below` : `${formatBps(-margin)} above`} the market price of ${zecUsd.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                    </span>
                  )}
                </dt>
                <dd>1 ZEC = {formatUsdc(terms.rateMicroUsdcPerZec)} USDC</dd>
              </div>
              <div className="row">
                <dt>
                  USDC from the maker
                  <span className="row-note">The maker’s firm price arrives later. Below {formatUsdc(shown.usdcMin)} USDC the swap stops and your ZEC comes back.</span>
                </dt>
                <dd>{formatUsdc(shown.usdcExpected)} USDC</dd>
              </div>
              <div className="row">
                <dt>
                  NEAR Intents: USDC to {token.symbol} on {chain}
                  <span className="row-note">
                    {shown.exitCostBps !== undefined ? `Costs about ${formatBps(shown.exitCostBps)}, included. ` : ''}
                    The price is fetched again at delivery. Below the minimum, nothing is sent until you choose.
                  </span>
                </dt>
                <dd>about {fmtDest(shown.destExpected)}</dd>
              </div>
              <div className="row row-total">
                <dt>Guaranteed minimum</dt>
                <dd>{fmtDest(shown.destMin)}</dd>
              </div>
            </dl>
          </details>
        </>
      ) : price.status === 'loading' ? (
        <div className="price-loading" role="status">
          <Spinner /> Getting a price from NEAR Intents…
        </div>
      ) : null}
      {price.status === 'error' && price.error.kind !== 'recipient' && (
        <Notice tone="warn" title="No price from NEAR Intents" role="alert">
          <p>{exitErrorText(price.error, token)}</p>
          <button type="button" className="btn btn-secondary btn-sm" onClick={price.refresh}>
            <Icon name="refresh" size={16} /> Try again
          </button>
        </Notice>
      )}
    </section>
  );
}

function exitErrorText(e: { kind: string; message: string; minAmountIn?: bigint }, token: Token): string {
  if (e.kind === 'too_low' && e.minAmountIn) return `This amount is too small for ${token.symbol}. NEAR Intents needs at least ${formatUsdc(e.minAmountIn)} USDC for this route.`;
  if (e.kind === 'no_liquidity') return `NEAR Intents has no liquidity for ${token.symbol} right now. Choose another token or try later.`;
  if (e.kind === 'network') return e.message;
  return `NEAR Intents answered: ${e.message}`;
}

export function HowItWorks() {
  const items = useMemo(
    () => [
      { icon: 'wallet' as const, title: 'You pay once', text: 'Scan a QR code with your own shielded wallet. The ZEC goes to a temporary wallet that lives in this browser.' },
      { icon: 'lock' as const, title: 'The maker locks USDC first', text: 'The maker puts USDC in an escrow on NEAR before your ZEC moves on. Your browser checks it.' },
      { icon: 'key' as const, title: 'Two key halves', text: 'Your ZEC goes to an address that needs your key half and the maker’s. Claiming the USDC reveals yours; a refund reveals the maker’s.' },
      { icon: 'arrow' as const, title: 'Out to any chain', text: 'NEAR Intents turns the USDC into the token you chose and sends it to your address.' },
    ],
    [],
  );
  return (
    <section className="how" aria-labelledby="how-title">
      <h2 id="how-title" className="h2">
        How it works
      </h2>
      <ol className="how-list">
        {items.map((it) => (
          <li key={it.title} className="how-item">
            <span className="how-icon">
              <Icon name={it.icon} />
            </span>
            <div>
              <h3 className="h3">{it.title}</h3>
              <p>{it.text}</p>
            </div>
          </li>
        ))}
      </ol>
      <div className="privacy">
        <div>
          <h3 className="h3">
            <Icon name="shield" size={16} /> Stays private
          </h3>
          <p>Your Zcash wallet, its balance and its history. The link between your Zcash wallet and the address that receives the tokens.</p>
        </div>
        <div>
          <h3 className="h3">
            <Icon name="eye" size={16} /> Is public
          </h3>
          <p>On NEAR: the USDC amount and the escrow steps. On the destination chain: the amount and your address. The services this page talks to see your IP address unless you use Tor or a VPN.</p>
        </div>
      </div>
    </section>
  );
}
