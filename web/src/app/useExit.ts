// The exit step on the page side. When the USDC is in NEAR Intents (stage paid_out), the
// page fetches a fresh 1Click quote and hands it to the engine, which signs the intent.
// In exit_retry the user decides: try again, or accept a lower amount.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineClient, ExitRequest, SwapState } from '../engine';
import { fetchExitQuote, OneClickError, type ExitQuote } from '../lib/oneclick';
import type { PageFailure } from './settings';

export interface ExitControl {
  busy: boolean;
  error?: string;
  retry: () => void;
  acceptLower: () => void;
}

const RETRY_MS = 15_000;

export function useExit(engine: EngineClient | undefined, swap: SwapState | null, simulated: boolean, pageFailure?: PageFailure): ExitControl {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const inFlight = useRef(false);
  const lowered = useRef(false);

  const run = useCallback(
    async (mode: 'auto' | 'retry' | 'accept') => {
      if (!engine || !swap?.payout || inFlight.current) return;
      inFlight.current = true;
      setBusy(true);
      setError(undefined);
      try {
        let quote: ExitQuote = await fetchExitQuote({
          amountIn: BigInt(swap.payout.usdc),
          destinationAsset: swap.params.destination.assetId,
          recipient: swap.params.destination.address,
          refundTo: swap.payout.intentsAccount,
          // The simulated engine moves no money, so it gets dry quotes.
          dry: simulated,
        });
        if (pageFailure === 'exit-price' && simulated && !lowered.current) {
          lowered.current = true;
          quote = { ...quote, amountOut: (quote.amountOut * 85n) / 100n, minAmountOut: (quote.minAmountOut * 85n) / 100n };
        }
        const offered = swap.exit?.offered;
        const request: ExitRequest = {
          quote: {
            amountIn: quote.amountIn.toString(),
            amountOut: quote.amountOut.toString(),
            minAmountOut: quote.minAmountOut.toString(),
            depositAddress: quote.depositAddress,
            deadline: quote.deadline,
          },
          // Accept only what the user saw: if the new minimum is lower still, ask again.
          acceptBelowMinimum: mode === 'accept' && !!offered && quote.minAmountOut >= BigInt(offered.minAmountOut),
        };
        await engine.exit(request);
      } catch (e) {
        const msg = e instanceof OneClickError ? `NEAR Intents did not give a price: ${e.message}` : (e as Error).message;
        setError(msg);
      } finally {
        inFlight.current = false;
        setBusy(false);
      }
    },
    [engine, swap, simulated, pageFailure],
  );

  // Start the exit by itself as soon as the USDC is paid out, and retry while it fails.
  const stage = swap?.stage;
  useEffect(() => {
    if (stage !== 'paid_out') return;
    const first = setTimeout(() => void run('auto'), 0);
    const id = setInterval(() => void run('auto'), RETRY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(id);
    };
    // Run once per arrival in paid_out, not on every state update.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stage, swap?.id]);

  return { busy, error, retry: () => void run('retry'), acceptLower: () => void run('accept') };
}
