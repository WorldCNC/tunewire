'use strict';

// Wrapped: your year in music as a story (loaded on first use).

async function openWrapped(year) {
  let s;
  try { s = await call('listening_stats', { range: 'year', year }); } catch (e) { toast(errMsg(e), 'error'); return; }
  if (!s.plays) { toast(`No listening recorded for ${year} yet`, 'warn'); return; }
  const soFar = year === new Date().getFullYear() && new Date().getMonth() < 11;
  const topA = s.top_artists[0];
  const topT = s.top_tracks[0];
  const p = personality(s);
  const palette = ['#3b2a63', '#7a2f4f', '#1f4d5c', '#6b4a1a', '#24543a', '#4a2a2a', '#2a3b6b', '#5c1f4d'];
  const peakMonth = s.series.reduce((a, b) => (b.minutes > a.minutes ? b : a), s.series[0] || { label: '', minutes: 0 });

  const big = (txt, cls = '') => h('div', { class: `w-big ${cls}` }, txt);
  const slides = [
    { bg: palette[0], el: h('div', { class: 'w-slide w-intro' }, h('div', { class: 'w-kicker' }, 'Tunewire Wrapped'), big(String(year), 'huge'), h('p', {}, soFar ? 'Your year in music, so far.' : 'Your year in music.'), h('p', { class: 'w-hint' }, 'Tap to continue')) },
    { bg: palette[1], el: h('div', { class: 'w-slide' }, h('p', {}, 'This year you listened for'), big(fmtMinutes(s.minutes), 'count'), h('p', {}, 'minutes'),
      h('p', { class: 'w-sub' }, `That's ${(s.minutes / 1440).toFixed(1)} days of music, across ${s.days_active} days you pressed play.${peakMonth.minutes ? ` ${peakMonth.label} was your biggest month.` : ''}`)) },
    topA ? { bg: palette[2], img: topA.thumbnail, el: h('div', { class: 'w-slide' }, h('p', {}, 'Your top artist'),
      topA.thumbnail ? h('img', { class: 'w-photo round', 'data-px': 500, src: topA.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : null,
      big(topA.name), h('p', { class: 'w-sub' }, `${topA.plays} plays · ${fmtMinutes(topA.minutes)} minutes together`)) } : null,
    s.top_artists.length > 1 ? { bg: palette[3], el: h('div', { class: 'w-slide w-list' }, h('p', {}, 'Your top artists'),
      h('ol', {}, ...s.top_artists.slice(0, 5).map((a) => h('li', {}, a.thumbnail ? h('img', { src: a.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : h('span', { class: 'w-noimg' }), h('b', {}, a.name), h('em', {}, `${fmtMinutes(a.minutes)} min`))))) } : null,
    topT ? { bg: palette[4], img: topT.track.thumbnail, el: h('div', { class: 'w-slide' }, h('p', {}, 'Your song of the year'),
      topT.track.thumbnail ? h('img', { class: 'w-photo', 'data-px': 500, src: topT.track.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : null,
      big(topT.track.title), h('p', {}, topT.track.artist), h('p', { class: 'w-sub' }, `You played it ${topT.plays} times.`),
      h('button', { class: 'btn primary w-play', html: `${icon('play')}Play it`, onclick: (e) => { e.stopPropagation(); playTrack(topT.track); } })) } : null,
    s.top_tracks.length > 1 ? { bg: palette[5], el: h('div', { class: 'w-slide w-list' }, h('p', {}, 'Your top songs'),
      h('ol', {}, ...s.top_tracks.slice(0, 5).map((t) => h('li', {}, t.track.thumbnail ? h('img', { src: t.track.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : h('span', { class: 'w-noimg' }), h('div', {}, h('b', {}, t.track.title), h('span', {}, t.track.artist)), h('em', {}, `${t.plays}×`))))) } : null,
    { bg: palette[6], el: h('div', { class: 'w-slide' }, h('p', {}, 'Your listening personality'), big(p.name), h('p', { class: 'w-sub' }, p.text),
      h('div', { class: 'w-facts' },
        h('div', {}, h('b', {}, s.artists.toLocaleString()), h('span', {}, 'artists')),
        h('div', {}, h('b', {}, s.new_artists.toLocaleString()), h('span', {}, 'new to you')),
        h('div', {}, h('b', {}, `${s.longest_streak}`), h('span', {}, 'day streak')))) },
    { bg: palette[7], share: true, el: null },
  ].filter(Boolean);

  // The last slide: a summary card you can save as an image.
  const card = h('div', { class: 'w-card' },
    h('div', { class: 'w-card-head' }, h('span', {}, `My ${year}${soFar ? ' so far' : ''}`), h('b', {}, 'Tunewire')),
    topA?.thumbnail ? h('img', { class: 'w-card-img', src: topA.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : null,
    h('div', { class: 'w-card-cols' },
      h('div', {}, h('span', {}, 'Top artists'), h('ol', {}, ...s.top_artists.slice(0, 5).map((a) => h('li', {}, a.name)))),
      h('div', {}, h('span', {}, 'Top songs'), h('ol', {}, ...s.top_tracks.slice(0, 5).map((t) => h('li', {}, t.track.title))))),
    h('div', { class: 'w-card-cols' },
      h('div', {}, h('span', {}, 'Minutes listened'), h('b', {}, fmtMinutes(s.minutes))),
      h('div', {}, h('span', {}, 'Personality'), h('b', {}, p.name))));
  slides.at(-1).el = h('div', { class: 'w-slide w-share' }, card,
    h('div', { class: 'actions justify-center' },
      h('button', { class: 'btn primary', html: `${icon('download')}Save image`, onclick: (e) => { e.stopPropagation(); saveWrappedImage(s, year, p, soFar); } }),
      cloudOn() ? h('button', { class: 'btn', html: `${icon('friends')}Compare with friends`, onclick: (e) => { e.stopPropagation(); compareWrapped(s, year); } }) : null,
      h('button', { class: 'btn', html: `${icon('save')}Save top songs as playlist`, onclick: async (e) => {
        e.stopPropagation();
        try { await call('create_playlist', { name: `My top songs ${year}`, tracks: s.top_tracks.slice(0, 50).map((t) => t.track), sourceUrl: null }); toast(`Saved “My top songs ${year}”`, 'ok'); } catch (err) { toast(errMsg(err), 'error'); }
      } })));

  const bars = h('div', { class: 'w-bars' }, ...slides.map(() => h('div', {}, h('i'))));
  const stage = h('div', { class: 'w-stage' });
  const closeBtn = h('button', { class: 'w-close', title: 'Close', html: icon('x') });
  const overlay = h('div', { class: 'wrapped', role: 'dialog', 'aria-label': `${year} Wrapped` }, h('div', { class: 'w-bg' }), bars, closeBtn, stage);
  document.body.append(overlay);

  let i = -1, timer = null, start = 0;
  const DUR = 7000;
  const go = (n) => {
    if (n < 0) n = 0;
    if (n >= slides.length) return close();
    i = n;
    const sl = slides[i];
    overlay.style.setProperty('--w-bg', sl.bg);
    $('.w-bg', overlay).style.backgroundImage = sl.img ? `url("${sl.img}")` : '';
    stage.replaceChildren(sl.el);
    sl.el.classList.remove('in'); void sl.el.offsetWidth; sl.el.classList.add('in');
    const count = $('.count', sl.el);
    if (count) countUp(count, Math.round(s.minutes));
    $$('.w-bars > div', overlay).forEach((b, k) => b.className = k < i ? 'done' : k === i ? 'now' : '');
    clearTimeout(timer);
    start = performance.now();
    if (!sl.share) timer = setTimeout(() => go(i + 1), DUR);
    $('.w-bars > div.now i', overlay)?.style.setProperty('--dur', sl.share ? '0s' : `${DUR}ms`);
  };
  const close = () => {
    clearTimeout(timer);
    document.removeEventListener('keydown', onKey, true);
    overlay.classList.add('out');
    setTimeout(() => overlay.remove(), 250);
  };
  const onKey = (e) => {
    if (e.key === 'Escape') { e.stopPropagation(); close(); }
    if (e.key === 'ArrowRight' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); go(i + 1); }
    if (e.key === 'ArrowLeft') { e.preventDefault(); e.stopPropagation(); go(i - 1); }
  };
  document.addEventListener('keydown', onKey, true);
  closeBtn.addEventListener('click', close);
  stage.addEventListener('click', (e) => {
    if (e.target.closest('button')) return;
    const r = stage.getBoundingClientRect();
    go(e.clientX < r.left + r.width * 0.3 ? i - 1 : i + 1);
  });
  void start;
  go(0);
}

function countUp(el, target) {
  const t0 = performance.now();
  const step = (t) => {
    const k = Math.min(1, (t - t0) / 1400);
    el.textContent = Math.round(target * (1 - Math.pow(1 - k, 3))).toLocaleString();
    if (k < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

/** Draw the share card on a canvas (text only, so remote covers can't taint it). */
async function saveWrappedImage(s, year, p, soFar) {
  const W = 1080, H = 1350;
  const c = document.createElement('canvas');
  c.width = W; c.height = H;
  const g = c.getContext('2d');
  const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#1db954';
  const grad = g.createLinearGradient(0, 0, W, H);
  grad.addColorStop(0, '#1a1426'); grad.addColorStop(0.55, '#2a1838'); grad.addColorStop(1, '#0e1a22');
  g.fillStyle = grad; g.fillRect(0, 0, W, H);
  g.fillStyle = accent; g.globalAlpha = 0.22;
  g.beginPath(); g.arc(W * 0.85, H * 0.12, 360, 0, Math.PI * 2); g.fill();
  g.globalAlpha = 1;
  const font = (w, px) => `${w} ${px}px system-ui, -apple-system, "Segoe UI", sans-serif`;
  const fit = (text, max, w, px) => { g.font = font(w, px); let t = text; while (g.measureText(t).width > max && t.length > 1) t = t.slice(0, -2); return t === text ? t : `${t.trimEnd()}…`; };
  g.fillStyle = '#fff';
  g.font = font(600, 34); g.fillText(`TUNEWIRE WRAPPED`, 80, 120);
  g.font = font(800, 150); g.fillText(String(year), 72, 270);
  if (soFar) { g.font = font(500, 36); g.globalAlpha = 0.7; g.fillText('so far', 80, 320); g.globalAlpha = 1; }
  const col = (x, title, items) => {
    g.font = font(600, 30); g.globalAlpha = 0.65; g.fillText(title, x, 430); g.globalAlpha = 1;
    items.slice(0, 5).forEach((t, k) => { g.font = font(700, 40); g.fillText(fit(`${k + 1}  ${t}`, 440, 700, 40), x, 500 + k * 66); });
  };
  col(80, 'Top artists', s.top_artists.map((a) => a.name));
  col(560, 'Top songs', s.top_tracks.map((t) => t.track.title));
  g.font = font(600, 30); g.globalAlpha = 0.65; g.fillText('Minutes listened', 80, 930); g.fillText('Personality', 560, 930); g.globalAlpha = 1;
  g.font = font(800, 80); g.fillText(fmtMinutes(s.minutes), 80, 1020);
  g.fillText(fit(p.name.replace(/^The /, ''), 440, 800, 80), 560, 1020);
  g.font = font(500, 32); g.globalAlpha = 0.7;
  g.fillText(`${s.artists} artists · ${s.tracks} songs · ${s.longest_streak}-day streak`, 80, 1120);
  g.globalAlpha = 1;
  g.fillStyle = accent; g.fillRect(80, 1210, 120, 10);
  try {
    const path = await call('save_share_image', { name: `Tunewire Wrapped ${year}`, data: c.toDataURL('image/png') });
    toast(`Saved to ${path}`, 'ok', 5000);
  } catch (e) { toast(errMsg(e), 'error'); }
}
