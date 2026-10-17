'use strict';

// ===========================================================================
// Search: songs from every source, plus a top result, artists, albums and
// playlists from YouTube Music. Before typing: recent searches and Browse all.
// ===========================================================================

let searchSeq = 0;
const srch = { q: '', tab: 'all', extras: null, extrasErr: null, songsDone: false };

function setSource(src, save = false) {
  state.source = src;
  for (const b of $$('#source-select button')) b.classList.toggle('active', b.dataset.source === src);
  if (save) call('update_settings', { patch: { last_source: src } }).catch(() => {});
}

$('#source-select').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-source]');
  if (!b) return;
  setSource(b.dataset.source, true);
  if (srch.q && !isUrl(srch.q)) searchSongs(srch.q, ++searchSeq);
});

$('#search-form').addEventListener('submit', (e) => {
  e.preventDefault();
  const q = $('#search-input').value.trim();
  if (q) doSearch(q);
});

function syncSearchClear() {
  $('#search-form').classList.toggle('has-text', !!$('#search-input').value);
}
$('#search-input').addEventListener('input', syncSearchClear);
$('#search-clear').addEventListener('click', () => {
  $('#search-input').value = '';
  syncSearchClear();
  searchSeq++;
  srch.q = '';
  showSearchBody(false);
  renderHero();
  $('#search-input').focus();
});

$('#search-chips').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-tab]');
  if (b) setSearchTab(b.dataset.tab);
});

function setSearchTab(tab) {
  srch.tab = tab;
  for (const b of $$('#search-chips button')) b.classList.toggle('active', b.dataset.tab === tab);
  $('#search-all').hidden = tab !== 'all';
  $('#search-songs').hidden = tab !== 'songs';
  $('#search-grid').hidden = !['artists', 'albums', 'playlists'].includes(tab);
  if (tab === 'all') renderSearchAll();
  if (['artists', 'albums', 'playlists'].includes(tab)) renderSearchGrid();
  $('#view-search').scrollTop = 0;
}

async function doSearch(q, { push = true } = {}) {
  // Tunewire share links open the shared thing instead of searching.
  if (handleShareLink(q)) { $('#search-input').value = ''; syncSearchClear(); return; }
  showView('search', { push, reopen: () => { $('#search-input').value = q; syncSearchClear(); doSearch(q, { push: false }); } });
  showSearchBody(true);
  state.sort = null;
  srch.q = q;
  srch.extras = null;
  srch.extrasErr = null;
  srch.songsDone = false;
  state.results = [];
  if (!isUrl(q)) rememberSearch(q);
  const seq = ++searchSeq;
  $('#search-warnings').replaceChildren();

  if (isUrl(q)) {
    $('#search-chips').hidden = true;
    setSearchTab('songs');
    $('#search-results').replaceChildren(...skeletonRows());
    $('#search-meta').textContent = 'Opening link…';
    try {
      const pl = await call('play_url', { url: q });
      if (seq !== searchSeq) return;
      state.results = pl.tracks;
      $('#search-meta').textContent = pl.tracks.length > 1 ? `${pl.title || 'Link'} · ${pl.tracks.length} songs · playing as queue` : `${pl.title || 'Link'} · playing`;
      renderResults(pl.tracks.length > 1 ? (i) => playList(pl.tracks, i) : null);
      if (pl.tracks.length > 1) {
        $('#search-warnings').append(h('div', { class: 'notice' }, 'Opened as a playlist. ',
          h('button', { class: 'btn small', onclick: async () => {
            try {
              const created = await call('create_playlist', { name: pl.title || 'Playlist', tracks: pl.tracks, sourceUrl: pl.url });
              toast('Saved to Your Library', 'ok');
              openPlaylist(created.id);
            } catch (e) { toast(errMsg(e), 'error'); }
          } }, 'Save to Your Library')));
      }
    } catch (e) {
      if (seq !== searchSeq) return;
      $('#search-meta').textContent = 'Could not open link';
      $('#search-results').replaceChildren(h('div', { class: 'empty' }, h('p', {}, errMsg(e))));
    }
    return;
  }

  $('#search-chips').hidden = false;
  setSearchTab(srch.tab === 'songs' || srch.tab === 'all' ? srch.tab : 'all');
  call('ytm_search_extras', { query: q }).then((x) => {
    if (seq !== searchSeq) return;
    srch.extras = x;
    if (srch.tab === 'all') renderSearchAll();
    else if (srch.tab !== 'songs') renderSearchGrid();
  }).catch((e) => {
    if (seq !== searchSeq) return;
    srch.extrasErr = errMsg(e);
    srch.extras = { top: null, artists: [], albums: [], playlists: [] };
    if (srch.tab === 'all') renderSearchAll();
  });
  searchSongs(q, seq);
}

