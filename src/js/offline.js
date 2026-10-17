'use strict';

// ===========================================================================
// Downloads & local files
// ===========================================================================

async function refreshDownloads() {
  try { state.downloads = await call('list_downloads'); } catch { state.downloads = []; }
  state.downloadKeys = new Set(state.downloads.map((d) => d.key));
  renderLibrary();
  paintDownloadRings();
  if (state.view === 'downloads') renderDownloads();
}

async function downloadTracks(tracks) {
  const todo = tracks.filter((t) => !state.downloadKeys.has(t.key) && !t.versions.every((v) => v.platform === 'local'));
  if (!todo.length) return toast('Already downloaded', 'info', 1500);
  try {
    const n = await call('download_tracks', { tracks: todo });
    toast(n === 1 ? `Downloading “${todo[0].title}”` : `Downloading ${n} songs`, 'info', 2000);
  } catch (e) { toast(errMsg(e), 'error'); }
}

// Download button for a whole list: a ring fills up as its songs finish.
const RING_C = 2 * Math.PI * 15;

function downloadRing(tracks) {
  const btn = h('button', { class: 'ab-icon dl-ring' });
  btn._tracks = tracks;
  btn.addEventListener('click', async () => {
    const st = ringState(tracks);
    if (st.active) return toast(`Downloading · ${st.done} of ${st.total} songs ready`, 'info', 2000);
    if (st.total && st.done === st.total) {
      if (await confirmModal({ title: 'Remove from downloads?', message: `The ${st.total} downloaded songs will be deleted from this computer. They stay in the playlist.`, okLabel: 'Remove', danger: true })) {
        for (const t of tracks) if (state.downloadKeys.has(t.key)) await call('delete_download', { key: t.key }).catch(() => {});
        toast('Removed from downloads', 'info', 1800);
      }
      return;
    }
    await downloadTracks(tracks);
    paintRing(btn);
  });
  paintRing(btn);
  return btn;
}

function ringState(tracks) {
  const eligible = tracks.filter((t) => !t.versions.every((v) => v.platform === 'local'));
  const done = eligible.filter((t) => state.downloadKeys.has(t.key)).length;
  const active = eligible.some((t) => state.downloading.has(t.key));
  return { total: eligible.length, done, active };
}

function paintRing(btn) {
  const st = ringState(btn._tracks);
  const full = st.total > 0 && st.done === st.total;
  const frac = st.total ? st.done / st.total : 0;
  btn.classList.toggle('done', full);
  btn.classList.toggle('active', st.active);
  btn.title = full ? 'Downloaded · click to remove' : st.active ? `Downloading · ${st.done} of ${st.total}` : 'Download';
  const key = `${full}|${st.active}|${st.done}`;
  if (btn.dataset.k === key) return;
  btn.dataset.k = key;
  if (full) { btn.innerHTML = '<span class="dl-done">' + icon('download') + '</span>'; return; }
  if (!st.active) { btn.innerHTML = icon('download'); return; }
  btn.innerHTML = `<svg class="ring" viewBox="0 0 36 36" aria-hidden="true"><circle cx="18" cy="18" r="15" class="ring-bg"/><circle cx="18" cy="18" r="15" class="ring-fg" style="stroke-dasharray:${RING_C};stroke-dashoffset:${RING_C * (1 - frac)}"/></svg>${icon('download')}`;
}

function paintDownloadRings() {
  for (const b of $$('.dl-ring')) paintRing(b);
}

async function deleteDownload(key) {
  try { await call('delete_download', { key }); toast('Download removed', 'info', 1500); } catch (e) { toast(errMsg(e), 'error'); }
}

