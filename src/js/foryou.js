'use strict';

// ===========================================================================
// Home ("For you"): Daily Mixes, Discover Weekly, On Repeat, stats teaser.
// Your stats page and the yearly Wrapped story.
// ===========================================================================

const home = { data: null, refreshing: false, seq: 0 };

function greeting() {
  const hr = new Date().getHours();
  return hr < 5 ? 'Good night' : hr < 12 ? 'Good morning' : hr < 18 ? 'Good afternoon' : 'Good evening';
}

function fmtMinutes(m) {
  m = Math.round(m || 0);
  return m.toLocaleString();
}
function fmtHours(m) {
  const hrs = (m || 0) / 60;
  return hrs >= 10 ? `${Math.round(hrs).toLocaleString()} h` : `${hrs.toFixed(1)} h`;
}

/** 2×2 cover collage (or a single cover). */
function mixArt(covers, cls = 'mix-art') {
  const list = (covers || []).filter(Boolean);
  const box = h('div', { class: `${cls}${list.length < 4 ? ' single' : ' grid4'}` });
  if (!list.length) { box.innerHTML = icon('music'); return box; }
  for (const url of list.length < 4 ? list.slice(0, 1) : list.slice(0, 4)) {
    box.append(h('img', { src: url, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => { e.target.style.visibility = 'hidden'; } }));
  }
  return box;
}

function mixTile(mix, i = 0) {
  const play = h('button', { class: 'mix-play', title: `Play ${mix.title}`, html: icon('play'), onclick: (e) => { e.stopPropagation(); playList(mix.tracks, 0); } });
  const tile = h('div', { class: `mix-tile kind-${mix.kind}`, style: `--i:${i}`, role: 'button', tabindex: '0', title: mix.title },
    h('div', { class: 'mix-cover' }, mixArt(mix.covers), h('span', { class: 'mix-label' }, mix.title), play),
    h('div', { class: 'mix-sub' }, mix.subtitle));
  tile.addEventListener('click', () => openMix(mix));
  tile.addEventListener('keydown', (e) => { if (e.key === 'Enter') openMix(mix); });
  tile.addEventListener('mouseenter', () => mix.tracks[0] && prefetchTrack(mix.tracks[0]), { once: true });
  return tile;
}

function skeletonTiles(n) {
  return Array.from({ length: n }, (_, i) => h('div', { class: 'mix-tile skeleton', style: `--i:${i}` }, h('div', { class: 'mix-cover' }), h('div', { class: 'mix-sub' }, ' ')));
}

/** Compact shortcut tiles at the top (Liked, playlists, mixes). */
function shortcutTiles(data) {
  const items = [];
  if (state.favorites?.length) items.push({ title: 'Liked songs', covers: state.favorites.slice(0, 4).map((t) => t.thumbnail), go: () => showView('favorites') });
  if (data?.discover) items.push({ title: 'Discover Weekly', covers: data.discover.covers, go: () => openMix(data.discover) });
  for (const m of data?.local || []) items.push({ title: m.title, covers: m.covers, go: () => openMix(m) });
  for (const p of (state.playlists || []).slice().sort((a, b) => b.updated - a.updated)) {
    items.push({ title: p.name, covers: p.tracks.slice(0, 4).map((t) => t.thumbnail), go: () => openPlaylist(p.id) });
  }
  if (data?.daily?.[0]) items.push({ title: data.daily[0].title, covers: data.daily[0].covers, go: () => openMix(data.daily[0]) });
  if (!items.length) return null;
  return h('div', { class: 'shortcut-grid' }, ...items.slice(0, 8).map((it, i) =>
    h('button', { class: 'shortcut', style: `--i:${i}`, onclick: it.go }, mixArt(it.covers, 'shortcut-art'), h('span', {}, it.title))));
}