async function searchSongs(q, seq) {
  const results = $('#search-results');
  $('#results-head').hidden = true;
  results.replaceChildren(...skeletonRows());
  $('#search-meta').innerHTML = '<span class="spinner"></span>';
  const t0 = performance.now();
  try {
    const res = await call('search_tracks', { query: q, source: state.source, limit: state.settings?.search_limit });
    if (seq !== searchSeq) return;
    state.results = res.tracks;
    srch.songsDone = true;
    const dupes = res.tracks.reduce((n, t) => n + t.versions.length, 0);
    const where = state.source === 'all' ? 'all sources' : PLATFORMS[state.source]?.label;
    $('#search-meta').textContent = `${res.tracks.length} songs from ${dupes} results on ${where} · ${res.cached ? 'cached' : ((performance.now() - t0) / 1000).toFixed(1) + ' s'}`;
    for (const w of res.warnings) $('#search-warnings').append(h('div', { class: 'notice' }, w));
    results.classList.add('stagger');
    renderResults();
    setTimeout(() => results.classList.remove('stagger'), 700);
    if (srch.tab === 'all') renderSearchAll();
    res.tracks.slice(0, 2).forEach(prefetchTrack);
    plugins.emit('search-completed', { query: q, source: state.source, tracks: res.tracks });
  } catch (e) {
    if (seq !== searchSeq) return;
    srch.songsDone = true;
    $('#search-meta').textContent = '';
    results.replaceChildren(h('div', { class: 'empty' }, h('p', {}, errMsg(e))));
    if (e.kind === 'yt_dlp_missing') offerYtdlpInstall(errMsg(e));
    if (srch.tab === 'all') renderSearchAll();
  }
}

/** Opens what a card points to. */
function openCard(c) {
  if (c.kind === 'artist') return openArtist(c.title, null, { browseId: c.browse_id, thumbnail: c.thumbnail });
  if (c.track && (c.kind === 'song' || c.kind === 'video')) return playTrack(c.track);
  if (c.browse_id) return openAlbum({ browseId: c.browse_id, title: c.title });
}

function playCard(c) {
  if (c.track) return playTrack(c.track);
  if (c.kind === 'artist') return call('ytm_artist', { name: c.title, browseId: c.browse_id }).then((a) => playList(a.top_songs, 0)).catch((e) => toast(errMsg(e), 'error'));
  if (c.browse_id) return call('ytm_collection', { id: c.browse_id }).then((col) => playList(col.tracks, 0)).catch((e) => toast(errMsg(e), 'error'));
}

function cardSub(c) {
  if (c.kind === 'artist') return 'Artist';
  const parts = c.subtitle.split(' • ').filter((p) => p && !/views|plays|subscribers/.test(p));
  return parts.join(' • ');
}

/** A tile: cover, title, subtitle and a play button on hover. */
function cardTile(c, i = 0) {
  const art = h('div', { class: 'tile-art' });
  if (c.thumbnail) art.append(h('img', { src: c.thumbnail, 'data-px': 300, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => { e.target.remove(); art.innerHTML = icon(c.kind === 'artist' ? 'user' : 'disc'); } }));
  else art.innerHTML = icon(c.kind === 'artist' ? 'user' : 'disc');
  art.append(h('button', { class: 'tile-play', title: `Play ${c.title}`, html: icon('play'), onclick: (e) => { e.stopPropagation(); playCard(c); } }));
  const tile = h('div', { class: `tile${c.kind === 'artist' ? ' round' : ''}`, style: `--i:${i}`, role: 'button', tabindex: '0', title: c.title },
    art, h('div', { class: 'tile-title' }, c.title), h('div', { class: 'tile-sub' }, cardSub(c)));
  tile.addEventListener('click', () => openCard(c));
  tile.addEventListener('keydown', (e) => { if (e.key === 'Enter') openCard(c); });
  return tile;
}

function cardRow(cards, limit = 7) {
  return h('div', { class: 'card-row' }, ...cards.slice(0, limit).map((c, i) => cardTile(c, i)));
}

