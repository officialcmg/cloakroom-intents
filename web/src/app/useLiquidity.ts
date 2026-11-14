// The maker's liquidity on the swap form: read from NEAR when the form opens, every minute
// while it is open, and when the tab is shown again (at most once per 30 s). A failed read
// shows nothing and blocks nothing. The simulated demo shows the fixed value from its
// directory and reads no network.
import { useEffect, useState } from 'react';
import type { MakerDirectory, MakerTerms } from '../lib/directory';
import { LIQUIDITY_MIN_INTERVAL_MS, LIQUIDITY_REFRESH_MS, readMakerLiquidity } from '../lib/liquidity';
import { nearRpcs, type NetworkConfig } from '../lib/netconfig';

export interface Liquidity {
  /** micro-USDC the maker can pay; undefined until read, or after a failed read. */
  microUsdc?: bigint;
  /** The value is the demo's fixed number, not a read. */
  simulated?: boolean;
  /** Whether the page reads (or shows) a value at all: the line keeps its place while it loads. */
  enabled: boolean;
}

export function useMakerLiquidity(network: NetworkConfig | undefined, directory: MakerDirectory | undefined, terms: MakerTerms | undefined): Liquidity {
  const [microUsdc, setMicroUsdc] = useState<bigint>();
  const maker = directory?.maker.nearAccount;
  const rpcKey = network ? nearRpcs(network).join(' ') : '';
  const verifier = network?.near.verifier;
  const token = network?.near.token;
  const decimals = network?.near.tokenDecimals ?? 6;

  useEffect(() => {
    if (!network || !maker || !verifier || !token) return;
    let live = true;
    let last = 0;
    let ctl: AbortController | undefined;
    const read = () => {
      const t = Date.now();
      if (t - last < LIQUIDITY_MIN_INTERVAL_MS) return;
      last = t;
      ctl?.abort();
      ctl = new AbortController();
      readMakerLiquidity({ rpcs: rpcKey.split(' '), verifier, token, maker, tokenDecimals: decimals }, fetch, ctl.signal).then(
        (v) => live && setMicroUsdc(v),
        () => live && setMicroUsdc(undefined),
      );
    };
    read();
    const timer = setInterval(read, LIQUIDITY_REFRESH_MS);
    const onShow = () => document.visibilityState === 'visible' && read();
    document.addEventListener('visibilitychange', onShow);
    window.addEventListener('focus', onShow);
    return () => {
      live = false;
      ctl?.abort();
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onShow);
      window.removeEventListener('focus', onShow);
    };
    // `network` is listed by its parts: a new object with the same values does not read again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rpcKey, maker, verifier, token, decimals]);

  if (!network) {
    const demo = terms?.demoLiquidityMicroUsdc;
    return demo !== undefined ? { microUsdc: demo, simulated: true, enabled: true } : { enabled: false };
  }
  return { microUsdc, enabled: !!maker };
}