async function renderHome() {
  const seq = ++home.seq;
  const body = $('#home-body');
  if (!home.data) {
    try { home.data = await call('for_you'); } catch (e) { home.data = { daily: [], local: [], discover: null, stale: false, empty: true }; }
  }
  if (seq !== home.seq) return;
  const d = home.data;

  const blocks = [h('div', { class: 'home-hello' }, h('h1', {}, greeting()))];
  const shortcuts = shortcutTiles(d);
  if (shortcuts) blocks.push(shortcuts);

  if (d.empty) {
    blocks.push(h('div', { class: 'empty home-empty' },
      h('span', { class: 'big', html: icon('sparkle') }),
      h('p', {}, 'Your mixes will show up here.'),
      h('p', { class: 'muted small' }, 'Play some songs, like a few, or import a playlist. Tunewire builds Daily Mixes and a weekly discovery playlist from what you listen to.'),
      h('div', { class: 'actions', style: 'justify-content:center' },
        h('button', { class: 'btn primary', onclick: () => showView('charts') }, 'Browse charts'),
        h('button', { class: 'btn', onclick: () => { showView('search'); $('#search-input').focus(); } }, 'Search'))));
    body.replaceChildren(...blocks);
    return;
  }

  // Made for you: Discover Weekly first, then the Daily Mixes.
  const madeRow = h('div', { class: 'mix-row' });
  const made = [d.discover, ...d.daily].filter(Boolean);
  if (made.length) madeRow.append(...made.map((m, i) => mixTile(m, i)));
  else if (d.stale || home.refreshing) madeRow.append(...skeletonTiles(5));
  const refreshBtn = h('button', { class: 'btn small', html: `${icon('refresh')}New mixes`, title: 'Build new mixes now', onclick: () => refreshHome(true, refreshBtn) });
  if (made.length || d.stale || home.refreshing) {
    blocks.push(h('div', { class: 'home-block' },
      h('div', { class: 'section-title' }, 'Made for you', h('span', { class: 'meta', style: 'margin-left:8px' }, 'Daily Mixes update every day, Discover Weekly every Monday'), h('span', { style: 'margin-left:auto' }), refreshBtn),
      madeRow));
  }

  if (d.local.length) {
    blocks.push(h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'From your listening'),
      h('div', { class: 'mix-row' }, ...d.local.map((m, i) => mixTile(m, i)))));
  }

  blocks.push(h('div', { class: 'home-block', id: 'home-stats' }));

  const played = (state.history || []).slice(0, 6);
  if (played.length) {
    blocks.push(h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Recently played',
      h('button', { class: 'btn small', style: 'margin-left:auto', onclick: () => showView('history') }, 'Show all')),
      h('div', { class: 'track-list' }, ...played.map((e, i) => trackRow(e.track, i, { onPlay: () => playTrack(e.track, e.platform) })))));
  }
  body.replaceChildren(...blocks);
  markPlaying();
  renderHomeStats();
  if (d.stale && !home.refreshing) refreshHome(false, refreshBtn);
}

async function refreshHome(force, btn) {
  if (home.refreshing) return;
  home.refreshing = true;
  if (btn) { btn.disabled = true; btn.innerHTML = `<span class="spinner"></span>Building…`; }
  if (force && state.view === 'home') {
    const row = $('#home-body .mix-row');
    row?.replaceChildren(...skeletonTiles(5));
  }
  try {
    home.data = await call('refresh_for_you', { force });
  } catch (e) {
    toast(`Couldn’t build mixes: ${errMsg(e)}`, 'error');
    if (home.data) home.data.stale = false;
  } finally {
    home.refreshing = false;
  }
  if (state.view === 'home') renderHome();
}

