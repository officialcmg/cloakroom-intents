// The exit from the page's side. The engine runs the exit by itself once the USDC is paid
// out: firm quote, safety check, signature, submission, status. The page only acts in
// exit_retry, where the user decides: try again, or accept a lower minimum.
import { useCallback, useState } from 'react';
import type { EngineClient, SwapState } from '../engine';

export interface ExitControl {
  busy: boolean;
  error?: string;
  retry: () => void;
  acceptLower: () => void;
}

export function useExit(engine: EngineClient | undefined, swap: SwapState | null): ExitControl {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const run = useCallback(
    async (acceptMinimum?: string) => {
      if (!engine || busy) return;
      setBusy(true);
      setError(undefined);
      try {
        await engine.exit(acceptMinimum ? { acceptMinimum } : {});
      } catch (e) {
        setError((e as Error).message);
      } finally {
        setBusy(false);
      }
    },
    [engine, busy],
  );

  const offered = swap?.exit?.offered?.minAmountOut;
  return { busy, error, retry: () => void run(), acceptLower: () => void run(offered) };
}