function renderSearchAll() {
  const box = $('#search-all');
  const x = srch.extras;
  const songs = state.results.slice(0, 4);
  const blocks = [];

  // Top result + songs.
  let topEl;
  if (!x) topEl = h('div', { class: 'top-card skeleton' });
  else if (x.top) {
    const c = x.top;
    const art = h('div', { class: `tc-art${c.kind === 'artist' ? ' round' : ''}` });
    if (c.thumbnail) art.append(h('img', { src: c.thumbnail, 'data-px': 200, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }));
    topEl = h('div', { class: 'top-card', role: 'button', tabindex: '0' }, art,
      h('div', { class: 'tc-title' }, c.title),
      h('div', { class: 'tc-sub' }, h('span', { class: 'tc-kind' }, c.kind[0].toUpperCase() + c.kind.slice(1)), c.kind === 'artist' ? null : h('span', {}, c.subtitle.split(' • ').slice(1, 2).join(''))),
      h('button', { class: 'tile-play', title: `Play ${c.title}`, html: icon('play'), onclick: (e) => { e.stopPropagation(); playCard(c); } }));
    topEl.addEventListener('click', () => openCard(c));
  } else if (songs[0]) {
    const t = songs[0];
    const art = h('div', { class: 'tc-art' });
    if (t.thumbnail) art.append(h('img', { src: t.thumbnail, alt: '', referrerpolicy: 'no-referrer' }));
    topEl = h('div', { class: 'top-card', role: 'button' }, art, h('div', { class: 'tc-title' }, t.title), h('div', { class: 'tc-sub' }, h('span', { class: 'tc-kind' }, 'Song'), h('span', {}, t.artist)),
      h('button', { class: 'tile-play', html: icon('play'), onclick: (e) => { e.stopPropagation(); playTrack(t); } }));
    topEl.addEventListener('click', () => playTrack(t));
  }
  const songList = h('div', { class: 'track-list top-songs' });
  if (!srch.songsDone) songList.append(...skeletonRows(4));
  else songList.append(...songs.map((t, i) => trackRow(t, i)));
  songList._ctx = {};
  if (!srch.songsDone || songs.length || topEl) {
    blocks.push(h('div', { class: 'search-top' },
      topEl ? h('div', {}, h('h2', {}, 'Top result'), topEl) : h('div'),
      h('div', {}, h('h2', {}, 'Songs', state.results.length > 4 ? h('button', { class: 'btn small', onclick: () => setSearchTab('songs') }, 'Show all') : null), songList)));
  }
  if (x) {
    const shelf = (title, cards, tab) => (cards.length ? h('div', {}, h('div', { class: 'section-title' }, title, h('button', { class: 'btn small ml-auto', onclick: () => setSearchTab(tab) }, 'Show all')), cardRow(cards, 7)) : null);
    blocks.push(shelf('Artists', x.artists, 'artists'), shelf('Albums', x.albums, 'albums'), shelf('Playlists', x.playlists, 'playlists'));
    if (srch.extrasErr && !state.results.length) blocks.push(h('div', { class: 'empty' }, h('p', {}, srch.extrasErr)));
  }
  if (x && srch.songsDone && !songs.length && !x.top && !x.artists.length && !x.albums.length) {
    blocks.push(h('div', { class: 'empty' }, h('p', {}, `No results found for “${srch.q}”`), h('p', { class: 'small' }, 'Check the spelling, or try fewer or different words.')));
  }
  box.replaceChildren(...blocks.filter(Boolean));
  markPlaying();
}

function renderSearchGrid() {
  const box = $('#search-grid');
  const x = srch.extras;
  if (!x) { box.replaceChildren(loadingBox('Searching', skeletonCards(8))); return; }
  const cards = x[srch.tab] || [];
  box.replaceChildren(cards.length ? h('div', { class: 'card-row' }, ...cards.map((c, i) => cardTile(c, i))) : h('div', { class: 'empty' }, h('p', {}, `No ${srch.tab} found.`)));
}

// Search history lives in settings (the backend records each search).
function recentSearches() {
  return (state.settings?.search_history || []).slice(0, 12);
}
function rememberSearch(q) {
  if (state.settings) state.settings.search_history = [q, ...(state.settings.search_history || []).filter((x) => x.toLowerCase() !== q.toLowerCase())].slice(0, 30);
}

