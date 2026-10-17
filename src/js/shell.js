'use strict';

// ===========================================================================
// App shell: title bar & window controls, Your Library controls, the right
// panel's Now playing view, full screen player, sleep timer, shuffle, device.
// ===========================================================================

const appWindow = window.__TAURI__?.window?.getCurrentWindow?.() || null;

// Title bar ---------------------------------------------------------------------
$('#nav-back').addEventListener('click', goBack);
$('#nav-fwd').addEventListener('click', goForward);
$('#nav-home').addEventListener('click', () => showView('home'));
$('#search-browse').addEventListener('click', () => {
  $('#search-input').value = '';
  syncSearchClear();
  showView('search');
  showSearchBody(false);
  renderHero();
});
$('#nav-releases').addEventListener('click', () => showView('releases'));
$('#nav-menu').addEventListener('click', (e) => openMenu(e.currentTarget, [
  { label: 'Settings', icon: 'gear', onClick: () => showView('settings') },
  { label: 'Themes', icon: 'palette', onClick: () => showView('themes') },
  { label: 'Plugins', icon: 'puzzle', onClick: () => showView('plugins') },
  'sep',
  { label: 'Import playlists', icon: 'importIcon', onClick: () => openImport() },
  { label: 'Downloads', icon: 'download', onClick: () => showView('downloads') },
  { label: 'Local files', icon: 'folder', onClick: () => showView('local') },
  'sep',
  { label: 'Keyboard shortcuts', icon: 'keyboard', onClick: showShortcuts },
  { label: 'Check for updates', icon: 'refresh', onClick: () => checkForUpdate(true) },
]));

if (appWindow) {
  $('#win-min').addEventListener('click', () => appWindow.minimize());
  $('#win-max').addEventListener('click', () => appWindow.toggleMaximize());
  $('#win-close').addEventListener('click', () => appWindow.close());
  const syncMax = async () => {
    try {
      const max = await appWindow.isMaximized();
      $('#win-max').innerHTML = icon(max ? 'winRestore' : 'winMax');
      $('#win-max').title = max ? 'Restore' : 'Maximize';
      document.documentElement.classList.toggle('maximized', max);
    } catch { /* not available */ }
  };
  appWindow.onResized?.(syncMax);
  syncMax();
} else {
  $('#win-controls').hidden = true; // browser preview
}

// Your Library controls -----------------------------------------------------------
(() => {
  try {
    if (localStorage.getItem('tw-lib-collapsed') === '1') $('#app').classList.add('lib-collapsed');
    state.libSort = localStorage.getItem('tw-lib-sort') || 'recents';
  } catch { /* optional */ }
})();
$('#lib-collapse').addEventListener('click', () => {
  const on = $('#app').classList.toggle('lib-collapsed');
  $('#lib-collapse').title = on ? 'Expand Your Library' : 'Collapse Your Library';
  try { localStorage.setItem('tw-lib-collapsed', on ? '1' : '0'); } catch { /* optional */ }
});
$('#lib-create').addEventListener('click', (e) => openMenu(e.currentTarget, [
  { label: 'Create a new playlist', icon: 'plus', onClick: () => createPlaylist() },
  { label: 'Create a playlist folder', icon: 'folderPlus', onClick: async () => {
    const name = await promptModal({ title: 'New folder', okLabel: 'Create', placeholder: 'Folder name' });
    if (name) createPlaylist({ folder: name });
  } },
  'sep',
  { label: 'Import from Spotify', icon: 'importIcon', onClick: () => openSpotifyImport() },
  { label: 'Import a link or file', icon: 'link', onClick: () => openImport() },
]));
$('#lib-chips').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-filter]');
  if (!b) return;
  state.libFilter = state.libFilter === b.dataset.filter ? null : b.dataset.filter;
  renderLibrary();
});
$('#lib-search-btn').addEventListener('click', () => { $('#lib-search').classList.add('open'); $('#lib-search-input').focus(); });
$('#lib-search-input').addEventListener('input', (e) => { state.libQuery = e.target.value; renderLibrary(); });
$('#lib-search-input').addEventListener('blur', (e) => { if (!e.target.value) $('#lib-search').classList.remove('open'); });
$('#lib-sort').addEventListener('click', (e) => openMenu(e.currentTarget, [
  { label: 'Sort by', heading: true },
  ...Object.entries(LIB_SORTS).map(([k, l]) => ({ label: `${state.libSort === k ? '✓ ' : ''}${l}`, onClick: () => {
    state.libSort = k;
    try { localStorage.setItem('tw-lib-sort', k); } catch { /* optional */ }
    renderLibrary();
  } })),
]));

