// Elements with data-reveal fade and rise into place the first time they scroll into view.
// Nothing is hidden when the user asks for reduced motion, or when the browser has no
// IntersectionObserver: the class that hides them is set only after the observer runs.
import { useEffect } from 'react';

export function useReveal(): void {
  useEffect(() => {
    if (typeof IntersectionObserver === 'undefined' || !matchMedia('(prefers-reduced-motion: no-preference)').matches) return;
    const root = document.documentElement;
    const items = [...document.querySelectorAll<HTMLElement>('[data-reveal]')];
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          e.target.classList.add('is-in');
          io.unobserve(e.target);
        }
      },
      { rootMargin: '0px 0px -8% 0px', threshold: 0.08 },
    );
    // Elements already on screen show at once, without the effect.
    const h = innerHeight;
    for (const el of items) {
      if (el.getBoundingClientRect().top < h) el.classList.add('is-in');
      else io.observe(el);
    }
    root.classList.add('reveal-on');
    return () => {
      io.disconnect();
      root.classList.remove('reveal-on');
    };
  }, []);
}
