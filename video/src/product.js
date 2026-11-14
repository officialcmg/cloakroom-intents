// Scene 4: the real swap, inside a designed browser frame. The footage, the true time of
// every frame and the page's boxes come from footage.js (written by cut_footage.py).
(function () {
  const { clamp, lerp, ease, prog, T, appear, fmtClock } = E;
  const $ = (id) => document.getElementById(id);
  const F = window.FOOTAGE;
  if (!F) return;
  const B = { x: 210, y: 176, w: 1500, h: 820 }; // the browser frame on the canvas
  const V = { x: B.x, y: B.y + 64, w: B.w, h: B.h - 64 }; // its viewport
  const VW = 1280, VH = 800; // the recording's viewport, CSS pixels
  const LEAD = 0.9; // the footage starts this long after the scene
  const N = F.frames.length;
  const ev = F.events;

  Object.assign($('s4browser').style, { left: B.x + 'px', top: B.y + 'px', width: B.w + 'px', height: B.h + 'px' });
  $('s4urltext').textContent = F.host + '/swap';
  const img = $('s4img');
  const img2 = img.cloneNode();
  img2.id = 's4img2';
  $('s4view').appendChild(img2);

  const boxAt = (ft) => F.boxes[F.frames[clamp(Math.floor(ft * 30), 0, N - 1)].b] || {};
  const centre = (b) => (b ? [b[0] + b[2] / 2, b[1] + b[3] / 2] : null);
  const firstFrame = (pred) => {
    for (const fr of F.frames) if (pred(F.boxes[fr.b] || {}, fr)) return fr.i / 30;
    return null;
  };
  const stepIdx = (re) => (bx) => (bx.steps || []).findIndex((s) => re.test(s.title));
  const lockRe = /locks the USDC/i, claimRe = /claim the USDC/i;
  const tLocked = firstFrame((bx) => { const i = stepIdx(lockRe)(bx); return i >= 0 && /is-done/.test(bx.steps[i].cls); });
  const tClaim = firstFrame((bx) => { const i = stepIdx(claimRe)(bx); return i >= 0 && /is-(current|done)/.test(bx.steps[i].cls); });
  const stepDot = (re) => (bx) => { const i = stepIdx(re)(bx); return i >= 0 ? centre(bx.steps[i].dot) : null; };
  const stepsBox = (ft) => boxAt(ft).stepsCard;

  // Camera keys: [footage time, centre x, centre y, zoom], in the recording's CSS pixels.
  const key = (t, c, z) => (c ? [t, c[0], c[1], z] : null);
  const formC = centre(boxAt(ev.form + 0.3).payAmount || boxAt(ev.form + 0.3).form) || [640, 400];
  const priceC = centre(boxAt(ev.pay - 0.1).price) || formC;
  const qrB = boxAt(ev.pay + 0.5).qr;
  const qrC = qrB ? [qrB[0] + qrB[2] / 2 + 150, qrB[1] + qrB[3] / 2] : [640, 400];
  const progC = [640, 430];
  const sb = stepsBox(ev.progress + 3);
  const stepsC = sb ? [sb[0] + sb[2] / 2 - 60, (tLocked ? stepDot(lockRe)(boxAt(tLocked))?.[1] : null) ?? sb[1] + sb[3] / 2] : progC;
  const claimC = tClaim ? [stepsC[0], stepDot(claimRe)(boxAt(tClaim))?.[1] ?? stepsC[1]] : stepsC;
  const doneC = centre(boxAt(ev.done + 0.8).done) || [640, 300];
  const KEYS = [
    key(0, formC, 1.5),
    key(ev.price + 0.3, [priceC[0], priceC[1] + 20], 1.6),
    key(ev.pay, qrC, 1.6),
    key(ev.progress + 0.4, progC, 1.17),
    key(Math.max(ev.progress + 2.2, (tLocked ?? ev.progress + 3) - 1.4), stepsC, 1.55),
    key(Math.max(ev.progress + 4.5, (tClaim ?? ev.done - 3) - 1.0), claimC, 1.55),
    key(ev.done, [doneC[0], doneC[1] + 60], 1.5),
  ].filter(Boolean).sort((a, b) => a[0] - b[0]);
  const MOVE = 1.1;
  function camAt(ft) {
    let c = KEYS[0];
    for (let i = 1; i < KEYS.length; i++) {
      const k = KEYS[i];
      if (ft >= k[0]) { c = k; continue; }
      const p = ease.camera(clamp((ft - (k[0] - MOVE * 0.35)) / MOVE));
      const prev = c;
      c = [ft, lerp(prev[1], k[1], p), lerp(prev[2], k[2], p), lerp(prev[3], k[3], p)];
      break;
    }
    let [, cx, cy, z] = c;
    const hw = V.w / 2 / z, hh = V.h / 2 / z;
    cx = clamp(cx, hw, VW - hw);
    cy = clamp(cy, hh, VH - hh);
    return { cx, cy, z };
  }
  const toScreen = (cam, p) => [V.x + V.w / 2 + (p[0] - cam.cx) * cam.z, V.y + V.h / 2 + (p[1] - cam.cy) * cam.z];

  // Callouts: text, footage time window, target, where the pill sits relative to the target.
  const CALLOUTS = [
    { n: '01', text: 'Runs in your browser. No install, no account.', a: 0.5, b: ev.price + 0.2, tgt: (bx) => { const c = bx.payAmount || bx.form; return c ? [c[0] + 30, c[1] + c[3] / 2] : null; }, dx: -60, dy: -150, align: 'right' },
    { n: '02', text: 'Pay once from any shielded wallet', a: ev.pay + 0.5, b: ev.progress, tgt: (bx) => (bx.qr ? [bx.qr[0] + bx.qr[2] - 10, bx.qr[1] + bx.qr[3] * 0.3] : null), dx: 120, dy: -170, align: 'left' },
    tLocked != null && { n: '03', text: 'The maker locks USDC first', a: tLocked - 0.6, b: tLocked + 2.6, tgt: stepDot(lockRe), dx: -90, dy: -140, align: 'right' },
    tClaim != null && { n: '04', text: 'Claimed with a key made here', a: tClaim - 0.2, b: ev.done - 0.7, tgt: stepDot(claimRe), dx: -90, dy: 130, align: 'right' },
    { n: '05', text: 'Delivered', a: ev.done + 0.5, b: 99, tgt: (bx) => (bx.done ? [bx.done[0] + bx.done[2] + 22, bx.done[1] + bx.done[3] / 2] : null), dx: 110, dy: -130, align: 'left' },
  ].filter(Boolean);
  $('s4callouts').innerHTML = CALLOUTS.map((c, i) => `<div class="callout" id="s4c${i}"><span class="n">${c.n}</span>${c.text}</div>`).join('') +
    `<div class="callout" id="s4real" style="background:#fff;color:var(--ink);font-size:44px;padding:22px 36px;left:50%;top:930px"><span class="n" style="color:var(--brand)">REAL TIME</span>${Math.floor(F.realSecs / 60)} min ${Math.round(F.realSecs % 60)} s from payment to done</div>`;
  const svgNS = 'http://www.w3.org/2000/svg';
  const leaders = CALLOUTS.map(() => {
    const g = document.createElementNS(svgNS, 'g');
    g.innerHTML = '<path fill="none" stroke="#0a3a2f" stroke-width="2.5" stroke-linecap="round"/><circle r="10" fill="#9fe3c6" stroke="#0a3a2f" stroke-width="3"/><circle r="22" fill="none" stroke="#0a3a2f" stroke-opacity=".25" stroke-width="2"/>';
    $('s4leader').appendChild(g);
    return g;
  });

  let lastSrc = '', lastSrc2 = '';
  window.scene4 = function (t) {
    // The scene rises in over scene 3.
    const r = prog(t, 0, 0.95, ease.inOutQuint);
    $('s4').style.clipPath = r < 1 ? `inset(${(1 - r) * 100}% 0 0 0 round ${lerp(80, 0, r)}px ${lerp(80, 0, r)}px 0 0)` : 'none';
    T($('s4arc'), { y: (1 - r) * 200 - t * 4 });
    T($('s4glow'), { x: t * 8 });
    const bIn = prog(t, 0.25, 1.3, ease.spring);
    T($('s4cam'), { y: (1 - bIn) * 140, s: lerp(0.94, 1, bIn), o: prog(t, 0.25, 0.4), origin: '50% 60%' });
    appear($('s4badge'), prog(t, 0.9, 0.8, ease.outQuint), { dy: 16 });
    const ft = clamp(t - LEAD, 0, N / 30 - 1 / 30);
    const fi = Math.floor(ft * 30 + 1e-6);
    const fr = F.frames[fi];
    const cam = camAt(ft);
    const tf = `translate(${(V.w / 2 - cam.cx * cam.z).toFixed(2)}px,${(V.h / 2 - cam.cy * cam.z).toFixed(2)}px) scale(${(cam.z / 2).toFixed(5)})`;
    img.style.transform = tf;
    img2.style.transform = tf;
    const src = F.dir + String(fr.i).padStart(5, '0') + '.jpg';
    const waits = [];
    if (src !== lastSrc) { img.src = src; lastSrc = src; waits.push(img.decode().catch(() => {})); }
    // A short dissolve where the edit skips time between screens.
    let x = 0, prevI = null;
    for (const k of ['price', 'pay']) {
      const d = ft - ev[k];
      if (d >= 0 && d < 0.35) { x = 1 - d / 0.35; prevI = Math.round(ev[k] * 30) - 1; }
    }
    img2.style.opacity = String(ease.inOutSine(x));
    if (prevI != null) {
      const s2 = F.dir + String(prevI).padStart(5, '0') + '.jpg';
      if (s2 !== lastSrc2) { img2.src = s2; lastSrc2 = s2; waits.push(img2.decode().catch(() => {})); }
    }
    // The readout: speed before the payment; the true time since the payment after it.
    const ro = $('s4readout');
    appear(ro, prog(t, 1.1, 0.8, ease.outQuint), { dy: 16 });
    const paid = fr.kind === 'paying' || fr.kind === 'progress' || fr.kind === 'claim' || fr.kind === 'done';
    ro.querySelector('.lbl').textContent = paid ? 'Real time since payment' : fr.spd > 1 ? 'Sped up' : 'Real speed';
    $('s4clk').textContent = paid ? fmtClock(fr.e) : `${fr.spd}×`;
    $('s4clk').style.display = paid || fr.spd > 1 ? '' : 'none';
    const spdTxt = fr.spd > 1 ? `${Math.round(fr.spd)}× speed` : 'real speed';
    $('s4spd').textContent = spdTxt;
    $('s4spd').style.display = paid ? '' : 'none';
    // Callouts with leaders to the page.
    const bx = F.boxes[fr.b] || {};
    CALLOUTS.forEach((c, i) => {
      const el = $(`s4c${i}`);
      const g = leaders[i];
      const p = prog(ft, c.a, 0.7, ease.spring) * (1 - prog(ft, c.b, 0.4, ease.inCubic));
      const raw = c.tgt(bx);
      if (!raw || p <= 0.001) { el.style.opacity = '0'; g.style.opacity = '0'; return; }
      const [tx, ty] = toScreen(cam, raw);
      if (tx < V.x + 8 || tx > V.x + V.w - 8 || ty < V.y + 8 || ty > V.y + V.h - 8) { el.style.opacity = '0'; g.style.opacity = '0'; return; }
      const w = el.offsetWidth, h = el.offsetHeight;
      let px = c.align === 'right' ? tx + c.dx - w : tx + c.dx;
      let py = ty + c.dy - h / 2;
      px = clamp(px, 96, 1824 - w);
      py = clamp(py, 150, 1000 - h);
      el.style.left = px + 'px';
      el.style.top = py + 'px';
      T(el, { y: (1 - p) * 18, s: lerp(0.92, 1, p), o: p });
      // Leader from the pill's nearest edge to the target.
      const ax = c.align === 'right' ? px + w - 30 : px + 30;
      const ay = c.dy < 0 ? py + h : py;
      const path = g.querySelector('path');
      const midY = ay + (ty - ay) * 0.55;
      path.setAttribute('d', `M${ax},${ay} C${ax},${midY} ${tx},${midY} ${tx},${ty}`);
      const len = Math.hypot(tx - ax, ty - ay) * 1.3 + 1;
      path.style.strokeDasharray = `${len}`;
      path.style.strokeDashoffset = `${len * (1 - prog(ft, c.a + 0.1, 0.6, ease.outCubic))}`;
      for (const cc of g.querySelectorAll('circle')) { cc.setAttribute('cx', tx); cc.setAttribute('cy', ty); }
      g.style.opacity = String(p);
    });
    const rp = prog(ft, ev.done + 1.6, 0.8, ease.spring);
    const re = $('s4real');
    re.style.transform = `translate(-50%, ${(1 - rp) * 20}px) scale(${lerp(0.94, 1, rp)})`;
    re.style.opacity = String(rp);
    return Promise.all(waits);
  };
  window.footageReady = Promise.resolve();
})();
