// Page settings read from the URL. They matter only with the simulated engine.
//   ?speed=20         run the simulated swap 20 times faster
//   ?fail=quote-low   inject a failure (see FAILURES below)
//   ?cpu=1            make the worker busy during simulated proofs, as the real prover is
//   ?pause=committing stop at a stage until "Skip this wait" is pressed
//   ?directory=URL    read the maker directory from another URL
import { SIM_FAILURES, STAGES, type SimFailure, type SimOptions, type Stage } from '../engine/api';

/** Failures the page itself injects (into the 1Click exit quote), on top of the engine's. */
export const PAGE_FAILURES = ['exit-price'] as const;
export type PageFailure = (typeof PAGE_FAILURES)[number];

export const FAILURES: { id: SimFailure | PageFailure; label: string }[] = [
  { id: 'quote-low', label: 'Maker quotes below your minimum' },
  { id: 'no-quote', label: 'Maker never answers' },
  { id: 'maker-quiet', label: 'Maker refunds instead of setting Ready' },
  { id: 'maker-gone', label: 'Maker disappears after funding' },
  { id: 'exit-fail', label: 'Delivery through NEAR Intents fails once' },
  { id: 'exit-price', label: 'Delivery price falls below the minimum' },
];

export const SPEEDS = [1, 5, 20, 100];

export interface PageSettings {
  sim: SimOptions;
  pageFailure?: PageFailure;
}

export function readSettings(search = location.search): PageSettings {
  const q = new URLSearchParams(search);
  const speedRaw = Number(q.get('speed') ?? '1');
  const speed = Number.isFinite(speedRaw) && speedRaw > 0 && speedRaw <= 10_000 ? speedRaw : 1;
  const failRaw = q.get('fail') ?? undefined;
  const fail = (SIM_FAILURES as readonly string[]).includes(failRaw ?? '') ? (failRaw as SimFailure) : undefined;
  const pageFailure = (PAGE_FAILURES as readonly string[]).includes(failRaw ?? '') ? (failRaw as PageFailure) : undefined;
  const pauseRaw = q.get('pause') ?? '';
  const pauseAt = (STAGES as readonly string[]).includes(pauseRaw) ? (pauseRaw as Stage) : undefined;
  return { sim: { speed, fail, burnCpu: q.get('cpu') === '1', pauseAt }, pageFailure };
}

export function settingsUrl(change: { speed?: number; fail?: string | null }): string {
  const url = new URL(location.href);
  if (change.speed !== undefined) {
    if (change.speed === 1) url.searchParams.delete('speed');
    else url.searchParams.set('speed', String(change.speed));
  }
  if (change.fail !== undefined) {
    if (change.fail) url.searchParams.set('fail', change.fail);
    else url.searchParams.delete('fail');
  }
  return url.toString();
}
