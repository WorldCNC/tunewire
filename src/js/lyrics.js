'use strict';

// ===========================================================================
// Lyrics panel (LRCLIB), time-synced when available
// ===========================================================================

const lyrics = { key: null, lines: null, plain: null, active: -1, loading: false, userScrolled: 0 };

function parseLrc(text) {
  const out = [];
  for (const raw of text.split('\n')) {
    const stamps = [...raw.matchAll(/\[(\d+):(\d+(?:\.\d+)?)\]/g)];
    if (!stamps.length) continue;
    const words = raw.replace(/\[[^\]]*\]/g, '').trim();
    for (const m of stamps) out.push({ t: Number(m[1]) * 60 + Number(m[2]), text: words });
  }
  return out.sort((a, b) => a.t - b.t);
}

function showPanel(name) {
  state.panel = name;
  for (const b of $$('.panel-tabs button')) b.classList.toggle('active', b.dataset.panel === name);
  $('#queue-list').hidden = name !== 'queue';
  $('#queue-actions').hidden = name !== 'queue';
  $('#lyrics-panel').hidden = name !== 'lyrics';
  $('#lyrics-actions').hidden = name !== 'lyrics';
  $('#np-panel').hidden = name !== 'now';
  $('#friends-panel').hidden = name !== 'friends';
  $('#together-panel').hidden = name !== 'together';
  if (name === 'friends') renderFriends();
  if (name === 'together') renderTogether();
  if (name === 'lyrics') loadLyrics();
  if (name === 'now') renderNowPlaying(true);
  syncPanelButtons();
  try { localStorage.setItem('tw-panel', name); } catch { /* optional */ }
}
$('.panel-tabs').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-panel]');
  if (b) showPanel(b.dataset.panel);
});

async function loadLyrics(force = false) {
  const p = state.playback;
  const track = p?.current?.track;
  const box = $('#lyrics-panel');
  if (box.hidden && !force && !fullLyrics.open) return;
  if (!track) {
    lyrics.key = null;
    box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Play a song to see its lyrics.')));
    return;
  }
  if (lyrics.key === track.key && !force) return;
  lyrics.key = track.key;
  lyrics.lines = null;
  lyrics.plain = null;
  lyrics.active = -1;
  box.replaceChildren(h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Looking up lyrics…'));
  const album = p.version?.album || track.versions.find((v) => v.album)?.album || null;
  let res;
  try {
    res = await call('get_lyrics', { artist: track.artist, title: track.title, album, duration: p.duration || track.duration || null });
  } catch (e) {
    if (lyrics.key !== track.key) return;
    box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Couldn’t load lyrics.'), h('p', { class: 'small' }, errMsg(e))));
    return;
  }
  if (lyrics.key !== track.key) return;
  if (!res) {
    box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, state.settings?.lyrics === false ? 'Lyrics are turned off in Settings.' : 'No lyrics found for this song.')));
    renderFullLyrics();
    return;
  }
  if (res.instrumental && !res.synced && !res.plain) {
    box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Instrumental')));
    renderFullLyrics();
    return;
  }
  const credit = h('div', { class: 'lyrics-credit' }, `Lyrics from ${res.source}`);
  if (res.synced) {
    lyrics.lines = parseLrc(res.synced);
    const list = h('div', { class: 'lyrics-lines synced' }, ...lyrics.lines.map((l, i) =>
      h('button', { class: 'lyric', dataset: { i }, onclick: () => seekTo(l.t) }, l.text || '♪')));
    box.replaceChildren(list, credit);
    syncLyrics(true);
    renderFullLyrics();
  } else {
    lyrics.plain = res.plain;
    box.replaceChildren(h('div', { class: 'lyrics-lines plain' }, ...res.plain.split('\n').map((l) => h('p', {}, l || ' '))), credit);
    renderFullLyrics();
  }
}

/** Highlight the current line and keep it in view. */
function syncLyrics(force = false) {
  syncFullLyrics(force);
  if (!lyrics.lines || $('#lyrics-panel').hidden) return;
  const pos = (state.playback?.position || 0) + 0.25;
  let i = -1;
  for (let k = 0; k < lyrics.lines.length && lyrics.lines[k].t <= pos; k++) i = k;
  if (i === lyrics.active && !force) return;
  lyrics.active = i;
  const els = $$('#lyrics-panel .lyric');
  els.forEach((el, k) => {
    el.classList.toggle('current', k === i);
    el.classList.toggle('past', k < i);
  });
  // Don't fight the user if they just scrolled the panel.
  if (i >= 0 && Date.now() - lyrics.userScrolled > 4000) {
    els[i].scrollIntoView({ block: 'center', behavior: force ? 'auto' : 'smooth' });
  }
}
$('#lyrics-panel').addEventListener('wheel', () => { lyrics.userScrolled = Date.now(); }, { passive: true });
