// The live price on the swap form: maker side from the directory, exit side from a dry
// 1Click quote. Quotes refresh every 30 seconds while the form is open.
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { MakerTerms } from '../lib/directory';
import { DRY_REFUND_ACCOUNT, fetchExitQuote, fetchTokens, OneClickError, type ExitQuote, type Token, type TokenList } from '../lib/oneclick';
import { combine, computeZecSide, type CombinedPrice, type ZecSideError } from '../lib/price';
import { quoteSampleAddress, validateDestinationAddress } from '../lib/address/chains';

export const QUOTE_REFRESH_MS = 30_000;
export const BUSY_RETRY_MS = 20_000;

// ---------------------------------------------------------------- tokens

let tokenCache: Promise<TokenList> | undefined;

export function useTokens(): { list?: TokenList; error?: string; retry: () => void } {
  const [list, setList] = useState<TokenList>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let live = true;
    tokenCache ??= fetchTokens();
    tokenCache.then(
      (l) => {
        if (!live) return;
        setList(l);
        setError(undefined);
      },
      (e: Error) => {
        tokenCache = undefined;
        if (live) setError(e.message);
      },
    );
    return () => {
      live = false;
    };
  }, [attempt]);
  return { list, error, retry: () => setAttempt((a) => a + 1) };
}

// ---------------------------------------------------------------- price

export type PriceState =
  | { status: 'idle' }
  | { status: 'invalid'; error: ZecSideError }
  | { status: 'loading'; previous?: CombinedPrice }
  | { status: 'ok'; price: CombinedPrice; quote: ExitQuote; recipientChecked: boolean }
  | { status: 'error'; error: OneClickError; previous?: CombinedPrice };

type Result = { key: string } & ({ ok: true; price: CombinedPrice; quote: ExitQuote; recipientChecked: boolean } | { ok: false; error: OneClickError });

export function usePrice(payZats: bigint | undefined, token: Token | undefined, address: string, terms: MakerTerms | undefined): PriceState & { refresh: () => void } {
  const [tick, setTick] = useState(0);
  const [result, setResult] = useState<Result>();
  const [lastOk, setLastOk] = useState<CombinedPrice>();

  const addressOk = token ? validateDestinationAddress(token.blockchain, address).ok : false;
  const recipient = token ? (addressOk ? address.trim() : quoteSampleAddress(token.blockchain)) : undefined;
  const zec = useMemo(() => (terms && payZats !== undefined ? computeZecSide(payZats, terms) : undefined), [terms, payZats]);
  const key = zec?.ok && token && recipient ? `${zec.value.payZats}|${token.assetId}|${recipient}|${tick}` : undefined;

  useEffect(() => {
    if (!key || !zec?.ok || !token || !recipient) return;
    const ctrl = new AbortController();
    let retry: ReturnType<typeof setTimeout> | undefined;
    const zecSide = zec.value;
    const checked = addressOk;
    const timer = setTimeout(() => {
      fetchExitQuote({ amountIn: zecSide.usdcExpected, destinationAsset: token.assetId, recipient, refundTo: DRY_REFUND_ACCOUNT, dry: true }, ctrl.signal).then(
        (quote) => {
          const price = combine(zecSide, quote);
          setLastOk(price);
          setResult({ key, ok: true, price, quote, recipientChecked: checked });
        },
        (e) => {
          if ((e as Error).name === 'AbortError') return;
          const error = e instanceof OneClickError ? e : new OneClickError((e as Error).message, 0, 'other');
          setResult({ key, ok: false, error });
          // "No price for this pair" clears within about 20 s: ask again then.
          if (error.kind === 'busy' || error.kind === 'network') retry = setTimeout(() => setTick((t) => t + 1), BUSY_RETRY_MS);
        },
      );
    }, 450);
    const refresh = setTimeout(() => setTick((t) => t + 1), QUOTE_REFRESH_MS);
    return () => {
      ctrl.abort();
      clearTimeout(timer);
      clearTimeout(refresh);
      clearTimeout(retry);
    };
    // The key covers every input of the request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  const refresh = useCallback(() => setTick((t) => t + 1), []);
  let state: PriceState;
  if (!zec || !token || !recipient) state = { status: 'idle' };
  else if (!zec.ok) state = { status: 'invalid', error: zec.error };
  else if (!result || result.key !== key) state = { status: 'loading', previous: lastOk };
  else if (result.ok) state = { status: 'ok', price: result.price, quote: result.quote, recipientChecked: result.recipientChecked };
  else state = { status: 'error', error: result.error, previous: lastOk };
  return { ...state, refresh };
}
