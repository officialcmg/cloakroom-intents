// The film: six scenes on one timeline. renderFrame(t) draws the film at time t (seconds).
// Every text a viewer reads is in film.html or in the COPY and footage data below; the
// source of each claim is listed in launch-facts.json.
(function () {
  const { clamp, lerp, ease, prog, T, appear, words, chars, revealWords } = E;
  const $ = (id) => document.getElementById(id);
  const ICON = {
    shield: 'M12 3l7 3v5c0 4.5-3 8.2-7 10-4-1.8-7-5.5-7-10V6l7-3z',
    lockBody: 'M6 11h12v9H6z',
    shackle: 'M8.5 11V8a3.5 3.5 0 017 0v3',
    bridge: 'M2.5 16.5h19M5 16.5v-2.5a7 7 0 0114 0v2.5M9 16.5v-3.3M15 16.5v-3.3M12 16.5V12',
    swap: 'M5 8.5h13l-3.5-3.5M19 15.5H6l3.5 3.5',
    eye: 'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12zM12 15a3 3 0 100-6 3 3 0 000 6z',
    key: 'M15 9a3 3 0 11-6 0 3 3 0 016 0zM12 12v9M12 17h3M12 20h2',
    check: 'M5 12.5l4.5 4.5L19 7.5',
  };
  const svg = (d, size, sw = 1.8, extra = '') => `<svg class="ic" width="${size}" height="${size}" viewBox="0 0 24 24" stroke-width="${sw}" ${extra}><path d="${d}"/></svg>`;

  // Scene windows on the film timeline (seconds). Each scene overlaps the one before it
  // by its reveal.
  const SC = {
    s1: [0, 6.4],
    s2: [5.6, 14.4],
    s3: [13.6, 26.6],
    s4: [25.8, 52.6],
    s5: [51.8, 62.4],
    s6: [61.6, 70.0],
  };
  const DURATION = 70.0;

  // ---------- brand ----------
  for (const id of ['s1mark', 's6mark']) $(id).innerHTML = BRAND.MARK_SVG.replace(/bm/g, id + 'x'); // unique ids: a mask in a hidden copy would blank the other
  for (const id of ['s1word', 's6word']) $(id).innerHTML = `<span class="wp" style="display:inline-block">${BRAND.word1}</span><span class="wp serif" style="display:inline-block;font-size:1.12em;letter-spacing:0;margin-left:0.22em">${BRAND.word2}</span>`;
  $('s6url').innerHTML = `${BRAND.url} ${svg('M5 12h14M13 6l6 6-6 6', 40, 2.2)}`;

  // ---------- scene 1: cold open ----------
  const s1a = chars($('s1a'));
  const s1wordChars = [...$('s1word').children];
  function scene1(t) {
    T($('s1glow'), { x: Math.sin(t * 0.35) * 60 + t * 18, y: Math.cos(t * 0.3) * 30, o: 0.55 + 0.45 * prog(t, 0, 2.5, ease.inOutSine) });
    T($('s1arc'), { y: (1 - prog(t, 0, 3.5, ease.outCubic)) * 200 - t * 6, o: prog(t, 0.2, 2.5) });
    // "Shielded ZEC" tracks in.
    const out = prog(t, 3.8, 0.7, ease.inCubic);
    $('s1a').style.letterSpacing = `${lerp(0.06, -0.045, prog(t, 0.2, 2.2, ease.outQuart))}em`;
    s1a.forEach((c, i) => {
      const p = prog(t, 0.25 + i * 0.045, 1.0, ease.outExpo);
      T(c, { y: (1 - p) * 36, o: p, blur: (1 - p) * 14 });
    });
    const line = prog(t, 1.45, 1.1, ease.inOutCubic);
    T($('s1line'), { sx: line, sy: 1, o: line > 0 ? 1 : 0, origin: '0 50%' });
    $('s1dot').style.left = `${line * 100}%`;
    $('s1dot').style.opacity = String(clamp(line * 6) * (1 - prog(t, 2.55, 0.35)));
    const b = prog(t, 2.3, 1.1, ease.outExpo);
    T($('s1b'), { x: (1 - b) * -26, o: b, blur: (1 - b) * 18 });
    T($('s1row'), { y: -out * 60, o: 1 - out, blur: out * 12 });
    // The wordmark.
    const m = prog(t, 4.15, 1.1, ease.spring);
    T($('s1mark'), { s: lerp(0.55, 1, m), r: lerp(-14, 0, m), o: prog(t, 4.15, 0.4), blur: (1 - prog(t, 4.15, 0.5)) * 8 });
    s1wordChars.forEach((c, i) => {
      const p = prog(t, 4.35 + i * 0.3, 1.0, ease.outExpo);
      T(c, { x: (1 - p) * 40, o: p, blur: (1 - p) * 10 });
    });
    appear($('s1eye'), prog(t, 5.0, 0.9, ease.outQuint), { dy: 20 });
  }

  // ---------- scene 2: the problem ----------
  const s2h = words($('s2h'));
  const NODES = [
    { x: 300, label: 'Shielded ZEC', icon: 'shield', tone: 'ok' },
    { x: 760, label: 'Transparent address', icon: 'open', tone: 'warn' },
    { x: 1220, label: 'Bridge', icon: 'bridge', tone: 'muted' },
    { x: 1640, label: 'Swap', icon: 'swap', tone: 'muted' },
  ];
  const LINKS = ['unshield', 'bridge', 'swap'];
  const NY = 548; // tile centre
  {
    const d = $('s2diag');
    let html = '';
    NODES.forEach((n, i) => {
      const inner =
        n.icon === 'open'
          ? `<div class="abs" id="s2sh-l" style="left:28px;top:28px;width:104px;height:104px;clip-path:inset(0 50% 0 0);color:var(--warn)">${svg(ICON.shield, 104, 1.6)}</div>
             <div class="abs" id="s2sh-r" style="left:28px;top:28px;width:104px;height:104px;clip-path:inset(0 0 0 50%);color:var(--warn)">${svg(ICON.shield, 104, 1.6)}</div>
             <div class="abs" id="s2eyeic" style="left:52px;top:56px;color:var(--warn)">${svg(ICON.eye, 56, 1.9)}</div>`
          : n.icon === 'shield'
            ? `<div style="color:var(--green-600)">${svg(ICON.shield, 104, 1.6, 'fill="rgb(23 115 90 / 10%)"')}</div>`
            : `<div style="color:var(--ink-2)">${svg(ICON[n.icon], 96, 1.6)}</div>`;
      html += `<div class="abs" id="s2n${i}" style="left:${n.x - 80}px;top:${NY - 80}px;width:160px">
          <div class="card tile" style="position:relative;width:160px;height:160px;border-radius:40px">${inner}</div>
          <p class="display" style="margin:28px -120px 0;text-align:center;font-size:44px;letter-spacing:-0.03em;line-height:1.05;color:${n.tone === 'warn' ? 'var(--warn)' : 'var(--ink)'}">${n.label}</p>
        </div>`;
    });
    LINKS.forEach((l, i) => {
      const a = NODES[i].x + 104, b = NODES[i + 1].x - 104;
      html += `<div class="abs" id="s2l${i}" style="left:${a}px;top:${NY - 1}px;width:${b - a}px;height:3px;background-image:linear-gradient(90deg, ${i === 0 ? 'var(--warn)' : 'var(--ink-2)'} 0 60%, transparent 60% 100%);background-size:14px 3px;opacity:.55"></div>
        <div class="abs mono" id="s2t${i}" style="left:${a}px;width:${b - a}px;top:${NY - 54}px;text-align:center;font-size:24px;color:${i === 0 ? 'var(--warn)' : 'var(--ink-2)'}">${l}</div>`;
    });
    html += `<div class="abs mono" id="s2tag" style="left:${NODES[1].x - 250}px;width:500px;top:${NY - 150}px;text-align:center"><span style="display:inline-block;white-space:nowrap;padding:10px 18px;border-radius:999px;background:#fbf0da;color:var(--warn);font-size:22px">Leaves the shielded pool</span></div>`;
    d.innerHTML = html;
  }
  function scene2(t) {
    // Reveal: a light panel rises over the cold open.
    const r = prog(t, 0, 0.95, ease.inOutQuint);
    $('s2').style.clipPath = r < 1 ? `inset(${(1 - r) * 100}% 0 0 0 round ${lerp(80, 0, r)}px ${lerp(80, 0, r)}px 0 0)` : 'none';
    T($('s2arc'), { y: (1 - r) * 240 - t * 8 });
    appear($('s2eye'), prog(t, 0.55, 0.8, ease.outQuint), { dy: 18 });
    revealWords(s2h, t, 0.65, { stagger: 0.07, d: 1.0 });
    NODES.forEach((n, i) => {
      const at = [1.6, 2.6, 3.7, 4.6][i];
      const p = prog(t, at, 0.9, ease.spring);
      T($(`s2n${i}`), { y: (1 - p) * 40, s: lerp(0.86, 1, p), o: prog(t, at, 0.35), blur: (1 - prog(t, at, 0.5)) * 8 });
    });
    LINKS.forEach((_, i) => {
      const at = [2.05, 3.2, 4.15][i];
      const p = prog(t, at, 0.6, ease.inOutCubic);
      $(`s2l${i}`).style.clipPath = `inset(0 ${(1 - p) * 100}% 0 0)`;
      appear($(`s2t${i}`), prog(t, at + 0.2, 0.6), { dy: 10, blur: 4 });
    });
    // The shield opens at the transparent address.
    const o = prog(t, 2.95, 0.9, ease.spring);
    T($('s2sh-l'), { x: -o * 12, r: -o * 16, origin: '50% 95%' });
    T($('s2sh-r'), { x: o * 12, r: o * 16, origin: '50% 95%' });
    T($('s2eyeic'), { s: lerp(0.4, 1, o), o: prog(t, 3.1, 0.4) });
    appear($('s2tag'), prog(t, 3.3, 0.7, ease.outQuint), { dy: 14 });
    appear($('s2p'), prog(t, 5.2, 1.0, ease.outQuint), { dy: 24 });
    // Parallax drift.
    T($('s2diag'), { x: -t * 3 });
  }

  // ---------- scene 3: the idea ----------
  const s3h = words($('s3h'));
  const BEATS = [
    'The ZEC goes to a shielded address. Its key is split in two halves.',
    'An escrow on NEAR releases the USDC to whoever reveals the right half.',
    'The USDC lands in NEAR Intents under a key made in your browser. One signature sends it on.',
  ];
  $('s3beats').innerHTML = BEATS.map((b, i) => `<li id="s3b${i}" style="display:flex;gap:28px;margin:0 0 28px">
      <span class="mono" style="font-size:24px;color:var(--mint-300);padding-top:14px">0${i + 1}</span>
      <span class="body" style="color:#fff;line-height:1.18">${b}</span></li>`).join('');
  const CX = 1340; // centre of the drawing
  const KEY_SVG = `<svg width="400" height="130" viewBox="0 0 400 130" fill="none" stroke="#9fe3c6" stroke-width="14" stroke-linecap="round"><circle cx="78" cy="65" r="44"/><circle cx="78" cy="65" r="10" stroke-width="8"/><path d="M122 65H362M304 65v32M344 65v22"/></svg>`;
  const CHAINS = window.CHAIN_NAMES;
  {
    const a = $('s3art');
    a.innerHTML = `
      <div class="abs" id="s3shield" style="left:${CX - 120}px;top:150px;width:240px;height:240px;color:var(--mint-300)">
        ${svg(ICON.shield, 240, 1.1, 'fill="rgb(159 227 198 / 8%)"')}
        <div class="coin zec" id="s3zec" style="left:120px;top:118px">ZEC</div>
      </div>
      <div class="abs" id="s3shieldlbl" style="left:${CX - 300}px;width:600px;top:404px;text-align:center"><span class="chip">Shielded address</span></div>
      <div class="abs" id="s3keyL" style="left:${CX - 200}px;top:500px;width:400px;height:130px;clip-path:polygon(0 0,222px 0,200px 33px,222px 65px,200px 98px,222px 130px,0 130px)">${KEY_SVG}</div>
      <div class="abs" id="s3keyR" style="left:${CX - 200}px;top:500px;width:400px;height:130px;clip-path:polygon(222px 0,400px 0,400px 130px,222px 130px,200px 98px,222px 65px,200px 33px)">${KEY_SVG}</div>
      <div class="abs" id="s3lblL" style="left:${CX - 420}px;width:400px;top:668px;text-align:center"><span class="chip" style="font-size:32px;padding:14px 26px">Your browser</span></div>
      <div class="abs" id="s3lblR" style="left:${CX + 20}px;width:400px;top:668px;text-align:center"><span class="chip" style="font-size:32px;padding:14px 26px">The maker</span></div>
      <div class="gcard" id="s3escrow" style="left:${CX - 340}px;top:780px;width:680px;height:168px;display:flex;align-items:center;gap:28px;padding:0 36px">
        <div style="position:relative;width:76px;height:76px;color:#fff;flex:none">
          <svg class="ic" width="76" height="76" viewBox="0 0 24 24" stroke-width="1.7"><path d="${ICON.lockBody}"/></svg>
          <svg class="ic abs" id="s3shackle" style="left:0;top:0" width="76" height="76" viewBox="0 0 24 24" stroke-width="1.7"><path d="${ICON.shackle}"/></svg>
        </div>
        <div style="flex:1;min-width:0;padding-right:110px"><div class="display" style="font-size:44px;letter-spacing:-0.03em">Escrow on NEAR</div>
          <div class="mono" id="s3escrowNote" style="white-space:nowrap;font-size:22px;color:var(--mint-300);margin-top:10px">The maker locks USDC first</div></div>
      </div>
      <div class="coin usdc" id="s3usdc" style="left:${CX + 282}px;top:864px">USDC</div>
      <div class="gcard" id="s3intents" style="left:${CX - 370}px;top:190px;width:740px;height:200px;padding:36px 40px">
        <div class="display" style="font-size:52px;letter-spacing:-0.035em">NEAR Intents</div>
        <div style="display:flex;gap:12px;margin-top:22px"><span class="chip" style="font-size:24px">${svg(ICON.key, 26, 2)}&nbsp; Key made in your browser</span><span class="chip" style="font-size:24px">Gas from the maker</span></div>
      </div>
      <div class="abs" id="s3chips" style="left:0;top:0"></div>
      <div class="abs display" id="s3count" style="left:${CX - 400}px;width:800px;top:930px;text-align:center;font-size:44px;letter-spacing:-0.03em;color:#fff"><span id="s3countN">0</span> chains in the swap form</div>`;
    const box = $('s3chips');
    box.innerHTML = CHAINS.map((c, i) => `<span class="chip abs" id="s3c${i}" style="font-size:26px;padding:12px 20px">${c}</span>`).join('');
  }
  let chipPos = null;
  function layoutChips() {
    // Rows centred under the NEAR Intents card.
    const W = 940, gap = 12, rowH = 52 + 14;
    const rows = [[]];
    let w = 0;
    CHAINS.forEach((_, i) => {
      const cw = $(`s3c${i}`).offsetWidth;
      if (w + cw > W && rows[rows.length - 1].length) { rows.push([]); w = 0; }
      rows[rows.length - 1].push([i, cw]);
      w += cw + gap;
    });
    chipPos = [];
    rows.forEach((row, r) => {
      const tw = row.reduce((a, [, cw]) => a + cw, 0) + gap * (row.length - 1);
      let x = CX - tw / 2;
      for (const [i, cw] of row) {
        chipPos[i] = { x, y: 448 + r * rowH, w: cw };
        x += cw + gap;
      }
    });
  }
  const qb = (a, c, b, p) => (1 - p) * (1 - p) * a + 2 * (1 - p) * p * c + p * p * b;
  function scene3(t) {
    const r = prog(t, 0, 1.0, ease.inOutQuint);
    $('s3').style.clipPath = r < 1 ? `circle(${r * 2300}px at 760px ${NY}px)` : 'none';
    T($('s3glow'), { x: Math.sin(t * 0.3) * 50, y: t * 4 });
    appear($('s3eye'), prog(t, 0.7, 0.8, ease.outQuint), { dy: 18 });
    revealWords(s3h, t, 0.8, { stagger: 0.07, d: 1.0 });
    // Beats: active one bright.
    const beatAt = [1.4, 5.2, 8.6];
    BEATS.forEach((_, i) => {
      const p = prog(t, beatAt[i], 0.8, ease.outQuint);
      const next = i < 2 ? prog(t, beatAt[i + 1], 0.6) : 0;
      T($(`s3b${i}`), { y: (1 - p) * 24, o: p * lerp(1, 0.38, next), blur: (1 - p) * 6 });
    });
    // Beat 1: the shielded address and the split key.
    const b1 = prog(t, 1.5, 1.0, ease.spring);
    const fade3 = prog(t, 8.6, 0.8, ease.inOutCubic); // beat 3 clears the drawing
    T($('s3shield'), { s: lerp(0.8, 1, b1) * lerp(1, 0.85, fade3), y: -fade3 * 40, o: prog(t, 1.5, 0.4) * (1 - fade3), blur: (1 - prog(t, 1.5, 0.6)) * 10 + fade3 * 10 });
    T($('s3zec'), { s: lerp(0.4, 1, prog(t, 1.8, 0.9, ease.spring)), o: prog(t, 1.8, 0.3) });
    appear($('s3shieldlbl'), prog(t, 2.0, 0.8, ease.outQuint) * (1 - fade3), { dy: 14 });
    const k = prog(t, 2.3, 0.8, ease.spring);
    const split = prog(t, 3.1, 1.0, ease.spring);
    // The left half (the browser's) later flies into the escrow's lock.
    const fly = prog(t, 5.9, 1.0, ease.inOutCubic);
    const lx = -split * 220, ly = 0;
    const lockX = CX - 340 + 36 + 38 - CX, lockY = 780 + 84 - 565; // lock centre relative to the key centre
    T($('s3keyL'), {
      x: lerp(lx, lockX + 60, fly), y: lerp(ly, lockY, fly) - Math.sin(fly * Math.PI) * 120,
      s: k * lerp(1, 0.32, fly), r: -split * 8 + fly * -20, o: prog(t, 2.3, 0.3) * (1 - prog(t, 6.75, 0.25)), blur: (1 - prog(t, 2.3, 0.5)) * 8,
    });
    T($('s3keyR'), { x: split * 220, s: k * lerp(1, 0.9, fade3), r: split * 8, o: prog(t, 2.3, 0.3) * (1 - fade3), blur: (1 - prog(t, 2.3, 0.5)) * 8 + fade3 * 8 });
    appear($('s3lblL'), prog(t, 3.5, 0.8, ease.outQuint) * (1 - prog(t, 5.6, 0.4)), { dy: 14 });
    appear($('s3lblR'), prog(t, 3.65, 0.8, ease.outQuint) * (1 - fade3), { dy: 14 });
    // Beat 2: the escrow, the reveal, the release.
    const e = prog(t, 5.0, 0.9, ease.spring);
    T($('s3escrow'), { y: (1 - e) * 60 + fade3 * 40, s: lerp(0.92, 1, e), o: prog(t, 5.0, 0.4) * (1 - fade3), blur: (1 - prog(t, 5.0, 0.6)) * 10 + fade3 * 10 });
    const open = prog(t, 6.8, 0.6, ease.spring);
    T($('s3shackle'), { y: -open * 9, r: open * 0, origin: '30% 50%' });
    if (t > 6.9) $('s3escrowNote').textContent = 'Your half revealed: released';
    else $('s3escrowNote').textContent = 'The maker locks USDC first';
    // The USDC coin: sits in the escrow, leaves when it opens, lands in NEAR Intents.
    const u0 = prog(t, 5.3, 0.8, ease.spring);
    const rel = prog(t, 7.2, 0.9, ease.inOutCubic);
    const land = prog(t, 9.3, 1.1, ease.inOutCubic);
    const ux = lerp(lerp(0, 140, rel), -46, land);
    const uy = lerp(-rel * 150, 290 - 864 - 0, land);
    const arcY = -Math.sin(land * Math.PI) * 60;
    T($('s3usdc'), { x: ux, y: uy + arcY, s: lerp(0.4, 1, u0) * lerp(1, 0.8, land), o: prog(t, 5.3, 0.3) * (1 - prog(t, 10.3, 0.3)) });
    // Beat 3: NEAR Intents and the chains.
    const ni = prog(t, 9.0, 1.0, ease.spring);
    T($('s3intents'), { y: (1 - ni) * 50, s: lerp(0.9, 1, ni), o: prog(t, 9.0, 0.4), blur: (1 - prog(t, 9.0, 0.6)) * 10 });
    if (!chipPos) layoutChips();
    CHAINS.forEach((_, i) => {
      const pos = chipPos[i];
      const dist = Math.hypot(pos.x + pos.w / 2 - CX, pos.y - 400) / 600;
      const a = 10.2 + dist * 0.55 + (i % 5) * 0.025;
      const p = prog(t, a, 0.9, ease.outQuint);
      T($(`s3c${i}`), { x: lerp(CX - pos.w / 2, pos.x, p), y: lerp(330, pos.y, p), s: lerp(0.5, 1, p), o: prog(t, a, 0.3), blur: (1 - p) * 8 });
    });
    const cnt = prog(t, 10.3, 1.4, ease.outCubic);
    $('s3countN').textContent = String(Math.round(cnt * CHAINS.length));
    appear($('s3count'), prog(t, 10.3, 0.7, ease.outQuint), { dy: 16 });
  }

  // ---------- scene 5: proof ----------
  const s5h = words($('s5h'));
  const STATS = [
    { n: 32, txt: 'real swaps completed on public test networks' },
    { n: 26, txt: 'of them in one overnight soak test' },
    { n: '6–9', txt: 'minutes for a typical swap, mostly waiting for Zcash blocks' },
    { n: 0, txt: 'bridges or custodians holding the ZEC' },
  ];
  $('s5stats').innerHTML = STATS.map((s, i) => `<div class="stat" id="s5s${i}" style="left:${144 + (i % 2) * 848}px;top:${336 + Math.floor(i / 2) * 264}px">
    <div class="num" id="s5n${i}">0</div><div class="txt">${s.txt}</div></div>`).join('');
  function scene5(t) {
    const r = prog(t, 0, 0.95, ease.inOutQuint);
    $('s5').style.clipPath = r < 1 ? `inset(0 0 ${(1 - r) * 100}% 0 round 0 0 ${lerp(80, 0, r)}px ${lerp(80, 0, r)}px)` : 'none';
    T($('s5glow'), { x: -t * 10, y: Math.sin(t * 0.4) * 30 });
    appear($('s5eye'), prog(t, 0.6, 0.8, ease.outQuint), { dy: 18 });
    revealWords(s5h, t, 0.7, { stagger: 0.07, d: 1.0 });
    STATS.forEach((s, i) => {
      const a = 1.3 + i * 0.18;
      const p = prog(t, a, 1.0, ease.spring);
      T($(`s5s${i}`), { y: (1 - p) * 70, r: (1 - p) * (i % 2 ? 3 : -3), o: prog(t, a, 0.35), blur: (1 - prog(t, a, 0.6)) * 10 });
      const c = prog(t, a + 0.2, 1.6, ease.outCubic);
      const el = $(`s5n${i}`);
      if (typeof s.n === 'number') el.textContent = String(Math.round(c * s.n));
      else el.textContent = c < 0.5 ? String(Math.round(c * 2 * 6)) : '6–' + Math.round(lerp(6, 9, (c - 0.5) * 2));
    });
    appear($('s5p'), prog(t, 3.4, 1.0, ease.outQuint), { dy: 22 });
  }

  // ---------- scene 6: end card ----------
  const s6h = words($('s6h'));
  const s6chars = [...$('s6word').children];
  function scene6(t) {
    const r = prog(t, 0, 1.0, ease.inOutQuint);
    $('s6').style.clipPath = r < 1 ? `circle(${r * 1300}px at 960px 540px)` : 'none';
    T($('s6glow'), { x: Math.sin(t * 0.4) * 60, y: -t * 6 });
    T($('s6arc'), { y: (1 - prog(t, 0, 2, ease.outCubic)) * 200 });
    const m = prog(t, 0.5, 1.0, ease.spring);
    T($('s6mark'), { s: lerp(0.6, 1, m), r: lerp(-12, 0, m), o: prog(t, 0.5, 0.35), blur: (1 - prog(t, 0.5, 0.5)) * 8 });
    s6chars.forEach((c, i) => {
      const p = prog(t, 0.65 + i * 0.3, 1.0, ease.outExpo);
      T(c, { x: (1 - p) * 34, o: p, blur: (1 - p) * 8 });
    });
    revealWords(s6h, t, 1.3, { stagger: 0.06, d: 1.0 });
    appear($('s6url'), prog(t, 2.2, 0.9, ease.spring), { dy: 24, s0: 0.94 });
    appear($('s6status'), prog(t, 2.6, 0.9, ease.outQuint), { dy: 18 });
    // A slow push-in for the hold.
    T($('s6').firstElementChild, {});
    const push = prog(t, 0, 8.4, ease.inOutSine);
    $('s6').querySelector('.abs[style*="flex-direction"]').style.transform = `scale(${lerp(0.985, 1.015, push)})`;
    // Fade to the brand green at the very end.
    $('s6').style.filter = 'none';
  }

  const SCENES = { s1: scene1, s2: scene2, s3: scene3, s4: (t) => window.scene4 && window.scene4(t), s5: scene5, s6: scene6 };

  window.FILM = { SC, DURATION, FPS: 30 };
  window.renderFrame = async function (t) {
    const waits = [];
    for (const [id, [a, b]] of Object.entries(SC)) {
      const el = $(id);
      const on = t >= a && t < b;
      el.style.visibility = on ? 'visible' : 'hidden';
      if (on) {
        const w = SCENES[id](t - a);
        if (w) waits.push(w);
      }
    }
    await Promise.all(waits);
  };
  window.filmReady = document.fonts.ready.then(() => window.footageReady || null);
})();
