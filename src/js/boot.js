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
  // Progress-only updates (a few bytes, ~4×/s): no full re-render.
  await listen('playback-tick', (e) => {
    if (!state.playback) return;
    Object.assign(state.playback, e.payload);
    renderProgress();
    syncLyrics();
    plugins.emit('playback', state.playback);
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
    if (changed && state.view === 'search' && !$('#search-body').hidden) { renderResults(state.lastOnPlay || null); if (srch.tab === 'all') renderSearchAll(); }
  });
  await listen('status', (e) => {
    const { level, message } = e.payload;
    if (level !== 'info' || message !== 'End of queue' || state.queue.items.length) toast(message, level);
    plugins.emit('status', e.payload);
  });
  await listen('for-you-ready', () => { if (state.view === 'home') renderHome(); });
  await listen('library-changed', (e) => {
    const what = e.payload;
    if (what === 'playlists') refreshPlaylists();
    if (what === 'favorites') refreshFavorites();
    if (what === 'history' && state.view === 'history') refreshHistory();
    if (what === 'history' && state.view === 'stats') renderStats();
    if (what === 'downloads') refreshDownloads();
    if (what === 'local') refreshLocalCount();
    if (what === 'followed') refreshFollowed();
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

  // Everything the first screen needs, in parallel (all local reads).
  const theme = call('load_theme', { id: state.settings.theme }).catch(() => call('load_theme', { id: 'midnight' }).catch(() => ({ vars: {} })));
  const [t, queue, playback] = await Promise.all([theme, call('get_queue'), call('get_playback_state')]);
  applyTheme(t);
  updateAmbient();
  state.queue = queue;
  state.playback = playback;
  renderQueue();
  renderPlayback();
  await Promise.all([refreshHistory().catch(() => {}), refreshPlaylists(), refreshFavorites(), refreshDownloads(), refreshLocalCount(), refreshFollowed()]);
  renderHome();
  cloudInit().then(() => {
    $$('.cloud-only').forEach((el) => { el.hidden = !cloudOn(); });
    paintJamButton();
    // A tunewire:// link that started the app.
    invoke('plugin:deep-link|get_current').then((urls) => (urls || []).forEach(handleShareLink)).catch(() => {});
  });
  try { showPanel(localStorage.getItem('tw-panel') || 'queue'); } catch { showPanel('queue'); }
  document.body.classList.toggle('show-sources', !!state.settings.show_sources);

  // Slower checks (they start processes or use the network) after first paint.
  refreshTools();
  checkHealth().then((health) => {
    paintHealth();
    if (!health.ok) offerYtdlpInstall(health.message);
    else if (/outdated/.test(health.message)) toast(health.message, 'warn', 10000);
  });

  for (const id of state.settings.enabled_plugins || []) plugins.start(id);
  if (state.settings.auto_update !== false) scheduleUpdateChecks();
  showWhatsNew();
  setTimeout(checkReleasesQuietly, 5000);
}

boot().catch((e) => toast(`Startup failed: ${errMsg(e)}`, 'error', 15000));
