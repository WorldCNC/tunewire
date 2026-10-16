'use strict';

// ===========================================================================
// Backend events
// ===========================================================================

async function wireEvents() {
  await listen('playback', (e) => {
    state.playback = e.payload;
    renderPlayback();
    syncLyrics();
    plugins.emit('playback', e.payload);
  });
  await listen('queue-changed', (e) => {
    state.queue = e.payload;
    renderQueue();
    plugins.emit('queue-changed', e.payload);
  });
  await listen('track-started', (e) => {
    plugins.emit('track-started', e.payload);
    loadLyrics();
    if (state.view === 'history') refreshHistory();
  });
  await listen('track-ended', (e) => plugins.emit('track-ended', e.payload));
  await listen('download', (e) => onDownloadEvent(e.payload));
  // Background completion of guessed YouTube Music metadata.
  await listen('metadata-updated', (e) => {
    const metas = new Map(e.payload.map((m) => [m.url, m]));
    let changed = false;
    for (const t of state.results) {
      for (const v of t.versions) {
        const m = metas.get(v.url);
        if (!m) continue;
        if (m.artist) v.artist = m.artist;
        v.duration = m.duration ?? v.duration;
        v.album = m.album ?? v.album;
        v.artist_url = m.artist_url ?? v.artist_url;
        v.partial = false;
        changed = true;
      }
      const ytm = t.versions.find((v) => v.platform === 'ytmusic');
      if (ytm && metas.has(ytm.url) && t.versions.every((v) => v.platform === 'ytmusic')) {
        t.artist = ytm.artist || t.artist;
        t.duration = t.duration ?? ytm.duration;
      }
    }
    if (changed && state.view === 'search' && !$('#search-body').hidden) renderResults(state.lastOnPlay || null);
  });
  await listen('status', (e) => {
    const { level, message } = e.payload;
    if (level !== 'info' || message !== 'End of queue' || state.queue.items.length) toast(message, level);
    plugins.emit('status', e.payload);
  });
  await listen('library-changed', (e) => {
    const what = e.payload;
    if (what === 'playlists') refreshPlaylists();
    if (what === 'favorites') refreshFavorites();
    if (what === 'history' && state.view === 'history') refreshHistory();
    if (what === 'downloads') refreshDownloads();
    if (what === 'local') refreshLocalCount();
    plugins.emit('library-changed', { what });
  });
}

// ===========================================================================
// Boot
// ===========================================================================

async function boot() {
  hydrateIcons();
  await wireEvents();

  try {
    state.settings = await call('get_settings');
  } catch (e) {
    toast(`Could not load settings: ${errMsg(e)}`, 'error');
    state.settings = { platform_order: ['ytmusic', 'soundcloud', 'youtube', 'spotify'], search_limit: 12, theme: 'midnight', last_source: 'all', plugin_settings: {}, enabled_plugins: [] };
  }
  setSource(state.settings.last_source || 'all');
  setVolumeUI(state.settings.volume ?? 0.7);

  try {
    applyTheme(await call('load_theme', { id: state.settings.theme }));
  } catch {
    applyTheme(await call('load_theme', { id: 'midnight' }).catch(() => ({ vars: {} })));
  }

  await refreshHistory().catch(() => {});
  renderHero();
  refreshTools();
  const health = await checkHealth();
  if (!health.ok) {
    openModal({
      title: 'yt-dlp is required',
      subtitle: health.message,
      body: h('div', {},
        h('p', {}, 'Tunewire uses yt-dlp for all search, playlist and stream extraction. Install a recent version and make sure it is on your PATH:'),
        h('pre', { class: 'card', style: 'font-size:12px;white-space:pre-wrap' },
          'Linux:   pipx install yt-dlp   (or your package manager)\nmacOS:   brew install yt-dlp\nWindows: winget install yt-dlp\n\nAlternatively set TUNEWIRE_YTDLP=/full/path/to/yt-dlp'),
        h('p', { class: 'muted' }, 'Then press Re-check.')),
      footer: [h('button', { class: 'btn primary', onclick: async (e) => { const r = await checkHealth(); if (r.ok) { modalStack.at(-1)?.close(); toast('yt-dlp found', 'ok'); } else toast(r.message, 'error'); } }, 'Re-check')],
    });
  } else if (/outdated/.test(health.message)) {
    toast(health.message, 'warn', 10000);
  }

  const [queue, playback] = await Promise.all([call('get_queue'), call('get_playback_state')]);
  state.queue = queue;
  state.playback = playback;
  renderQueue();
  await Promise.all([refreshPlaylists(), refreshFavorites(), refreshDownloads(), refreshLocalCount()]);
  renderPlayback();

  for (const id of state.settings.enabled_plugins || []) plugins.start(id);
  if (state.settings.auto_update !== false) setTimeout(() => checkForUpdate(false), 8000);
  $('#search-input').focus();
}

boot().catch((e) => toast(`Startup failed: ${errMsg(e)}`, 'error', 15000));
