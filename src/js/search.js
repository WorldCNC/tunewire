'use strict';

// ===========================================================================
// Search
// ===========================================================================

let searchSeq = 0;

function setSource(src, save = false) {
  state.source = src;
  for (const b of $$('#source-select button')) b.classList.toggle('active', b.dataset.source === src);
  if (save) call('update_settings', { patch: { last_source: src } }).catch(() => {});
}

$('#source-select').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-source]');
  if (!b) return;
  setSource(b.dataset.source, true);
  const q = $('#search-input').value.trim();
  if (q && !isUrl(q)) doSearch(q);
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
  showSearchBody(false);
  renderHero();
  $('#search-input').focus();
});

async function doSearch(q) {
  showView('search');
  showSearchBody(true);
  $('#results-head').hidden = true;
  state.sort = null;
  if (!isUrl(q)) rememberSearch(q);
  const seq = ++searchSeq;
  const results = $('#search-results');
  const warn = $('#search-warnings');
  warn.replaceChildren();
  results.replaceChildren(...skeletonRows());

  if (isUrl(q)) {
    $('#search-title').textContent = 'Opening link…';
    $('#search-meta').textContent = '';
    try {
      const pl = await call('play_url', { url: q });
      if (seq !== searchSeq) return;
      state.results = pl.tracks;
      $('#search-title').textContent = pl.title || 'Link';
      $('#search-meta').textContent = pl.tracks.length > 1 ? `${pl.tracks.length} tracks · playing as queue` : 'Playing';
      renderResults(pl.tracks.length > 1 ? (i) => playList(pl.tracks, i) : null);
      if (pl.tracks.length > 1) {
        warn.append(h('div', { class: 'notice' }, 'Loaded as a playlist. ',
          h('button', { class: 'btn small', onclick: async () => {
            const name = await promptModal({ title: 'Save playlist', value: pl.title, okLabel: 'Save' });
            if (name) call('create_playlist', { name, tracks: pl.tracks, sourceUrl: pl.url }).then(() => toast('Saved', 'ok')).catch((e) => toast(errMsg(e), 'error'));
          } }, 'Save as local playlist')));
      }
    } catch (e) {
      if (seq !== searchSeq) return;
      $('#search-title').textContent = 'Could not open link';
      results.replaceChildren(h('div', { class: 'empty' }, h('p', {}, errMsg(e))));
    }
    return;
  }

  $('#search-title').textContent = q;
  $('#search-meta').innerHTML = `<span class="spinner"></span>`;
  const t0 = performance.now();
  try {
    const res = await call('search_tracks', { query: q, source: state.source, limit: state.settings?.search_limit });
    if (seq !== searchSeq) return;
    state.results = res.tracks;
    const dupes = res.tracks.reduce((n, t) => n + t.versions.length, 0);
    const where = state.source === 'all' ? 'all sources' : PLATFORMS[state.source]?.label;
    $('#search-meta').textContent = `${res.tracks.length} songs from ${dupes} results on ${where} · ${res.cached ? 'cached' : ((performance.now() - t0) / 1000).toFixed(1) + ' s'}`;
    for (const w of res.warnings) warn.append(h('div', { class: 'notice' }, w));
    const list = $('#search-results');
    list.classList.add('stagger');
    renderResults();
    setTimeout(() => list.classList.remove('stagger'), 700);
    if (res.tracks[0]) prefetchTrack(res.tracks[0]);
    plugins.emit('search-completed', { query: q, source: state.source, tracks: res.tracks });
  } catch (e) {
    if (seq !== searchSeq) return;
    $('#search-meta').textContent = '';
    results.replaceChildren(h('div', { class: 'empty' }, h('p', {}, errMsg(e)),
      e.kind === 'yt_dlp_missing' ? h('p', { class: 'muted' }, 'Install yt-dlp (https://github.com/yt-dlp/yt-dlp) and restart Tunewire.') : null));
  }
}

// Search history lives in settings (the backend records each search).
function recentSearches() {
  return (state.settings?.search_history || []).slice(0, 10);
}
function rememberSearch(q) {
  if (state.settings) state.settings.search_history = [q, ...(state.settings.search_history || []).filter((x) => x.toLowerCase() !== q.toLowerCase())].slice(0, 30);
}

function renderHero() {
  const home = $('#search-home');
  const spot = state.tools?.spotdl;
  const intro = h('p', { class: 'home-intro' },
    'Searches ', badge('ytmusic'), ', ', badge('soundcloud'), ', ', badge('youtube'),
    [' and ', badge('spotify')],
    ' at once. Each song appears once, with an icon for every source that has it. You can also paste a track or playlist link.');
  const blocks = [h('div', { class: 'home-block' }, h('h1', { style: 'margin-bottom:8px' }, 'Search'), intro)];
  const recent = recentSearches();
  if (recent.length) {
    blocks.push(h('div', { class: 'home-block' },
      h('div', { class: 'section-title' }, 'Recent searches',
        h('button', { class: 'btn small', style: 'margin-left:auto', onclick: async () => { await call('clear_search_history', {}).catch(() => {}); state.settings.search_history = []; renderHero(); } }, 'Clear')),
      h('div', { class: 'recent-searches' }, ...recent.map((q) => h('button', { onclick: () => { $('#search-input').value = q; syncSearchClear(); doSearch(q); } }, q)))));
  }
  const played = (state.history || []).slice(0, 8);
  if (played.length) {
    const head = h('div', { class: 'table-head' });
    tableHead(head);
    blocks.push(h('div', { class: 'home-block' }, h('div', { class: 'section-title' }, 'Recently played'), head,
      h('div', { class: 'track-list' }, ...played.map((e, i) => trackRow(e.track, i, { onPlay: () => playTrack(e.track, e.platform) })))));
  }
  home.replaceChildren(...blocks);
  markPlaying();
}

function showSearchBody(show) {
  $('#search-home').hidden = show;
  $('#search-body').hidden = !show;
}



function renderResults(onPlayIndex = null) {
  const list = $('#search-results');
  if (!state.results.length) {
    $('#results-head').hidden = true;
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'No results.'), h('p', { class: 'small' }, 'Try other words or another source.')));
    return;
  }
  // Sorting keeps the original (relevance) index for playlist playback.
  const indexed = state.results.map((t, i) => [t, i]);
  const s = state.sort;
  if (s) {
    const get = (t) => (s.key === 'duration' ? t.duration || 0 : (t[s.key] || '').toLowerCase());
    indexed.sort(([a], [b]) => (get(a) > get(b) ? 1 : get(a) < get(b) ? -1 : 0) * s.dir);
  }
  tableHead($('#results-head'), {
    sort: s,
    onSort: (key) => {
      state.sort = state.sort?.key === key ? (state.sort.dir === 1 ? { key, dir: -1 } : null) : { key, dir: 1 };
      renderResults(onPlayIndex);
    },
  });
  list.replaceChildren(...indexed.map(([t, i], n) => trackRow(t, n, onPlayIndex ? { onPlay: () => onPlayIndex(i) } : {})));
  state.lastOnPlay = onPlayIndex;
  markPlaying();
}