// Right panel ----------------------------------------------------------------------
$('#panel-close').addEventListener('click', () => setPanelOpen(false));
$('#btn-np').addEventListener('click', () => togglePanel('now'));
$('#btn-lyrics').addEventListener('click', () => togglePanel('lyrics'));
$('#np-cover').addEventListener('click', () => togglePanel('now'));
$('#np-title').addEventListener('click', () => { if (state.playingFrom && state.playingFrom !== 'liked') openPlaylist(state.playingFrom); else if (state.playingFrom === 'liked') showView('favorites'); });
(() => {
  try { if (localStorage.getItem('tw-panel-open') === '0') $('#app').classList.add('queue-hidden'); } catch { /* optional */ }
})();

// Now playing view: big cover, artist card, next in queue.
const npv = { key: null, artistCache: new Map(), seq: 0 };

function renderNowPlaying(force = false) {
  const box = $('#np-panel');
  if (!box || box.hidden) return;
  const p = state.playback;
  const track = p?.current?.track;
  if (!track) {
    npv.key = null;
    box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Nothing playing'), h('p', { class: 'small' }, 'Play a song and it shows up here.')));
    return;
  }
  const nextItem = state.queue.items[(state.queue.current ?? -1) + 1];
  const key = `${track.key}|${nextItem?.uid}|${state.favKeys.has(track.key)}`;
  if (!force && npv.key === key) return;
  npv.key = key;
  const thumb = p.version?.thumbnail || track.thumbnail;
  const big = thumb ? thumb.replace(/=w\d+-h\d+/, '=w800-h800').replace('/mqdefault.jpg', '/maxresdefault.jpg').replace('-t500x500.', '-t500x500.') : null;
  const cover = h('div', { class: 'npv-cover' });
  if (big) cover.append(h('img', { src: big, 'data-px': 640, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => { if (thumb && e.target.src !== thumb) e.target.src = thumb; else e.target.remove(); } }));
  else cover.innerHTML = icon('music');
  const liked = state.favKeys.has(track.key);
  const head = h('div', { class: 'npv-head' },
    h('div', {},
      h('div', { class: 'npv-title' }, track.title),
      track.artist ? h('button', { class: 'npv-artist', onclick: () => openArtist(track.artist, track) }, track.artist) : null),
    h('button', { class: `icon-btn like${liked ? ' on' : ''}`, title: liked ? 'Remove from Liked Songs' : 'Save to Liked Songs', html: icon(liked ? 'heartFill' : 'heart'), onclick: () => toggleFavorite(track) }));

  const about = h('div', { class: 'npv-card npv-about' });
  const nextCard = nextItem ? h('div', { class: 'npv-card' },
    h('h4', {}, 'Next in queue', h('button', { onclick: () => showPanel('queue') }, 'Open queue')),
    (() => {
      const row = h('div', { class: 'npv-next' }, thumbEl(nextItem.track.thumbnail),
        h('div', { class: 't-main' }, h('div', { class: 't-title' }, nextItem.track.title), h('div', { class: 't-sub' }, nextItem.track.artist || '')));
      row.addEventListener('click', () => call('play_index_cmd', { index: (state.queue.current ?? -1) + 1 }));
      return row;
    })()) : null;
  const via = $('#np-via').cloneNode(true);
  via.removeAttribute('id');
  const source = h('div', { class: 'npv-card' }, h('h4', {}, 'Playing from', h('button', { onclick: () => openVersions(track, { queueUid: p.current.uid, current: p.version?.platform }) }, 'Change source')),
    h('div', { class: 'npv-source' }, via, p.version?.album ? h('div', {}, `Album: ${p.version.album}`) : null));
  box.replaceChildren(...[cover, head, about, nextCard, source].filter(Boolean));
  if (track.artist) fillArtistCard(about, track);
  else about.remove();
}

