// Local UI preferences that survive reloads (theme, last token). No engine import, so the
// landing page can use them.
import { useState } from 'react';

export function usePref<T extends string>(key: string, initial: T): [T, (v: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      return (localStorage.getItem(`zecret.${key}`) as T | null) ?? initial;
    } catch {
      return initial;
    }
  });
  const set = (v: T) => {
    setValue(v);
    try {
      localStorage.setItem(`zecret.${key}`, v);
    } catch {
      // Storage refused: the preference lasts for this visit only.
    }
  };
  return [value, set];
}
