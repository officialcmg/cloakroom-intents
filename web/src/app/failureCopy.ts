// What the page says when it cannot start: a title and a next step that match the real
// cause, and the technical message as a detail. The causes come from useZecret.ts (kind)
// and from the engine's INIT_FAILED messages (src/engine/real/realEngine.ts, init).
import type { AppState } from './useZecret';

export type FailureCause =
  | 'config_refused'
  | 'config'
  | 'directory'
  | 'maker_unverified'
  | 'maker_unreachable'
  | 'engine_download'
  | 'engine_crashed'
  | 'engine';

export interface FailureCopy {
  cause: FailureCause;
  title: string;
  /** What happened and where the money is, in plain words. */
  body: string;
  /** What the user can do next. */
  next: string;
  /** The technical message, shown smaller. */
  detail?: string;
  /** Whether "Try again" can help. */
  retry: boolean;
}

/** The engine's prefix for a failed check of the maker's signed directory entry. */
export const MAKER_CHECK_PREFIX = 'The maker’s directory entry did not pass the check';
const MAKER_CHECK_PREFIX_ASCII = "The maker's directory entry did not pass the check";

/** Messages that describe the maker's directory entry, not this browser. */
const MAKER_ENTRY = [/^The maker directory is not valid/, /^The maker directory is for /, /^The maker names the escrow /, /^The maker directory has no signed entry/];

/** A network problem, not a wrong answer: the check could not ask NEAR. */
// The engine's transport says "no answer (network error or CORS)" when no HTTP answer
// arrived, and "HTTP <status>" for an answer that is not JSON (crates/zecret-wasm/src/web.rs,
// crates/zecret-core/src/near/rpc.rs).
const NETWORK = /no answer|network ?error|failed to fetch|timed? ?out|timeout|HTTP (5\d\d|429)\b|offline/i;

/**
 * `hasSwap`: this browser holds a swap (see route.ts, markSwap). Then money may already
 * have moved, and the page says where the swap is instead of "nothing was sent".
 */
export function failureCopy(failure: AppState['failure'], { hasSwap = false }: { hasSwap?: boolean } = {}): FailureCopy {
  const NOTHING_SENT = hasSwap ? 'Your swap is stored in this browser. It continues when the page starts again.' : 'Nothing was sent and no money moved.';
  const message = failure?.message ?? '';
  const kind = failure?.kind ?? 'engine';

  if (kind === 'config') {
    if (/loads only its own network settings/.test(message))
      return {
        cause: 'config_refused',
        title: 'This link asks for network settings this site does not use',
        body: `This site runs only with its own network settings. A link cannot choose others. ${NOTHING_SENT}`,
        next: 'Open the site without the "config" part of the link.',
        detail: message,
        retry: false,
      };
    return {
      cause: 'config',
      title: 'The network settings did not load',
      body: `The page reads its network settings from a small file on this site. ${NOTHING_SENT}`,
      next: 'Check your connection and try again.',
      detail: message,
      retry: true,
    };
  }

  if (kind === 'directory')
    return {
      cause: 'directory',
      title: 'The maker’s details did not load',
      body: `The page reads the maker’s address and price from a small public file. ${NOTHING_SENT}`,
      next: 'Check your connection and try again.',
      detail: message,
      retry: true,
    };

  const makerCheck = message.startsWith(MAKER_CHECK_PREFIX) || message.startsWith(MAKER_CHECK_PREFIX_ASCII);
  if (makerCheck || MAKER_ENTRY.some((r) => r.test(message))) {
    const why = makerCheck ? message.slice(MAKER_CHECK_PREFIX.length).replace(/^:\s*/, '') : message;
    if (makerCheck && NETWORK.test(why))
      return {
        cause: 'maker_unreachable',
        title: 'The maker could not be checked',
        body: `Before a swap, this page checks the maker’s signed details on NEAR. NEAR did not answer. ${NOTHING_SENT}`,
        next: 'Check your connection and try again.',
        detail: message,
        retry: true,
      };
    return {
      cause: 'maker_unverified',
      title: 'The maker could not be verified',
      body: `Before a swap, this page checks the maker’s signed details against its NEAR account. The check failed, and this page does not swap with a maker it cannot verify. ${NOTHING_SENT}`,
      next: 'Try again later. The maker may be updating its details.',
      detail: message,
      retry: true,
    };
  }

  if (/^The WebAssembly engine did not load/.test(message))
    return {
      cause: 'engine_download',
      title: 'The swap engine did not load',
      body: `The swap engine is a large download (about 9 MB). It did not arrive or did not start. ${NOTHING_SENT}`,
      next: 'Check your connection and try again. If it fails again, this browser may block WebAssembly: try another browser, or a normal window instead of a private one.',
      detail: message,
      retry: true,
    };

  if (/^The swap engine stopped again/.test(message))
    return {
      cause: 'engine_crashed',
      title: 'The swap engine stopped',
      body: 'The engine in this tab stopped several times in ten minutes. A swap in progress stays stored in this browser.',
      next: 'Reload the page to continue. If it keeps stopping, restore the swap from its recovery file in another browser.',
      detail: message,
      retry: true,
    };

  return {
    cause: 'engine',
    title: 'The swap engine could not start',
    body: `The swap runs in this browser, in a background worker. The worker did not start. ${NOTHING_SENT}`,
    next: 'This browser may block Web Workers, WebAssembly or storage, for example in some private modes. Try another browser or a normal window.',
    detail: message || undefined,
    retry: true,
  };
}