async function fillArtistCard(el, track) {
  const name = track.artist.split(/, | & | feat\.? /i)[0].trim();
  const seq = ++npv.seq;
  let info = npv.artistCache.get(name);
  if (!info) {
    try {
      info = await call('ytm_artist', { name, browseId: artistBrowseId(track) });
      if (npv.artistCache.size > 30) npv.artistCache.delete(npv.artistCache.keys().next().value);
      npv.artistCache.set(name, info);
    } catch { el.remove(); return; }
  }
  if (seq !== npv.seq || !el.isConnected) return;
  const banner = h('div', { class: 'npv-banner' });
  if (info.banner || info.thumbnail) banner.style.backgroundImage = `url("${bannerUrl(info.banner || info.thumbnail, 640).replace(/"/g, '%22')}")`;
  el.replaceChildren(banner, h('div', { class: 'npv-about-body' },
    h('div', { class: 'npv-about-name' }, info.name),
    h('div', { class: 'npv-about-row' }, h('span', {}, info.audience || info.subscribers || ''), followButton(info.name, `https://www.youtube.com/channel/${info.browse_id}`, info.thumbnail)),
    info.description ? h('div', { class: 'npv-about-desc' }, info.description) : null));
  el.onclick = (e) => { if (!e.target.closest('button')) openArtist(info.name, track, { browseId: info.browse_id }); };
}

/** The YouTube Music channel id from a track's artist link, if any. */
function artistBrowseId(track) {
  const u = track?.versions?.map((v) => v.artist_url).find((x) => x && /\/channel\/UC/.test(x));
  return u ? u.match(/channel\/(UC[\w-]+)/)[1] : null;
}

// Player bar extras -----------------------------------------------------------------
$('#btn-shuffle').addEventListener('click', async () => {
  const on = !state.settings?.shuffle;
  try {
    await call('set_shuffle', { on });
    state.settings.shuffle = on;
    paintShuffle();
  } catch (e) { toast(errMsg(e), 'error'); }
});
function paintShuffle() {
  const on = !!(state.playback?.shuffle ?? state.settings?.shuffle);
  $('#btn-shuffle').classList.toggle('on', on);
  $('#btn-shuffle').title = on ? 'Disable shuffle' : 'Enable shuffle';
}

$('#btn-device').addEventListener('click', async (e) => {
  const anchor = e.currentTarget;
  let devs = [];
  try { devs = await call('list_output_devices'); } catch { /* none */ }
  const cur = state.settings?.output_device || '';
  openMenu(anchor, [
    { label: 'Connect to a device', heading: true },
    { label: `${cur === '' ? '✓ ' : ''}System default`, icon: 'speaker', onClick: () => setDevice('') },
    ...devs.map((d) => ({ label: `${cur === d.id ? '✓ ' : ''}${d.name}`, icon: 'speaker', onClick: () => setDevice(d.id) })),
  ]);
});
async function setDevice(id) {
  await updateSettings({ output_device: id });
  $('#btn-device').classList.toggle('on', !!id);
  toast('Output device changed', 'ok', 1500);
}

// Sleep timer
$('#btn-sleep').addEventListener('click', (e) => {
  const at = state.playback?.sleep_at;
  const set = (minutes, endOfTrack = false) => call('set_sleep_timer', { minutes, endOfTrack }).then((r) => {
    toast(r == null ? 'Sleep timer off' : r === 0 ? 'Playback will stop at the end of this song' : `Sleep timer: ${minutes} minutes`, 'ok', 2000);
  });
  openMenu(e.currentTarget, [
    { label: at != null ? (at === 0 ? 'Stops at the end of this song' : `Stops in ${Math.max(1, Math.round((at - Date.now()) / 60000))} min`) : 'Sleep timer', heading: true },
    ...[5, 10, 15, 30, 45, 60, 90].map((m) => ({ label: m < 60 ? `${m} minutes` : m === 60 ? '1 hour' : '1.5 hours', onClick: () => set(m) })),
    { label: 'End of song', onClick: () => set(null, true) },
    at != null ? 'sep' : null,
    at != null ? { label: 'Turn off timer', icon: 'x', onClick: () => set(null) } : null,
  ].filter(Boolean));
});
function paintSleep() {
  const btn = $('#btn-sleep');
  const at = state.playback?.sleep_at;
  btn.classList.toggle('on', at != null);
  btn.querySelector('.sleep-left')?.remove();
  if (at > 0) btn.append(h('span', { class: 'sleep-left' }, String(Math.max(1, Math.round((at - Date.now()) / 60000)))));
}
setInterval(paintSleep, 30000);