/** A small "this week" card on Home that links to the stats page. */
async function renderHomeStats() {
  const box = $('#home-stats');
  if (!box) return;
  let s;
  try { s = await call('listening_stats', { range: '4w' }); } catch { return; }
  if (!s.plays) { box.remove(); return; }
  const top = s.top_artists[0];
  const year = new Date().getFullYear();
  const wrappedTime = new Date().getMonth() >= 10;
  box.replaceChildren(
    h('div', { class: 'section-title' }, 'Your last 4 weeks', h('button', { class: 'btn small', style: 'margin-left:auto', onclick: () => showView('stats') }, 'All stats')),
    h('div', { class: 'stat-strip' },
      statCard(fmtMinutes(s.minutes), 'minutes listened'),
      statCard(s.plays.toLocaleString(), 'plays'),
      top ? statCard(top.name, 'top artist', top.thumbnail) : null,
      s.streak > 1 ? statCard(`${s.streak} days`, 'listening streak') : statCard(s.artists.toLocaleString(), 'artists'),
      h('button', { class: `stat-card wrapped-card${wrappedTime ? ' hot' : ''}`, onclick: () => openWrapped(year) },
        h('span', { class: 'wc-ico', html: icon('gift') }),
        h('div', {}, h('b', {}, `Your ${year} Wrapped`), h('span', {}, wrappedTime ? 'Your year in music is ready' : 'Your year so far')))));
}

function statCard(value, label, img) {
  return h('div', { class: 'stat-card' },
    img ? h('img', { src: img, alt: '', referrerpolicy: 'no-referrer', class: 'sc-img', onerror: (e) => e.target.remove() }) : null,
    h('div', {}, h('b', { title: value }, value), h('span', {}, label)));
}

// Mix page ------------------------------------------------------------------

function openMix(mix, { push = true } = {}) {
  showView('mix', { push, reopen: () => openMix(mix, { push: false }) });
  const kicker = { daily: 'Daily Mix · made for you', discover: 'Made for you · new every Monday', repeat: 'From your listening', rediscover: 'From your listening' }[mix.kind] || 'Mix';
  renderCollection($('#mix-body'), {
    id: `mix:${mix.id}`,
    kind: kicker,
    title: mix.title,
    tracks: mix.tracks,
    art: mixArt(mix.covers, 'hero-art'),
    cover: mix.covers[0],
    description: mix.subtitle,
    line: [h('b', {}, 'Tunewire')],
    actions: [
      h('button', { class: 'ab-icon', title: 'Save as playlist', html: icon('plus'), onclick: async () => {
        const date = new Date().toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
        const name = await promptModal({ title: 'Save as playlist', value: mix.kind === 'daily' || mix.kind === 'discover' ? `${mix.title} · ${date}` : mix.title, okLabel: 'Save' });
        if (!name) return;
        try { await call('create_playlist', { name, tracks: mix.tracks, sourceUrl: null }); toast(`Saved “${name}” to Your Library`, 'ok'); } catch (e) { toast(errMsg(e), 'error'); }
      } }),
      downloadRing(mix.tracks),
    ],
    menu: () => [{ label: 'Add to queue', icon: 'plus', onClick: () => enqueue(mix.tracks) }],
  });
}

// Stats page ----------------------------------------------------------------

const statsUi = { range: '4w', seq: 0 };
const RANGES = [['4w', 'Last 4 weeks'], ['6m', 'Last 6 months'], ['year', 'This year'], ['all', 'All time']];

