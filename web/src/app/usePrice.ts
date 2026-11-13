// The live price on the swap form: maker side from the directory, exit side from a dry
// 1Click quote. Quotes refresh every 30 seconds while the form is open.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { MakerTerms } from '../lib/directory';
import { DRY_REFUND_ACCOUNT, fetchExitQuote, fetchTokens, OneClickError, type ExitQuote, type Token, type TokenList } from '../lib/oneclick';
import { combine, computeZecSide, type CombinedPrice, type ZecSideError } from '../lib/price';
import { quoteSampleAddress, validateDestinationAddress } from '../lib/address/chains';

export const QUOTE_REFRESH_MS = 30_000;

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
      (l) => live && (setList(l), setError(undefined)),
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

export function usePrice(payZats: bigint | undefined, token: Token | undefined, address: string, terms: MakerTerms | undefined): PriceState & { refresh: () => void } {
  const [state, setState] = useState<PriceState>({ status: 'idle' });
  const [tick, setTick] = useState(0);
  const last = useRef<CombinedPrice | undefined>(undefined);

  const addressOk = token ? validateDestinationAddress(token.blockchain, address).ok : false;
  const recipient = token ? (addressOk ? address.trim() : quoteSampleAddress(token.blockchain)) : undefined;

  useEffect(() => {
    if (!terms || payZats === undefined || !token || !recipient) {
      setState({ status: 'idle' });
      return;
    }
    const zec = computeZecSide(payZats, terms);
    if (!zec.ok) {
      setState({ status: 'invalid', error: zec.error });
      return;
    }
    const ctrl = new AbortController();
    setState({ status: 'loading', previous: last.current });
    const timer = setTimeout(() => {
      fetchExitQuote({ amountIn: zec.value.usdcExpected, destinationAsset: token.assetId, recipient, refundTo: DRY_REFUND_ACCOUNT, dry: true }, ctrl.signal).then(
        (quote) => {
          const price = combine(zec.value, quote);
          last.current = price;
          setState({ status: 'ok', price, quote, recipientChecked: addressOk });
        },
        (e) => {
          if ((e as Error).name === 'AbortError') return;
          const err = e instanceof OneClickError ? e : new OneClickError((e as Error).message, 0, 'other');
          setState({ status: 'error', error: err, previous: last.current });
        },
      );
    }, 450);
    const refresh = setTimeout(() => setTick((t) => t + 1), QUOTE_REFRESH_MS);
    return () => {
      ctrl.abort();
      clearTimeout(timer);
      clearTimeout(refresh);
    };
    // `addressOk` is derived from `recipient`; listing it would refetch for nothing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [payZats, token?.assetId, recipient, terms, tick]);

  const refresh = useCallback(() => setTick((t) => t + 1), []);
  return { ...state, refresh };
}
