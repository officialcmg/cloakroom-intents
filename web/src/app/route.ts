// Which page this URL shows: the landing page or the swap app.
//
//   /                      the landing page
//   /swap                  the swap app (the engine starts here, never on the landing page)
//   /?demo=1, /?config=…   the swap app: links that name an app setting keep working at /
//
// Moving between the two uses the History API, so the engine keeps running when the user
// goes back to the landing page in the middle of a swap.
import { useSyncExternalStore } from 'react';

export type Route = 'landing' | 'app';

export const APP_PATH = '/swap';

/** URL parameters that only the swap app reads (see settings.ts, netconfig.ts, directory.ts). */
export const APP_PARAMS: readonly string[] = ['demo', 'config', 'speed', 'fail', 'pause', 'cpu', 'directory'];

export function routeFor(pathname: string, search: string): Route {
  const path = pathname.replace(/\/+$/, '') || '/';
  if (path === APP_PATH) return 'app';
  const q = new URLSearchParams(search);
  if (APP_PARAMS.some((p) => q.has(p))) return 'app';
  return 'landing';
}

const listeners = new Set<() => void>();
const current = () => routeFor(location.pathname, location.search);

function subscribe(l: () => void) {
  listeners.add(l);
  window.addEventListener('popstate', l);
  return () => {
    listeners.delete(l);
    window.removeEventListener('popstate', l);
  };
}

export function useRoute(): Route {
  return useSyncExternalStore(subscribe, current, () => 'landing');
}

/** Goes to `href` on this site without a page load. */
export function navigate(href: string): void {
  if (href === location.pathname + location.search) return;
  history.pushState(null, '', href);
  for (const l of listeners) l();
  window.scrollTo({ top: 0 });
}

/** Click handler for a same-site link: a plain click navigates in place, a modified click opens as usual. */
export function onLinkClick(e: React.MouseEvent<HTMLAnchorElement>): void {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  e.preventDefault();
  navigate(e.currentTarget.getAttribute('href') ?? '/');
}

// A browser that holds a swap: set by the app while the engine reports one, so the landing
// page can offer to open it without starting the engine.
const SWAP_MARK = 'zecret.hasSwap';

export function markSwap(present: boolean): void {
  try {
    if (present) localStorage.setItem(SWAP_MARK, '1');
    else localStorage.removeItem(SWAP_MARK);
  } catch {
    // Storage refused: the landing page then does not offer to open the swap.
  }
}

export function hasMarkedSwap(): boolean {
  try {
    return localStorage.getItem(SWAP_MARK) === '1';
  } catch {
    return false;
  }
}
