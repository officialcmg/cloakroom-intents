import { useEffect, useId, useState } from 'react';
import { chainInfo } from '../lib/address/chains';
import { CHAIN_ICONS } from './chainIconData';

type TokenIconModule = typeof import('./tokenIconData');

// The token logos are a separate chunk: it loads the first time a token tile renders.
let loaded: TokenIconModule | undefined;
let loading: Promise<TokenIconModule> | undefined;

function loadTokenIcons(): Promise<TokenIconModule> {
  loading ??= import('./tokenIconData').then((m) => (loaded = m));
  return loading;
}

function useTokenIcons(): TokenIconModule | undefined {
  const [mod, setMod] = useState(loaded);
  useEffect(() => {
    if (mod) return;
    let live = true;
    loadTokenIcons().then(
      (m) => live && setMod(m),
      () => {
        // A failed load keeps the letter tiles; the next tile that renders tries again.
        loading = undefined;
      },
    );
    return () => {
      live = false;
    };
  }, [mod]);
  return mod;
}

function hue(s: string): number {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

/** A coloured initial: the tile for a token that has no logo in the bundled sets. */
export function TokenBadge({ symbol, size = 28 }: { symbol: string; size?: number }) {
  return (
    <span className="token-badge" style={{ width: size, height: size, ['--h' as string]: hue(symbol) }} aria-hidden="true">
      {symbol.replace(/[^A-Za-z0-9]/g, '').slice(0, 1).toUpperCase()}
    </span>
  );
}

/**
 * Gives the ids in a logo's markup a prefix for this copy of the logo, so that a page with the
 * same logo in many rows still has unique ids.
 */
export function scopeIds(body: string, scope: string): string {
  return body.replace(/(\bid="|url\(#|href="#)/g, `$1${scope}`);
}

/**
 * A token's logo on a round tile, with its chain's logo in the bottom-right corner.
 * Decorative: the symbol and the chain name are always written next to it. A token with no
 * logo in the bundled sets keeps the coloured letter tile; the tile has the same size before
 * and after the logos load, so nothing moves.
 */
export function TokenAvatar({ symbol, blockchain, size = 28 }: { symbol: string; blockchain: string; size?: number }) {
  const icons = useTokenIcons();
  const icon = icons?.tokenIcon(symbol);
  const chainName = chainInfo(blockchain)?.name;
  const chain = chainName ? CHAIN_ICONS[chainName] : undefined;
  const scope = `a${useId().replace(/[^A-Za-z0-9_-]/g, '')}-`;
  return (
    <span className="token-avatar" style={{ width: size, height: size }} aria-hidden="true">
      {icon ? (
        <span className="token-logo">
          <svg viewBox={icon.viewBox} focusable="false" dangerouslySetInnerHTML={{ __html: scopeIds(icon.body, scope) }} />
        </span>
      ) : (
        <TokenBadge symbol={symbol} size={size} />
      )}
      {chain && (
        <span className="token-avatar-chain">
          <svg viewBox={chain.viewBox} focusable="false" dangerouslySetInnerHTML={{ __html: scopeIds(chain.body, scope) }} />
        </span>
      )}
    </span>
  );
}