// Search home: recent searches + Browse all ----------------------------------------
async function renderHero() {
  const home = $('#search-home');
  const blocks = [];
  const recent = recentSearches();
  if (recent.length) {
    blocks.push(h('div', { class: 'search-home-block' },
      h('div', { class: 'section-title mt-0' }, 'Recent searches',
        h('button', { class: 'btn small ml-auto', onclick: async () => { await call('clear_search_history', {}).catch(() => {}); state.settings.search_history = []; renderHero(); } }, 'Clear recent searches')),
      h('div', { class: 'recent-cards' }, ...recent.map((q) => h('button', { class: 'recent-card', onclick: () => { $('#search-input').value = q; syncSearchClear(); doSearch(q); } },
        h('span', { class: 'rc-ico', html: icon('clock') }), h('span', {}, q))))));
  }
  const browse = h('div', { class: 'search-home-block' }, h('div', { class: 'section-title', style: recent.length ? '' : 'margin-top:0' }, 'Browse all'));
  blocks.push(browse);
  home.replaceChildren(...blocks);
  if (!state.moods) {
    browse.append(h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Loading…'));
    try { state.moods = await call('ytm_moods'); } catch (e) { browse.lastChild.replaceWith(h('p', { class: 'muted' }, errMsg(e))); return; }
  }
  const tiles = [];
  for (const sec of state.moods) {
    for (const m of sec.items) tiles.push(m);
  }
  if (browse.lastChild?.classList?.contains('loading-line')) browse.lastChild.remove();
  const grid = h('div', { class: 'browse-grid' }, ...tiles.map((m, i) => {
    const tile = h('button', { class: 'browse-tile', style: `--c:${saturate(m.color)};--i:${i}`, onclick: () => openMood(m) }, h('span', {}, m.title));
    const img = moodArt.get(m.params);
    if (img) tile.append(h('img', { src: img, 'data-px': 160, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }));
    tile._mood = m;
    return tile;
  }));
  browse.append(grid);
  fillMoodArt(grid);
}

// Each tile shows the cover of that category's top playlist (cached).
const moodArt = new Map(Object.entries((() => { try { return JSON.parse(localStorage.getItem('tw-mood-art') || '{}'); } catch { return {}; } })()));
async function fillMoodArt(grid) {
  const todo = [...grid.children].filter((t) => !moodArt.has(t._mood.params));
  const worker = async () => {
    while (todo.length) {
      const tile = todo.shift();
      try {
        const shelves = await call('ytm_mood_page', { params: tile._mood.params });
        const url = shelves.flatMap((s) => s.cards).find((c) => c.thumbnail)?.thumbnail;
        if (!url) continue;
        moodArt.set(tile._mood.params, url);
        if (tile.isConnected && !tile.querySelector('img')) tile.append(h('img', { src: url, 'data-px': 160, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }));
      } catch { /* leave it plain */ }
    }
  };
  await Promise.all(Array.from({ length: 6 }, worker));
  try { localStorage.setItem('tw-mood-art', JSON.stringify(Object.fromEntries(moodArt))); } catch { /* optional */ }
}

/** YouTube's stripe colors are pastel; deepen them for white text. */
function saturate(hex) {
  const n = parseInt(hex.slice(1), 16);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  const k = 0.62;
  return `rgb(${Math.round(r * k)}, ${Math.round(g * k)}, ${Math.round(b * k)})`;
}

let moodSeq = 0;
async function openMood(m, { push = true } = {}) {
  const seq = ++moodSeq;
  showView('mood', { push, reopen: () => openMood(m, { push: false }) });
  const body = $('#mood-body');
  body.replaceChildren(
    h('div', { class: 'hero', style: `--hero:${saturate(m.color)};min-height:220px` }, h('div', { class: 'hero-meta' }, h('div', { class: 'hero-kicker' }, 'Browse'), h('h1', { class: 'hero-title' }, m.title))),
    loadingBox('Loading', h('div', { class: 'section-title sk w-20' }), skeletonCards(6), h('div', { class: 'section-title sk w-20' }), skeletonCards(6)));
  let shelves;
  try { shelves = await call('ytm_mood_page', { params: m.params }); } catch (e) { if (seq === moodSeq) body.lastChild.replaceWith(h('div', { class: 'empty' }, h('p', {}, errMsg(e)))); return; }
  if (seq !== moodSeq) return;
  body.lastChild.replaceWith(h('div', {}, ...shelves.map((s) => h('div', {}, h('div', { class: 'section-title' }, s.title), cardRow(s.cards, 14)))));
}

function showSearchBody(show) {
  $('#search-home').hidden = show;
  $('#search-body').hidden = !show;
}

function renderResults(onPlayIndex = null) {
  const list = $('#search-results');
  if (!state.results.length) {
    $('#results-head').hidden = true;
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'No songs found.'), h('p', { class: 'small' }, 'Try other words or another source.')));
    return;
  }
  // Sorting keeps the original (relevance) index for playlist playback.
  const indexed = state.results.map((t, i) => [t, i]);
  const s = state.sort;
  if (s) {
    const get = (t) => (s.key === 'duration' ? t.duration || 0 : s.key === 'album' ? trackAlbum(t).toLowerCase() : (t[s.key] || '').toLowerCase());
    indexed.sort(([a], [b]) => (get(a) > get(b) ? 1 : get(a) < get(b) ? -1 : 0) * s.dir);
  }
  tableHead($('#results-head'), {
    sort: s,
    onSort: (key) => {
      state.sort = state.sort?.key === key ? (state.sort.dir === 1 ? { key, dir: -1 } : null) : { key, dir: 1 };
      renderResults(onPlayIndex);
    },
  });
  list.replaceChildren(...indexed.map(([t, i], n) => trackRow(t, n, onPlayIndex ? { index: i, onPlay: () => onPlayIndex(i) } : { index: i })));
  list._ctx = {};
  state.lastOnPlay = onPlayIndex;
  markPlaying();
}
