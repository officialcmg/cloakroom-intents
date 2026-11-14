// Small shared pieces: logo, notices, copy buttons, hashes, progress bars, amounts.
import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { Icon, type IconName } from './Icon';

export function Logo() {
  const id = `logo-${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`;
  return (
    <span className="logo">
      <svg className="logo-mark" width="30" height="30" viewBox="0 0 32 32" aria-hidden="true">
        <defs>
          <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#2a8f72" />
            <stop offset="1" stopColor="#0d4f3e" />
          </linearGradient>
          <mask id={`${id}-m`}>
            <rect width="32" height="32" fill="#fff" />
            <circle cx="16" cy="10.6" r="1.7" fill="#000" />
            <path d="M16 15.2 18.6 26H13.4Z" fill="#000" />
          </mask>
        </defs>
        <rect width="32" height="32" rx="9.6" fill={`url(#${id})`} />
        <rect x="0.5" y="0.5" width="31" height="31" rx="9.1" fill="none" stroke="#fff" strokeOpacity="0.18" />
        {/* An open cape, held by one clasp: the cloak in the name. */}
        <path mask={`url(#${id}-m)`} fill="#fff" stroke="#fff" strokeWidth="1.2" strokeLinejoin="round" d="M12.8 7.2H19.2C19.6 13 21.8 19 24.8 24.8 22 25.4 19 25.6 16 25.6S10 25.4 7.2 24.8C10.2 19 12.4 13 12.8 7.2Z" />
      </svg>
      <span className="logo-word">
        Cloakroom<span className="logo-word-2"> Intents</span>
      </span>
    </span>
  );
}

export type Tone = 'neutral' | 'ok' | 'info' | 'warn' | 'danger';

const TONE_ICON: Record<Tone, IconName> = { neutral: 'info', ok: 'check', info: 'info', warn: 'clock', danger: 'alert' };

export function Notice({ tone = 'neutral', title, children, icon, role }: { tone?: Tone; title?: ReactNode; children?: ReactNode; icon?: IconName; role?: 'status' | 'alert' }) {
  return (
    <div className={`notice notice-${tone}`} role={role}>
      <Icon name={icon ?? TONE_ICON[tone]} className="notice-icon" />
      <div className="notice-body">
        {title && <p className="notice-title">{title}</p>}
        {children && <div className="notice-text">{children}</div>}
      </div>
    </div>
  );
}

export function useCopy(): [boolean, (text: string) => Promise<void>] {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Fallback for browsers that refuse the clipboard API.
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      document.execCommand('copy');
      ta.remove();
    }
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1800);
  };
  return [copied, copy];
}

export function CopyButton({ text, label, compact }: { text: string; label: string; compact?: boolean }) {
  const [copied, copy] = useCopy();
  return (
    <button type="button" className={compact ? 'btn btn-ghost btn-sm' : 'btn btn-secondary btn-sm'} onClick={() => copy(text)} aria-label={compact ? `Copy ${label}` : undefined}>
      <Icon name={copied ? 'check' : 'copy'} size={16} />
      {!compact && <span>{copied ? 'Copied' : `Copy ${label}`}</span>}
      <span className="sr-only" aria-live="polite">
        {copied ? `${label} copied` : ''}
      </span>
    </button>
  );
}

export function shorten(hash: string, head = 8, tail = 6): string {
  return hash.length <= head + tail + 1 ? hash : `${hash.slice(0, head)}…${hash.slice(-tail)}`;
}

export function Hash({ value, href, label }: { value: string; href?: string; label: string }) {
  return (
    <span className="hash-row">
      {href ? (
        <a className="hash" href={href} target="_blank" rel="noreferrer noopener" title={value}>
          {shorten(value)}
          <Icon name="external" size={14} />
          <span className="sr-only">{`Open ${label} in an explorer (new tab)`}</span>
        </a>
      ) : (
        <span className="hash" title={value}>
          {shorten(value)}
        </span>
      )}
      <CopyButton text={value} label={label} compact />
    </span>
  );
}

export function useNow(intervalMs = 1000, active = true): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs, active]);
  return now;
}

/** A progress bar the page draws from its own clock. It never claims 100% before the work ends. */
export function TimedProgress({ startedAt, estimatedMs, label }: { startedAt: number; estimatedMs?: number; label: string }) {
  const now = useNow(250);
  const elapsed = Math.max(0, now - startedAt);
  const known = estimatedMs !== undefined && estimatedMs > 0;
  // Ease toward 95%: linear to 85% at the estimate, then slow.
  let pct = 0;
  if (known) {
    const r = elapsed / estimatedMs;
    pct = r < 1 ? r * 85 : 85 + 10 * (1 - Math.exp(-(r - 1) * 2));
  }
  const left = known ? Math.max(0, Math.ceil((estimatedMs - elapsed) / 1000)) : undefined;
  return (
    <div className="timed">
      <div className="timed-head">
        <span className="timed-label">{label}</span>
        {left !== undefined && <span className="timed-left">{left > 0 ? `about ${formatDuration(left * 1000)}` : 'almost done'}</span>}
      </div>
      <div
        className={known ? 'bar' : 'bar bar-indeterminate'}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={known ? Math.round(pct) : undefined}
      >
        <div className="bar-fill" style={known ? { width: `${pct}%` } : undefined} />
      </div>
    </div>
  );
}

export function formatDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${Math.max(1, s)} s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return `${h} h ${m % 60} min`;
}

export function formatClock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}

export function Spinner({ label }: { label?: string }) {
  return (
    <span className="spinner" role={label ? 'status' : undefined}>
      <span className="spinner-dot" aria-hidden="true" />
      {label && <span className="sr-only">{label}</span>}
    </span>
  );
}