// Full screen player
const fsp = { el: null };
$('#btn-full').addEventListener('click', () => (fsp.el ? closeFullscreen() : openFullscreen()));
function openFullscreen() {
  const bg = h('div', { class: 'fs-bg' });
  const el = h('div', { class: 'fs-player', role: 'dialog', 'aria-label': 'Full screen' }, bg,
    h('button', { class: 'fs-close', title: 'Exit full screen (Esc)', html: icon('x'), onclick: closeFullscreen }),
    h('div', { class: 'fs-main' }, h('div', { class: 'fs-cover' }), h('div', { class: 'fs-meta' }, h('div', { class: 'fs-title' }), h('div', { class: 'fs-artist' }))),
    h('div', { class: 'fs-bottom' },
      h('div', { class: 'fs-progress' }, h('i')),
      h('div', { class: 'fs-row' }, h('span', { class: 'fs-pos' }), h('span', { class: 'fs-dur' })),
      h('div', { class: 'fs-controls' },
        h('button', { title: 'Previous', html: icon('prev'), onclick: () => call('previous_track') }),
        h('button', { class: 'fs-play', title: 'Play / pause', html: icon('play'), onclick: () => call('toggle_pause') }),
        h('button', { title: 'Next', html: icon('next'), onclick: () => call('next_track') }))));
  document.body.append(el);
  fsp.el = el;
  appWindow?.setFullscreen?.(true).catch(() => {});
  paintFullscreen();
}
function closeFullscreen() {
  fsp.el?.remove();
  fsp.el = null;
  appWindow?.setFullscreen?.(false).catch(() => {});
}
function paintFullscreen() {
  const el = fsp.el;
  if (!el) return;
  const p = state.playback;
  const t = p?.current?.track;
  const thumb = p?.version?.thumbnail || t?.thumbnail;
  const big = thumb ? thumb.replace(/=w\d+-h\d+/, '=w1200-h1200') : null;
  const cover = $('.fs-cover', el);
  if (cover.dataset.src !== (big || '')) {
    cover.dataset.src = big || '';
    cover.replaceChildren(...(big ? [h('img', { src: big, 'data-px': 1000, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => { if (thumb) e.target.src = thumb; } })] : []));
    el.style.setProperty('--fs-art', big ? `url("${big.replace(/"/g, '%22')}")` : 'none');
  }
  $('.fs-title', el).textContent = t?.title || 'Nothing playing';
  $('.fs-artist', el).textContent = t?.artist || '';
  $('.fs-play', el).innerHTML = icon(p?.status === 'playing' ? 'pause' : 'play');
  const dur = p?.duration || 0;
  $('.fs-progress i', el).style.width = `${dur ? Math.min(100, (p.position / dur) * 100) : 0}%`;
  $('.fs-pos', el).textContent = fmtTime(p?.position || 0);
  $('.fs-dur', el).textContent = fmtTime(dur);
}

// Problems with yt-dlp show up as a red pill in the title bar (otherwise hidden).
function paintHealth() {
  const bad = $('#health').classList.contains('bad');
  const pill = $('#health-bad');
  pill.hidden = !bad;
  if (bad) {
    pill.textContent = 'yt-dlp missing';
    pill.title = $('#health').title;
    pill.onclick = () => offerYtdlpInstall($('#health').title);
  }
}

/** Shown when yt-dlp isn't found: one click installs Tunewire's own copy. */
function offerYtdlpInstall(message) {
  const status = h('p', { class: 'muted small min-h-18' });
  const install = h('button', { class: 'btn primary' }, 'Install it for me');
  const m = openModal({
    title: 'yt-dlp is needed to play music',
    subtitle: message,
    body: h('div', {},
      h('p', {}, 'Tunewire uses yt-dlp to find and stream songs. It can download its own up-to-date copy (the official build from GitHub, checksum-verified). Nothing else on your computer is changed.'),
      h('details', {}, h('summary', { class: 'muted small pointer' }, 'Prefer to install it yourself?'),
        h('pre', { class: 'card fs-12 prewrap mt-8'}, 'Windows: winget install yt-dlp\nmacOS:   brew install yt-dlp\nLinux:   pipx install yt-dlp')),
      status),
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Not now'), install],
  });
  install.addEventListener('click', async () => {
    install.disabled = true;
    status.innerHTML = '<span class="spinner"></span> Downloading yt-dlp…';
    try {
      const r = await call('ytdlp_install');
      state.settings = await call('get_settings');
      const health = await checkHealth();
      paintHealth();
      if (health.ok) { m.close(); toast(`yt-dlp ${r.version || ''} installed. You’re ready to go.`, 'ok'); }
      else status.textContent = health.message;
    } catch (e) {
      status.textContent = errMsg(e);
      install.disabled = false;
      install.textContent = 'Try again';
    }
  });
}
