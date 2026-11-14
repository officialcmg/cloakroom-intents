// A small deterministic timeline. Nothing here reads the clock: every frame is a pure
// function of the film time t (seconds), so a frame renders the same way every time.
(function () {
  const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
  const lerp = (a, b, p) => a + (b - a) * p;

  // cubic-bezier as in CSS, solved by Newton steps with a bisection fallback.
  function bezier(x1, y1, x2, y2) {
    const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
    const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
    const sx = (u) => ((ax * u + bx) * u + cx) * u;
    const sy = (u) => ((ay * u + by) * u + cy) * u;
    const dx = (u) => (3 * ax * u + 2 * bx) * u + cx;
    return (p) => {
      if (p <= 0) return 0;
      if (p >= 1) return 1;
      let u = p;
      for (let i = 0; i < 8; i++) {
        const e = sx(u) - p;
        if (Math.abs(e) < 1e-6) return sy(u);
        const d = dx(u);
        if (Math.abs(d) < 1e-6) break;
        u -= e / d;
      }
      let lo = 0, hi = 1;
      u = p;
      for (let i = 0; i < 30; i++) {
        const v = sx(u);
        if (Math.abs(v - p) < 1e-6) break;
        if (v < p) lo = u; else hi = u;
        u = (lo + hi) / 2;
      }
      return sy(u);
    };
  }

  const ease = {
    linear: (p) => p,
    out: bezier(0.2, 0.7, 0.2, 1), // --ease in tokens.css
    spring: bezier(0.22, 1.2, 0.36, 1), // --ease-spring in tokens.css
    outCubic: (p) => 1 - Math.pow(1 - p, 3),
    outQuart: (p) => 1 - Math.pow(1 - p, 4),
    outQuint: (p) => 1 - Math.pow(1 - p, 5),
    outExpo: (p) => (p >= 1 ? 1 : 1 - Math.pow(2, -10 * p)),
    inCubic: (p) => p * p * p,
    inQuart: (p) => p * p * p * p,
    inOutCubic: (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2),
    inOutQuint: (p) => (p < 0.5 ? 16 * Math.pow(p, 5) : 1 - Math.pow(-2 * p + 2, 5) / 2),
    inOutSine: (p) => -(Math.cos(Math.PI * p) - 1) / 2,
    camera: bezier(0.65, 0, 0.25, 1),
  };

  /** Eased progress of t through [a, a + d]. */
  const prog = (t, a, d, e = ease.out) => e(clamp((t - a) / d));

  /** Sets opacity, transform and blur on an element. */
  function T(el, { x = 0, y = 0, s = 1, sx, sy, r = 0, o, blur, origin } = {}) {
    if (!el) return;
    const sc = sx !== undefined || sy !== undefined ? `scale(${sx ?? s},${sy ?? s})` : `scale(${s})`;
    el.style.transform = `translate3d(${x.toFixed(2)}px,${y.toFixed(2)}px,0) ${sc} rotate(${r.toFixed(3)}deg)`;
    if (o !== undefined) el.style.opacity = clamp(o).toFixed(4);
    if (blur !== undefined) el.style.filter = blur > 0.05 ? `blur(${blur.toFixed(2)}px)` : 'none';
    if (origin) el.style.transformOrigin = origin;
  }

  /** The standard appear: up, unblur, fade in. p in 0..1 (already eased). */
  const appear = (el, p, { dy = 28, blur = 10, s0 = 1 } = {}) =>
    T(el, { y: (1 - p) * dy, o: p, blur: (1 - p) * blur, s: lerp(s0, 1, p) });

  /** Wraps the words of el in masks; *text* becomes the serif italic. Returns the inner spans. */
  function words(el) {
    const src = el.getAttribute('data-text') ?? el.textContent;
    el.textContent = '';
    const out = [];
    let serif = false;
    for (const raw of src.split(' ')) {
      let w = raw;
      const startSerif = w.startsWith('*');
      if (startSerif) { serif = true; w = w.slice(1); }
      const endSerif = w.endsWith('*') || w.endsWith('*.') || w.endsWith('*,');
      let tail = '';
      if (endSerif) {
        const i = w.lastIndexOf('*');
        tail = w.slice(i + 1);
        w = w.slice(0, i);
      }
      const mask = document.createElement('span');
      mask.className = 'wmask';
      const inner = document.createElement('span');
      inner.className = 'winner' + (serif ? ' serif' : '');
      inner.textContent = w;
      mask.appendChild(inner);
      if (tail) {
        const t2 = document.createElement('span');
        t2.className = 'winner';
        t2.textContent = tail;
        mask.appendChild(t2);
      }
      el.appendChild(mask);
      el.appendChild(document.createTextNode(' '));
      out.push(mask);
      if (endSerif) serif = false;
    }
    return out;
  }

  /** Wraps each character in a span. Returns the spans. */
  function chars(el) {
    const src = el.textContent;
    el.textContent = '';
    return [...src].map((c) => {
      const s = document.createElement('span');
      s.className = 'ch';
      s.textContent = c === ' ' ? ' ' : c;
      el.appendChild(s);
      return s;
    });
  }

  /** Mask reveal of words: each slides up out of its mask, staggered. */
  function revealWords(ws, t, a, { stagger = 0.06, d = 0.9, out } = {}) {
    ws.forEach((w, i) => {
      const p = prog(t, a + i * stagger, d, ease.outQuint);
      let q = 0;
      if (out !== undefined) q = prog(t, out + i * 0.03, 0.5, ease.inCubic);
      const fs = parseFloat(getComputedStyleCached(w));
      for (const c of w.children) T(c, { y: (1 - p) * 1.15 * fs - q * 0.5 * fs, o: Math.min(1, p * 1.4) * (1 - q) });
    });
  }
  const fsCache = new WeakMap();
  function getComputedStyleCached(el) {
    if (!fsCache.has(el)) fsCache.set(el, getComputedStyle(el).fontSize);
    return fsCache.get(el);
  }

  const fmtClock = (s) => {
    s = Math.max(0, Math.floor(s));
    return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
  };

  window.E = { clamp, lerp, bezier, ease, prog, T, appear, words, chars, revealWords, fmtClock };
})();
