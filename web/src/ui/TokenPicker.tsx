import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { Token } from '../lib/oneclick';
import { chainInfo } from '../lib/address/chains';
import { Icon } from './Icon';

export function chainName(id: string): string {
  return chainInfo(id)?.name ?? id;
}

function hue(s: string): number {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

/** A coloured initial instead of a token logo, so the page loads no images from third parties. */
export function TokenBadge({ symbol, size = 28 }: { symbol: string; size?: number }) {
  return (
    <span className="token-badge" style={{ width: size, height: size, ['--h' as string]: hue(symbol) }} aria-hidden="true">
      {symbol.replace(/[^A-Za-z0-9]/g, '').slice(0, 1).toUpperCase()}
    </span>
  );
}

export function TokenPicker({
  tokens,
  value,
  onChange,
  disabled,
  id,
}: {
  tokens: Token[];
  value: Token | undefined;
  onChange: (t: Token) => void;
  disabled?: boolean;
  id?: string;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const listId = useId();

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return tokens;
    const words = q.split(/\s+/);
    // Exact symbol matches first, then symbols that start with a search word, then the rest.
    const score = (t: Token) => {
      const sym = t.symbol.toLowerCase();
      if (words.includes(sym)) return 0;
      if (words.some((w) => sym.startsWith(w))) return 1;
      return 2;
    };
    return tokens
      .filter((t) => {
        const hay = `${t.symbol} ${chainName(t.blockchain)} ${t.blockchain}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      })
      .map((t, i) => ({ t, i, s: score(t) }))
      .sort((x, y) => x.s - y.s || x.i - y.i)
      .map((x) => x.t);
  }, [tokens, query]);

  const chains = useMemo(() => new Set(tokens.map((t) => t.blockchain)).size, [tokens]);

  useEffect(() => {
    setActive(0);
  }, [query]);

  useEffect(() => {
    document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: 'nearest' });
  }, [active, listId]);

  const open = () => {
    setQuery('');
    const i = value ? tokens.findIndex((t) => t.assetId === value.assetId) : 0;
    setActive(Math.max(0, i));
    dialog.current?.showModal();
    requestAnimationFrame(() => {
      input.current?.focus();
      document.getElementById(`${listId}-${Math.max(0, i)}`)?.scrollIntoView({ block: 'center' });
    });
  };
  const close = () => dialog.current?.close();
  const pick = (t: Token) => {
    onChange(t);
    close();
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((a) => Math.min(filtered.length - 1, a + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const t = filtered[active];
      if (t) pick(t);
    }
  };

  return (
    <>
      <button type="button" id={id} className="token-select" onClick={open} disabled={disabled} aria-haspopup="dialog">
        {value ? (
          <>
            <TokenBadge symbol={value.symbol} />
            <span className="token-select-text">
              <span className="token-symbol">{value.symbol}</span>
              <span className="token-chain">on {chainName(value.blockchain)}</span>
            </span>
          </>
        ) : (
          <span className="token-select-text">
            <span className="token-symbol">Choose a token</span>
          </span>
        )}
        <Icon name="chevron" className="token-select-chevron" />
      </button>
      <dialog ref={dialog} className="dialog token-dialog" aria-labelledby={`${listId}-title`} onClick={(e) => e.target === dialog.current && close()}>
        <div className="dialog-head">
          <h2 id={`${listId}-title`} className="dialog-title">
            Choose what you receive
          </h2>
          <button type="button" className="btn btn-ghost btn-icon" onClick={close} aria-label="Close">
            <Icon name="close" />
          </button>
        </div>
        <p className="dialog-sub">
          {tokens.length} tokens on {chains} chains, delivered by NEAR Intents.
        </p>
        <div className="search">
          <Icon name="search" className="search-icon" />
          <input
            ref={input}
            className="input search-input"
            placeholder="Search, for example usdc solana"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={filtered.length ? `${listId}-${active}` : undefined}
            aria-label="Search tokens and chains"
            autoComplete="off"
            spellCheck={false}
          />
        </div>
        <ul className="token-list" role="listbox" id={listId} aria-label="Tokens">
          {filtered.map((t, i) => (
            <li
              key={t.assetId}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              className={`token-option${i === active ? ' is-active' : ''}${value?.assetId === t.assetId ? ' is-current' : ''}`}
              onMouseMove={() => setActive(i)}
              onClick={() => pick(t)}
            >
              <TokenBadge symbol={t.symbol} size={32} />
              <span className="token-option-text">
                <span className="token-symbol">{t.symbol}</span>
                <span className="token-chain">{chainName(t.blockchain)}</span>
              </span>
              {t.price !== undefined && <span className="token-price">${t.price < 1 ? t.price.toPrecision(3) : t.price.toLocaleString(undefined, { maximumFractionDigits: 2 })}</span>}
              {value?.assetId === t.assetId && <Icon name="check" className="token-check" />}
            </li>
          ))}
          {filtered.length === 0 && <li className="token-empty">No token matches “{query}”.</li>}
        </ul>
      </dialog>
    </>
  );
}