function fmtSize(b) {
  return b > 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${(b / 1e6).toFixed(1)} MB`;
}

function onDownloadEvent({ key, title, state: s, message }) {
  if (s === 'done' || s === 'error') state.downloading.delete(key);
  else state.downloading.set(key, { title, state: s });
  if (s === 'done') state.downloadKeys.add(key);
  paintDownloadRings();
  if (s === 'error') toast(`Download failed: ${title}. ${message || ''}`, 'error');
  if (s === 'done' && state.downloading.size === 0) toast('Downloads finished', 'ok', 2000);
  renderDownloadActivity();
}

function renderDownloadActivity() {
  const box = $('#dl-active');
  if (!box) return;
  const items = [...state.downloading.values()];
  box.replaceChildren(...(items.length ? [h('div', { class: 'notice' },
    h('span', { class: 'spinner' }),
    `${items.filter((i) => i.state === 'downloading').length} downloading, ${items.filter((i) => i.state === 'queued').length} waiting: `,
    items.slice(0, 3).map((i) => i.title).join(', ') + (items.length > 3 ? '…' : ''))] : []));
}

async function renderDownloads() {
  $('#dl-dir').innerHTML = `Saved to <code>${esc(await call('downloads_dir').catch(() => ''))}</code>. Downloaded songs play from disk, even offline.`;
  renderDownloadActivity();
  const list = $('#dl-list');
  const dl = state.downloads;
  const total = dl.reduce((s, d) => s + d.size, 0);
  $('#dl-meta').textContent = dl.length ? `${dl.length} songs, ${fmtSize(total)}` : '';
  if (!dl.length) {
    $('#dl-head').hidden = true;
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'No downloads yet.'), h('p', { class: 'small' }, 'Use “Download” in a song’s menu, or on an album or playlist.')));
    return;
  }
  tableHead($('#dl-head'));
  const tracks = dl.map((d) => d.track);
  list.replaceChildren(...dl.map((d, i) => trackRow(d.track, i, {
    sub: `${fmtSize(d.size)} · ${PLATFORMS[d.version.platform]?.label || ''}`,
    onPlay: () => playList(tracks, i),
    extraMenu: ['sep', { label: 'Delete file', icon: 'trash', onClick: () => deleteDownload(d.key) }],
  })));
  markPlaying();
}
$('#dl-play').addEventListener('click', () => playList(state.downloads.map((d) => d.track), 0));
$('#dl-folder').addEventListener('click', async () => {
  const dir = await pickFolder('Choose download folder');
  if (dir) { await updateSettings({ downloads_dir: dir }); renderDownloads(); }
});

async function pickFolder(title) {
  try {
    const r = await window.__TAURI__.dialog.open({ directory: true, multiple: false, title });
    return Array.isArray(r) ? r[0] : r;
  } catch (e) {
    toast(`Folder picker unavailable: ${errMsg(e)}`, 'error');
    return null;
  }
}

// Local library ---------------------------------------------------------------
let localTracks = [];
let localLimit = 300;

async function refreshLocalCount() {
  try { localTracks = await call('list_local'); } catch { localTracks = []; }
  state.localCount = localTracks.length;
  renderLibrary();
  $('#source-local').hidden = !localTracks.length;
  if (state.view === 'local') renderLocal();
}

function renderLocal() {
  const folders = state.settings?.local_folders || [];
  $('#local-folders').replaceChildren(...folders.map((f) => h('div', { class: 'folder' },
    h('span', { html: icon('folder') }), h('span', { class: 'path', title: f }, f),
    h('button', { class: 'icon-btn small', title: 'Remove folder', html: icon('x'), onclick: async () => {
      try { await call('remove_local_folder', { path: f }); state.settings = await call('get_settings'); renderLocal(); } catch (e) { toast(errMsg(e), 'error'); }
    } }))));
  const q = $('#local-filter').value.trim().toLowerCase();
  const shown = q ? localTracks.filter((t) => `${t.title} ${t.artist} ${t.versions[0]?.album || ''}`.toLowerCase().includes(q)) : localTracks;
  $('#local-meta').textContent = localTracks.length ? `${localTracks.length} songs in ${folders.length} folder${folders.length === 1 ? '' : 's'}` : '';
  const list = $('#local-list');
  if (!folders.length) {
    $('#local-head').hidden = true;
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Add a folder with music files to play them here and include them in search.'), h('p', { class: 'small' }, 'MP3, M4A/AAC, FLAC, Ogg, Opus and WAV are supported.')));
    return;
  }
  if (!shown.length) {
    $('#local-head').hidden = true;
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, q ? 'No matches.' : 'No audio files found in these folders.')));
    return;
  }
  tableHead($('#local-head'));
  const rows = shown.slice(0, localLimit).map((t, i) => trackRow(t, i, { sub: t.versions[0]?.album || null, onPlay: () => playList(shown, i) }));
  if (shown.length > localLimit) rows.push(h('button', { class: 'btn small show-more', onclick: () => { localLimit += 500; renderLocal(); } }, `Show more (${shown.length - localLimit} left)`));
  list.replaceChildren(...rows);
  markPlaying();
}
$('#local-filter').addEventListener('input', debounce(() => { localLimit = 300; renderLocal(); }, 150));
$('#local-play').addEventListener('click', () => playList(localTracks, 0));
$('#local-add').addEventListener('click', async () => {
  const dir = await pickFolder('Add music folder');
  if (!dir) return;
  toast('Scanning folder…', 'info', 2000);
  try {
    const n = await call('add_local_folder', { path: dir });
    state.settings = await call('get_settings');
    toast(`${n} songs in your local library`, 'ok');
  } catch (e) { toast(errMsg(e), 'error'); }
});
$('#local-rescan').addEventListener('click', async () => {
  try { const n = await call('rescan_local'); toast(`${n} songs found`, 'ok', 2000); } catch (e) { toast(errMsg(e), 'error'); }
});