async function renderStats(range = statsUi.range) {
  statsUi.range = range;
  const seq = ++statsUi.seq;
  const body = $('#stats-body');
  const seg = h('div', { class: 'segmented' }, ...RANGES.map(([k, label]) => h('button', { class: k === range ? 'active' : '', onclick: () => renderStats(k) }, label)));
  const year = new Date().getFullYear();
  const head = h('div', { class: 'page-head' }, h('h1', {}, 'Your stats'), seg,
    h('button', { class: 'btn wrapped-btn', style: 'margin-left:auto', html: `${icon('gift')}${year} Wrapped`, onclick: () => openWrapped(year) }));
  if (!body.firstChild) body.replaceChildren(head, h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Counting…'));
  let s;
  try { s = await call('listening_stats', { range }); } catch (e) { body.replaceChildren(head, h('div', { class: 'empty' }, h('p', {}, errMsg(e)))); return; }
  if (seq !== statsUi.seq) return;
  if (!s.plays && !s.skips) {
    body.replaceChildren(head, h('div', { class: 'empty' }, h('span', { class: 'big', html: icon('pulse') }), h('p', {}, 'Nothing here yet.'),
      h('p', { class: 'muted small' }, range === 'all' ? 'Play some music and your stats will build up here.' : 'No listening in this period. Try a longer range.')));
    return;
  }

  const cards = h('div', { class: 'stat-strip big' },
    statCard(fmtMinutes(s.minutes), `minutes · ${fmtHours(s.minutes)}`),
    statCard(s.plays.toLocaleString(), `plays${s.skips ? ` · ${s.skips} skipped` : ''}`),
    statCard(s.tracks.toLocaleString(), 'different songs'),
    statCard(s.artists.toLocaleString(), s.new_artists && range !== 'all' ? `artists · ${s.new_artists} new to you` : 'artists'),
    statCard(`${s.streak} ${s.streak === 1 ? 'day' : 'days'}`, `streak now · best ${s.longest_streak}`));

  // Listening over time.
  const max = Math.max(1, ...s.series.map((b) => b.minutes));
  const chart = h('div', { class: 'bar-chart' }, ...s.series.map((b, i) => h('div', { class: 'bar', style: `--h:${(b.minutes / max) * 100}%;--i:${i}`, title: `${b.label}: ${fmtMinutes(b.minutes)} min` },
    h('span', {}), h('em', {}, s.series.length > 14 && i % Math.ceil(s.series.length / 8) ? '' : b.label))));

  // Top artists.
  const artists = h('div', { class: 'top-artists' }, ...s.top_artists.slice(0, 10).map((a, i) => h('button', { class: 'top-artist', style: `--i:${i}`, onclick: () => openArtist(a.name, a.top_track) },
    h('div', { class: 'ta-img' }, a.thumbnail ? h('img', { src: a.thumbnail, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : null, h('span', { class: 'ta-rank' }, String(i + 1))),
    h('b', {}, a.name), h('span', { class: 'meta' }, `${a.plays} plays · ${fmtMinutes(a.minutes)} min`))));

  // Top songs.
  const topTracks = s.top_tracks.slice(0, 25);
  const thead = h('div', { class: 'table-head' });
  tableHead(thead);
  const songs = h('div', { class: 'track-list' }, ...topTracks.map((t, i) => trackRow(t.track, i, {
    onPlay: () => playList(topTracks.map((x) => x.track), i),
    action: h('span', { class: 'play-count', title: `${fmtMinutes(t.minutes)} min` }, `${t.plays}×`),
  })));

  // When you listen.
  const hmax = Math.max(1, ...s.hours);
  const hours = h('div', { class: 'hour-chart' }, ...s.hours.map((m, hr) => h('div', { class: 'hbar', style: `--h:${(m / hmax) * 100}%`, title: `${hr}:00 · ${fmtMinutes(m)} min` }, h('span', {}), h('em', {}, hr % 6 === 0 ? `${hr}:00` : ''))));
  const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  const dmax = Math.max(1, ...s.weekdays);
  const weekdays = h('div', { class: 'day-chart' }, ...s.weekdays.map((m, i) => h('div', { class: 'drow' }, h('span', {}, days[i]), h('div', { class: 'dtrack' }, h('i', { style: `width:${(m / dmax) * 100}%` })), h('em', {}, `${fmtMinutes(m)} min`))));
  const peak = s.hours.indexOf(Math.max(...s.hours));

  const total = s.platforms.reduce((a, b) => a + b.minutes, 0) || 1;
  const platforms = s.platforms.length ? h('div', { class: 'platform-split' },
    h('div', { class: 'ps-bar' }, ...s.platforms.map((p) => h('i', { style: `width:${(p.minutes / total) * 100}%`, class: `ps-${p.label.toLowerCase().replace(/[^a-z]/g, '')}`, title: `${p.label}: ${Math.round((p.minutes / total) * 100)}%` }))),
    h('div', { class: 'ps-legend' }, ...s.platforms.map((p) => h('span', {}, h('i', { class: `ps-${p.label.toLowerCase().replace(/[^a-z]/g, '')}` }), `${p.label} ${Math.round((p.minutes / total) * 100)}%`)))) : null;

  body.replaceChildren(head, cards,
    h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Listening time'), chart),
    s.top_artists.length ? h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Top artists'), artists) : null,
    topTracks.length ? h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Top songs',
      h('button', { class: 'btn small', style: 'margin-left:auto', html: `${icon('play')}Play all`, onclick: () => playList(topTracks.map((x) => x.track), 0) }),
      h('button', { class: 'btn small', html: `${icon('save')}Save as playlist`, onclick: async () => {
        const label = RANGES.find(([k]) => k === range)?.[1] || '';
        const name = await promptModal({ title: 'Save as playlist', value: `Top songs · ${label}`, okLabel: 'Save' });
        if (name) call('create_playlist', { name, tracks: topTracks.map((x) => x.track), sourceUrl: null }).then(() => toast(`Saved ${name}`, 'ok')).catch((e) => toast(errMsg(e), 'error'));
      } })), thead, songs) : null,
    h('div', { class: 'stats-two' },
      h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Time of day', h('span', { class: 'meta', style: 'margin-left:8px' }, `most around ${peak}:00`)), hours),
      h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Day of week'), weekdays)),
    platforms ? h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Where it played from'), platforms) : null,
    s.busiest_day ? h('p', { class: 'meta stats-foot' }, `Biggest day: ${s.busiest_day.label}, ${fmtMinutes(s.busiest_day.minutes)} minutes.${s.estimated ? ' Plays from before stats existed are estimated from your history.' : ''}`) : null);
  markPlaying();
}

// Wrapped -------------------------------------------------------------------

function personality(s) {
  const sum = (from, to) => s.hours.slice(from, to).reduce((a, b) => a + b, 0);
  const night = sum(0, 5) + s.hours[22] + s.hours[23];
  const morning = sum(5, 11);
  const total = s.hours.reduce((a, b) => a + b, 0) || 1;
  const explorer = s.artists ? s.new_artists / s.artists : 0;
  const loyal = s.top_artists[0] ? s.top_artists[0].minutes / (s.minutes || 1) : 0;
  if (night / total > 0.3) return { name: 'The Night Owl', text: 'Your music comes alive after dark. A big part of your listening happened between 10 pm and 5 am.' };
  if (morning / total > 0.35) return { name: 'The Early Riser', text: 'You start the day with music. Mornings are when you listen most.' };
  if (loyal > 0.25) return { name: 'The Devotee', text: `When you love an artist, you stay. ${s.top_artists[0].name} made up a quarter of your listening.` };
  if (explorer > 0.6) return { name: 'The Explorer', text: `Always looking for something new: ${s.new_artists} artists you'd never played before.` };
  if (s.longest_streak >= 14) return { name: 'The Daily Listener', text: `Music every single day, ${s.longest_streak} days in a row at your best.` };
  return { name: 'The All-Rounder', text: 'A bit of everything, any time of day. Your taste refuses to sit still.' };
}

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
      topA.thumbnail ? h('img', { class: 'w-photo round', src: topA.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : null,
      big(topA.name), h('p', { class: 'w-sub' }, `${topA.plays} plays · ${fmtMinutes(topA.minutes)} minutes together`)) } : null,
    s.top_artists.length > 1 ? { bg: palette[3], el: h('div', { class: 'w-slide w-list' }, h('p', {}, 'Your top artists'),
      h('ol', {}, ...s.top_artists.slice(0, 5).map((a) => h('li', {}, a.thumbnail ? h('img', { src: a.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : h('span', { class: 'w-noimg' }), h('b', {}, a.name), h('em', {}, `${fmtMinutes(a.minutes)} min`))))) } : null,
    topT ? { bg: palette[4], img: topT.track.thumbnail, el: h('div', { class: 'w-slide' }, h('p', {}, 'Your song of the year'),
      topT.track.thumbnail ? h('img', { class: 'w-photo', src: topT.track.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : null,
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
    h('div', { class: 'actions', style: 'justify-content:center' },
      h('button', { class: 'btn primary', html: `${icon('download')}Save image`, onclick: (e) => { e.stopPropagation(); saveWrappedImage(s, year, p, soFar); } }),
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
