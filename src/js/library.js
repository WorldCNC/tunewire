'use strict';

// ===========================================================================
// Your Library (sidebar), playlist pages, Liked Songs
// ===========================================================================

/** What the open playlist page shows: redraw only when this changes. */
const shownSig = (p) => (p ? `${p.name}|${p.description || ''}|${(p.cover || '').length}|${p.pinned}|${p.cloud_id || ''}|${p.cloud_role || ''}|${p.tracks.map((t) => t.key).join(',')}|${Object.keys(p.added || {}).length}` : '');

async function refreshPlaylists({ silent = false } = {}) {
  const before = shownSig(state.playlists.find((p) => p.id === state.selectedPlaylist));
  try { state.playlists = await call('list_playlists'); } catch (e) { toast(errMsg(e), 'error'); }
  renderLibrary();
  warmHeroColors();
  if (silent) return;
  if (state.view === 'playlist' && state.selectedPlaylist && state.selectedPlaylist !== 'remote') {
    const pl = state.playlists.find((p) => p.id === state.selectedPlaylist);
    if (!pl) { state.selectedPlaylist = null; showView('home'); return; }
    // Another playlist changed (a sync, a download…): leave this page alone.
    if (shownSig(pl) === before) return;
    renderPlaylistDetail();
  }
}

function openPlaylist(id, { push = true } = {}) {
  state.selectedPlaylist = id;
  if (state.playlists.find((p) => p.id === id)?.cloud_id) syncSoon(5000);
  showView('playlist', { push, reopen: () => openPlaylist(id, { push: false }) });
  renderPlaylistDetail();
}

/** Square cover: custom image, one cover, or a 2×2 collage. */
function coverArt(tracks, cls, custom) {
  const box = h('div', { class: cls });
  if (custom) { box.append(h('img', { src: custom, alt: '' })); return box; }
  const big = cls.includes('hero-art') ? 500 : cls.includes('lib-art') ? 96 : 160;
  const thumbs = [...new Set(tracks.map((t) => t.thumbnail).filter(Boolean))].slice(0, 4);
  if (!thumbs.length) { box.innerHTML = icon('music'); return box; }
  if (thumbs.length >= 4) box.classList.add('grid4');
  for (const t of thumbs.length >= 4 ? thumbs : thumbs.slice(0, 1)) {
    box.append(h('img', { src: t, 'data-px': thumbs.length >= 4 ? Math.round(big / 2) : big, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => { e.target.style.visibility = 'hidden'; } }));
  }
  return box;
}

// Sidebar ----------------------------------------------------------------------

const LIB_SORTS = { recents: 'Recents', added: 'Recently added', alpha: 'Alphabetical', creator: 'Most songs' };
const libFolders = new Set(JSON.parse((() => { try { return localStorage.getItem('tw-closed-folders') || '[]'; } catch { return '[]'; } })()));

function libItem({ art, name, sub, active, onClick, onMenu, dropPlaylist, playing }) {
  const el = h('button', { class: `lib-item${active ? ' active' : ''}${playing ? ' playing' : ''}`, title: name }, art,
    h('div', { class: 'min-w-0' }, h('div', { class: 'lib-name' }, name), h('div', { class: 'lib-sub' }, ...[].concat(sub))),
    playing ? h('span', { class: 'lib-eq', html: EQ }) : h('span'));
  el.addEventListener('click', onClick);
  if (onMenu) el.addEventListener('contextmenu', (e) => { e.preventDefault(); onMenu(e, el); });
  if (dropPlaylist) {
    el.addEventListener('dragover', (e) => { if (trackDrag) { e.preventDefault(); el.classList.add('drop-target'); } });
    el.addEventListener('dragleave', () => el.classList.remove('drop-target'));
    el.addEventListener('drop', async (e) => {
      if (!trackDrag) return;
      e.preventDefault();
      el.classList.remove('drop-target');
      const tracks = trackDrag.tracks;
      endTrackDrag();
      if (dropPlaylist === 'liked') return likeMany(tracks, true);
      try {
        const n = await call('add_to_playlist', { id: dropPlaylist.id, tracks });
        toast(n ? `Added ${n === 1 ? '' : n + ' songs '}to ${dropPlaylist.name}` : `Already in ${dropPlaylist.name}`, n ? 'ok' : 'info', 2000);
      } catch (er) { toast(errMsg(er), 'error'); }
    });
  }
  return el;
}

function playingFrom() {
  // Which library item the current queue came from (for the green title).
  return state.playingFrom || null;
}

function renderLibrary() {
  const list = $('#lib-list');
  if (!list) return;
  const q = state.libQuery.trim().toLowerCase();
  const f = state.libFilter;
  const items = [];
  const matches = (name) => !q || name.toLowerCase().includes(q);
  for (const b of $$('#lib-chips button')) b.classList.toggle('active', b.dataset.filter === f);
  $('#lib-sort-label').textContent = LIB_SORTS[state.libSort];

  // Liked Songs, Downloads and Local files sit at the top, like pinned items.
  if ((!f || f === 'playlist') && matches('Liked Songs')) {
    items.push(libItem({
      art: h('div', { class: 'lib-art liked', html: icon('heartFill') }),
      name: 'Liked Songs',
      sub: [h('span', { class: 'pin', html: icon('pin') }), `Playlist · ${state.favorites.length} songs`],
      active: state.view === 'favorites', onClick: () => showView('favorites'), dropPlaylist: 'liked', playing: playingFrom() === 'liked',
    }));
  }
  if ((!f || f === 'downloaded') && matches('Downloads') && (state.downloads.length || f === 'downloaded')) {
    items.push(libItem({ art: h('div', { class: 'lib-art special', html: icon('download') }), name: 'Downloads', sub: `${state.downloads.length} songs`, active: state.view === 'downloads', onClick: () => showView('downloads') }));
  }
  if (!f && matches('Local files') && state.localCount) {
    items.push(libItem({ art: h('div', { class: 'lib-art special', html: icon('folder') }), name: 'Local files', sub: `${state.localCount} songs`, active: state.view === 'local', onClick: () => showView('local') }));
  }

  if (!f || f === 'playlist' || f === 'downloaded') {
    let pls = state.playlists.filter((p) => matches(p.name));
    if (f === 'downloaded') pls = pls.filter((p) => p.tracks.length && p.tracks.every((t) => state.downloadKeys.has(t.key)));
    const sorters = {
      recents: (a, b) => (b.lastPlayed || b.updated) - (a.lastPlayed || a.updated),
      added: (a, b) => b.created - a.created,
      alpha: (a, b) => a.name.localeCompare(b.name),
      creator: (a, b) => b.tracks.length - a.tracks.length,
    };
    pls.sort((a, b) => (b.pinned - a.pinned) || sorters[state.libSort](a, b));
    const plItem = (pl) => libItem({
      art: coverArt(pl.tracks, 'lib-art', pl.cover),
      name: pl.name,
      sub: [pl.pinned ? h('span', { class: 'pin', html: icon('pin') }) : null, `Playlist · ${pl.tracks.length} songs`].filter(Boolean),
      active: state.view === 'playlist' && state.selectedPlaylist === pl.id,
      playing: playingFrom() === pl.id,
      onClick: () => openPlaylist(pl.id),
      onMenu: (e, el) => openMenu(el, playlistMenuItems(pl), { at: e }),
      dropPlaylist: pl,
    });
    // Folders (searching shows everything flat).
    const folders = new Map();
    const loose = [];
    for (const pl of pls) {
      if (pl.folder && !q) {
        if (!folders.has(pl.folder)) folders.set(pl.folder, []);
        folders.get(pl.folder).push(pl);
      } else loose.push(pl);
    }
    const pinned = loose.filter((p) => p.pinned);
    items.push(...pinned.map(plItem));
    for (const [name, inside] of [...folders].sort((a, b) => a[0].localeCompare(b[0]))) {
      const closed = libFolders.has(name);
      const head = libItem({
        art: h('div', { class: 'lib-art', html: icon('folder') }),
        name, sub: `Folder · ${inside.length} playlist${inside.length === 1 ? '' : 's'}`,
        onClick: () => {
          if (libFolders.has(name)) libFolders.delete(name); else libFolders.add(name);
          try { localStorage.setItem('tw-closed-folders', JSON.stringify([...libFolders])); } catch { /* optional */ }
          renderLibrary();
        },
        onMenu: (e, el) => openMenu(el, folderMenuItems(name, inside), { at: e }),
      });
      items.push(h('div', { class: `lib-folder${closed ? ' closed' : ''}` }, head, h('div', { class: 'lib-folder-body' }, ...inside.map(plItem))));
    }
    items.push(...loose.filter((p) => !p.pinned).map(plItem));
    if (state.remote && (!f || f === 'playlist') && matches(state.remote.title)) {
      items.push(libItem({
        art: coverArt(state.remote.tracks, 'lib-art'),
        name: state.remote.title, sub: 'Opened link · not saved',
        active: state.view === 'playlist' && state.selectedPlaylist === 'remote',
        onClick: () => openPlaylist('remote'),
      }));
    }
  }
  if (!f || f === 'artist') {
    for (const a of (state.followed || []).filter((x) => matches(x.name))) {
      items.push(libItem({
        art: (() => { const d = h('div', { class: 'lib-art round' }); if (a.thumbnail) d.append(h('img', { src: a.thumbnail, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() })); else d.innerHTML = icon('user'); return d; })(),
        name: a.name, sub: 'Artist',
        active: state.view === 'artist' && state.artistName === a.name,
        onClick: () => openArtist(a.name, null, { browseId: a.url?.match(/channel\/(UC[\w-]+)/)?.[1] }),
      }));
    }
  }
  if (!items.length) {
    items.push(h('div', { class: 'lib-empty' }, q ? `Nothing in Your Library matches “${state.libQuery}”.` :
      [h('b', {}, 'Create your first playlist'), h('div', {}, 'It’s easy, we’ll help you.'), h('button', { class: 'btn', onclick: createPlaylist }, 'Create playlist')]));
  }
  list.replaceChildren(...items);
}

async function createPlaylist({ folder = null, tracks = [] } = {}) {
  const name = await promptModal({ title: 'Create playlist', okLabel: 'Create', placeholder: 'My playlist', value: `My Playlist #${state.playlists.length + 1}` });
  if (!name) return;
  try {
    const pl = await call('create_playlist', { name, tracks });
    if (folder) await call('update_playlist_meta', { id: pl.id, patch: { folder } });
    await refreshPlaylists({ silent: true });
    openPlaylist(pl.id);
  } catch (e) { toast(errMsg(e), 'error'); }
}

function playlistMenuItems(pl) {
  const folders = [...new Set(state.playlists.map((p) => p.folder).filter(Boolean))].sort();
  return [
    { label: 'Play', icon: 'play', onClick: () => playCollection(pl.tracks, 0, pl.id) },
    { label: 'Add to queue', icon: 'plus', onClick: () => enqueue(pl.tracks) },
    'sep',
    { label: 'Edit details', icon: 'edit', onClick: () => editPlaylistDetails(pl) },
    { label: pl.pinned ? 'Unpin playlist' : 'Pin playlist', icon: 'pin', onClick: () => call('update_playlist_meta', { id: pl.id, patch: { pinned: !pl.pinned } }) },
    { label: 'Move to folder', heading: true },
    ...folders.filter((f) => f !== pl.folder).map((f) => ({ label: f, icon: 'folder', onClick: () => call('update_playlist_meta', { id: pl.id, patch: { folder: f } }) })),
    { label: 'New folder…', icon: 'folderPlus', onClick: async () => {
      const f = await promptModal({ title: 'New folder', okLabel: 'Create', placeholder: 'Folder name' });
      if (f) call('update_playlist_meta', { id: pl.id, patch: { folder: f } });
    } },
    pl.folder ? { label: 'Remove from folder', icon: 'x', onClick: () => call('update_playlist_meta', { id: pl.id, patch: { folder: '' } }) } : null,
    'sep',
    pl.source_url ? { label: 'Sync with source', icon: 'refresh', onClick: () => syncPlaylist(pl.id) } : null,
    { label: 'Download', icon: 'download', onClick: () => downloadTracks(pl.tracks) },
    ...shareMenuItems(pl),
    'sep',
    { label: 'Delete', icon: 'trash', onClick: () => deletePlaylist(pl) },
  ].filter(Boolean);
}

function folderMenuItems(name, inside) {
  return [
    { label: 'Create playlist in folder', icon: 'plus', onClick: () => createPlaylist({ folder: name }) },
    { label: 'Rename folder', icon: 'edit', onClick: async () => {
      const n = await promptModal({ title: 'Rename folder', value: name, okLabel: 'Rename' });
      if (n && n !== name) for (const p of inside) await call('update_playlist_meta', { id: p.id, patch: { folder: n } });
    } },
    { label: 'Delete folder (keeps playlists)', icon: 'trash', onClick: async () => { for (const p of inside) await call('update_playlist_meta', { id: p.id, patch: { folder: '' } }); } },
  ];
}

/** Deleted at once, with Undo (the whole playlist is kept until then). */
async function deletePlaylist(pl) {
  const snapshot = state.playlists.find((p) => p.id === pl.id) || pl;
  const index = state.playlists.indexOf(snapshot);
  try {
    await call('delete_playlist', { id: pl.id });
    if (state.selectedPlaylist === pl.id) { state.selectedPlaylist = null; if (state.view === 'playlist') goBack(); }
    undoToast(`Deleted “${pl.name}”`, () => call('restore_playlist', { playlist: snapshot, index: index < 0 ? null : index }));
  } catch (e) { toast(errMsg(e), 'error'); }
}

/** Remove songs (by index) with Undo: the old list is put back as it was. */
async function removeWithUndo(pl, indices) {
  const live = state.playlists.find((p) => p.id === pl.id) || pl;
  const before = live.tracks.slice();
  const added = { ...(live.added || {}) };
  try {
    const n = indices.length === 1
      ? (await call('remove_from_playlist', { id: pl.id, index: indices[0] }), 1)
      : await call('remove_many_from_playlist', { id: pl.id, indices });
    clearSelection();
    undoToast(`Removed ${n === 1 ? `“${before[indices[0]]?.title || 'song'}”` : `${n} songs`} from ${pl.name}`,
      () => call('replace_playlist_tracks', { id: pl.id, tracks: before, added }));
  } catch (e) { toast(errMsg(e), 'error'); }
}

/** Resize a picked image to a 300×300 JPEG data: URL. */
function imageToCover(file) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement('canvas');
      c.width = c.height = 300;
      const g = c.getContext('2d');
      const s = Math.min(img.width, img.height);
      g.drawImage(img, (img.width - s) / 2, (img.height - s) / 2, s, s, 0, 0, 300, 300);
      URL.revokeObjectURL(img.src);
      resolve(c.toDataURL('image/jpeg', 0.86));
    };
    img.onerror = () => reject(new Error('That file isn’t an image Tunewire can read.'));
    img.src = URL.createObjectURL(file);
  });
}

function editPlaylistDetails(pl) {
  let cover = pl.cover || null;
  const file = h('input', { type: 'file', accept: 'image/*', hidden: true });
  const art = coverArt(pl.tracks, 'hero-art editable', cover);
  art.style.cssText = 'width:180px;height:180px';
  const name = h('input', { class: 'input', value: pl.name, placeholder: 'Add a name', maxlength: 120 });
  const desc = h('textarea', { class: 'input font-inherit fs-135', rows: 5, placeholder: 'Add an optional description', maxlength: 600}, pl.description || '');
  art.addEventListener('click', () => file.click());
  file.addEventListener('change', async () => {
    if (!file.files[0]) return;
    try {
      cover = await imageToCover(file.files[0]);
      art.replaceChildren(h('img', { src: cover, alt: '' }));
    } catch (e) { toast(e.message, 'error'); }
  });
  const removeCover = h('button', { class: 'btn small', onclick: () => { cover = ''; art.replaceWith(Object.assign(coverArt(pl.tracks, 'hero-art editable', null), { onclick: () => file.click() })); } }, 'Remove photo');
  const m = openModal({
    title: 'Edit details',
    wide: true,
    body: h('div', { class: 'edit-grid' },
      h('div', { class: 'col gap-8 items-start' }, art, pl.cover ? removeCover : null, file),
      h('div', { class: 'col gap-10' }, name, desc)),
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), h('button', { class: 'btn primary', onclick: async () => {
      try {
        await call('update_playlist_meta', { id: pl.id, patch: { name: name.value, description: desc.value, ...(cover !== (pl.cover || null) ? { cover: cover || '' } : {}) } });
        m.close();
      } catch (e) { toast(errMsg(e), 'error'); }
    } }, 'Save')],
  });
}

// Collection pages (playlist, album, liked, mix) -----------------------------------

/** Pick a header color from the cover (falls back to a neutral gray). */
const heroColors = new Map();
/** Work out header colors for Your Library's covers ahead of time, so a
 * playlist opens in its color instead of grey-then-color. */
let heroWarm = null;
function warmHeroColors() {
  clearTimeout(heroWarm);
  heroWarm = setTimeout(async () => {
    for (const p of state.playlists.slice(0, 60)) {
      const thumb = p.cover || p.tracks.find((t) => t.thumbnail)?.thumbnail;
      if (!thumb || heroColors.has(thumb)) continue;
      try { const c = await call('cover_color', { url: imgUrl(thumb, 120) }); if (c) heroColors.set(thumb, c); } catch { /* later */ }
    }
  }, 1500);
}

async function heroColor(el, thumb, seqCheck) {
  // Known color: set it right away (re-renders must not flash gray).
  if (heroColors.has(thumb)) { el.style.setProperty('--hero', heroColors.get(thumb)); return; }
  if (el.dataset.heroFor !== (thumb || '')) el.style.setProperty('--hero', '#4a4a4a');
  el.dataset.heroFor = thumb || '';
  if (!thumb) return;
  try {
    const c = await call('cover_color', { url: imgUrl(thumb, 120) });
    if (c) heroColors.set(thumb, c);
    if (c && (!seqCheck || seqCheck())) el.style.setProperty('--hero', c);
  } catch { /* keep gray */ }
}

/** Shows the compact header once the big one scrolls away. */
function stickyHeader(view, hero, title, onPlay) {
  const bar = h('div', { class: 'sticky-head' },
    h('button', { class: 'big-play small', title: 'Play', html: icon('play'), onclick: onPlay }),
    h('span', { class: 'st-title' }, title));
  const io = new IntersectionObserver(([en]) => bar.classList.toggle('show', !en.isIntersecting && en.boundingClientRect.top < 0), { root: view, threshold: 0 });
  setTimeout(() => io.observe(hero), 0);
  return bar;
}

/** Play a list and remember where it came from (Your Library highlights it). */
function playCollection(tracks, start, from) {
  state.playingFrom = from || null;
  const pl = state.playlists.find((p) => p.id === from);
  if (pl) pl.lastPlayed = Date.now();
  renderLibrary();
  return playList(tracks, start);
}

const pageState = new Map(); // per page: { query, sort }

/**
 * A playlist-style page.
 * o: { id, kind, title, tracks, cover, art, line, description, editable, added,
 *      actions(bar), menu(items), removeMany, reorder, noCover, noAlbum, after }
 */
function renderCollection(container, o) {
  const view = container.closest('.view');
  const ps = pageState.get(o.id) || { query: '', sort: null };
  // Animate rows only when the page is opened, not when it re-renders.
  const firstShow = container._shownId !== o.id;
  container._shownId = o.id;
  pageState.set(o.id, ps);
  const tracks = o.tracks;
  const total = tracks.reduce((s, t) => s + (t.duration || 0), 0);
  const dur = total >= 3600 ? `${Math.floor(total / 3600)} hr ${Math.round((total % 3600) / 60)} min` : fmtTime(total).replace(/^(\d+):(\d+)$/, '$1 min $2 sec');

  const art = o.art || coverArt(tracks, 'hero-art', o.cover);
  if (o.editable) { art.classList.add('editable'); art.addEventListener('click', o.editable); }
  const title = h('h1', { class: `hero-title${o.title.length > 28 ? ' long' : ''}${o.editable ? ' editable' : ''}`, onclick: o.editable || null }, o.title);
  const hero = h('div', { class: 'hero' }, art,
    h('div', { class: 'hero-meta' },
      h('div', { class: 'hero-kicker' }, o.kind),
      title,
      o.description ? h('div', { class: 'hero-desc' }, o.description) : null,
      h('div', { class: 'hero-line' }, ...(o.line || []), tracks.length ? h('span', { class: 'dot' }, `${tracks.length} song${tracks.length === 1 ? '' : 's'}${total ? `, ${dur}` : ''}`) : null)));

  const play = () => {
    if (o.smart?.on && o.smart.recs?.length) return playCollection(smartMix(tracks, o.smart.recs), 0, o.id);
    return playCollection(tracks, 0, o.id);
  };
  const shuffleBtn = h('button', { class: `ab-icon${state.settings?.shuffle ? ' on' : ''}`, title: state.settings?.shuffle ? 'Turn off shuffle' : 'Shuffle', html: icon('shuffle'), onclick: async () => {
    await call('set_shuffle', { on: !state.settings?.shuffle });
    state.settings.shuffle = !state.settings.shuffle;
    shuffleBtn.classList.toggle('on', state.settings.shuffle);
    if (state.settings.shuffle && !isPlayingFrom(o.id)) playCollection(tracks, Math.floor(Math.random() * tracks.length), o.id);
  } });
  const findInput = h('input', { placeholder: 'Search in playlist', value: ps.query, spellcheck: 'false' });
  const find = h('div', { class: `ab-find${ps.query ? ' open' : ''}` }, h('button', { class: 'icon-btn', title: 'Search in playlist', html: icon('search'), onclick: () => { find.classList.add('open'); findInput.focus(); } }), findInput);
  findInput.addEventListener('input', () => { ps.query = findInput.value; renderRows(); });
  findInput.addEventListener('blur', () => { if (!findInput.value) find.classList.remove('open'); });
  const sorts = [['', 'Custom order'], ['title', 'Title'], ['artist', 'Artist'], ['album', 'Album'], ...(o.added ? [['added', 'Date added']] : []), ['duration', 'Duration']];
  const sortBtn = h('button', { class: 'ab-sort', onclick: () => openMenu(sortBtn, [{ label: 'Sort by', heading: true }, ...sorts.map(([k, l]) => ({ label: `${(ps.sort?.key || '') === k ? '✓ ' : ''}${l}`, onClick: () => { ps.sort = k ? { key: k, dir: k === 'added' ? -1 : 1 } : null; renderRows(); sortBtn.firstChild.textContent = l; } }))]) },
    h('span', {}, sorts.find(([k]) => k === (ps.sort?.key || ''))?.[1] || 'Custom order'), h('span', { html: icon('sort') }));

  const bar = h('div', { class: 'action-bar' },
    h('button', { class: 'big-play', title: `Play ${o.title}`, html: icon('play'), onclick: play }),
    tracks.length > 1 ? shuffleBtn : null,
    ...(o.actions || []),
    o.menu ? (() => { const b = h('button', { class: 'ab-icon', title: `More options for ${o.title}`, html: icon('dots') }); b.addEventListener('click', () => openMenu(b, o.menu())); return b; })() : null,
    h('span', { class: 'ab-spacer' }),
    tracks.length > 1 ? find : null,
    tracks.length > 1 ? sortBtn : null);

  const head = h('div', { class: 'table-head' });
  const list = h('div', { class: `track-list${firstShow ? ' stagger' : ''}`, role: 'listbox', 'aria-label': 'Songs', 'aria-multiselectable': 'true' });
  const renderRows = () => {
    let idx = tracks.map((t, i) => [t, i]);
    const q = ps.query.trim().toLowerCase();
    if (q) idx = idx.filter(([t]) => `${t.title} ${t.artist} ${trackAlbum(t)}`.toLowerCase().includes(q));
    const s = ps.sort;
    if (s) {
      const get = (t) => (s.key === 'duration' ? t.duration || 0 : s.key === 'added' ? o.added?.[t.key] || 0 : s.key === 'album' ? trackAlbum(t).toLowerCase() : (t[s.key] || '').toLowerCase());
      idx.sort(([a], [b]) => (get(a) > get(b) ? 1 : get(a) < get(b) ? -1 : 0) * s.dir);
    }
    tableHead(head, { added: !!o.added, albumLabel: o.noAlbum ? '' : 'Album', sort: s, onSort: (key) => { ps.sort = ps.sort?.key === key ? (ps.sort.dir === 1 ? { key, dir: -1 } : null) : { key, dir: 1 }; renderRows(); } });
    const shown = idx.map(([t]) => t);
    const row = ([t, i], n) => trackRow(t, n, {
      index: i,
      onPlay: () => playCollection(shown, n, o.id),
      added: o.added ? (o.added[t.key] || o.created || 0) : undefined,
      noCover: o.noCover,
      noAlbum: o.noAlbum,
      extraMenu: o.rowMenu ? o.rowMenu(t, i) : [],
    });
    renderChunked(list, idx, row, view);
    // Reordering only makes sense in the playlist's own order.
    list._ctx = { removeMany: o.removeMany || null, removeLabel: o.removeLabel, reorder: !q && !s ? o.reorder || null : null };
    if (!idx.length && tracks.length) list.append(h('div', { class: 'empty' }, h('p', {}, `Nothing matches “${ps.query}”.`)));
    markPlaying();
  };
  renderRows();
  setTimeout(() => list.classList.remove('stagger'), 700);

  const children = [stickyHeader(view, hero, o.title, play), hero, bar];
  if (o.status) children.push(o.status);
  if (tracks.length) children.push(head, list);
  else children.push(o.empty || h('div', { class: 'empty' }, h('p', {}, 'Nothing here yet.')));
  if (o.after) children.push(o.after);
  container.replaceChildren(...children);
  // The likely first click: resolve its stream now.
  if (firstShow && tracks[0] && !state.settings?.shuffle) prefetchTrack(tracks[0]);
  heroColor(view.querySelector('.hero')?.parentElement || container, o.cover || tracks.find((t) => t.thumbnail)?.thumbnail);
  return { list, renderRows };
}

/**
 * Rows for `items`, built as they come near the screen: the first screenful
 * at once, the rest in small batches spread over animation frames, so
 * scrolling never waits for a big batch to be built (that was the stutter
 * when the end of a long playlist came into view).
 */
const FIRST_ROWS = 40;
const BATCH_ROWS = 8;
function renderChunked(list, items, makeRow, root) {
  list._chunkIo?.disconnect();
  cancelAnimationFrame(list._chunkRaf || 0);
  let n = Math.min(FIRST_ROWS, items.length);
  list.replaceChildren(...items.slice(0, n).map((it, k) => makeRow(it, k)));
  if (n >= items.length) return;
  const sentinel = h('div', { class: 'chunk-sentinel', 'aria-hidden': 'true' });
  list.append(sentinel);
  let near = false;
  const step = () => {
    list._chunkRaf = 0;
    if (!near || !sentinel.isConnected) return;
    const t0 = performance.now();
    const frag = document.createDocumentFragment();
    // About 4 ms of work per frame, at least one batch.
    do {
      const next = items.slice(n, n + BATCH_ROWS).map((it, k) => makeRow(it, n + k));
      n += next.length;
      frag.append(...next);
    } while (n < items.length && performance.now() - t0 < 3);
    sentinel.before(frag);
    markPlaying();
    if (n >= items.length) { io.disconnect(); sentinel.remove(); return; }
    list._chunkRaf = requestAnimationFrame(step);
  };
  const io = new IntersectionObserver((entries) => {
    near = entries.some((e) => e.isIntersecting);
    if (near && !list._chunkRaf) list._chunkRaf = requestAnimationFrame(step);
  }, { root, rootMargin: '1600px 0px' });
  io.observe(sentinel);
  list._chunkIo = io;
}

function isPlayingFrom(id) {
  return state.playingFrom === id && ['playing', 'paused', 'loading'].includes(state.playback?.status);
}

/** Smart shuffle: the playlist shuffled, with one recommendation after every three songs. */
function smartMix(tracks, recs) {
  const base = [...tracks].sort(() => Math.random() - 0.5);
  const out = [];
  let r = 0;
  base.forEach((t, i) => {
    out.push(t);
    if (i % 3 === 2 && r < recs.length) out.push(recs[r++].track);
  });
  return out;
}

function renderPlaylistDetail() {
  const box = $('#playlist-detail');
  const id = state.selectedPlaylist;
  if (!id) return;
  const remote = id === 'remote';
  const pl = remote ? state.remote : state.playlists.find((p) => p.id === id);
  if (!pl) return;
  const tracks = pl.tracks;
  const rs = recState.get(remote ? 'remote' : pl.id);
  const smart = { on: !!smartOn.get(id), recs: rs?.list || null };

  const actions = [];
  if (!remote) {
    const smartBtn = h('button', { class: `ab-icon${smart.on ? ' on' : ''}`, title: smart.on ? 'Turn off smart shuffle' : 'Smart shuffle: mixes in recommended songs', html: icon('smartShuffle'), onclick: () => {
      smartOn.set(id, !smart.on);
      if (!smart.on) toast('Smart shuffle: recommended songs will be mixed in when you press play', 'info', 3000);
      renderPlaylistDetail();
    } });
    if (tracks.length > 2) actions.push(smartBtn);
    actions.push(downloadRing(tracks));
  } else if (pl.cloud) {
    // A playlist someone shared: follow it (keeps updating) or take a copy.
    actions.push(h('button', { class: 'ab-follow', onclick: () => followShared(pl) }, 'Follow'),
      h('button', { class: 'ab-icon', title: 'Save a copy to Your Library', html: icon('plus'), onclick: async () => {
        try {
          const created = await call('create_playlist', { name: pl.title, tracks, sourceUrl: null });
          toast(`Saved a copy of “${pl.title}”`, 'ok');
          await refreshPlaylists({ silent: true });
          openPlaylist(created.id, { push: false });
        } catch (e) { toast(errMsg(e), 'error'); }
      } }));
  } else {
    actions.push(h('button', { class: 'ab-follow', onclick: async () => {
      try {
        const created = await call('create_playlist', { name: pl.title, tracks, sourceUrl: pl.url });
        toast(`Saved “${pl.title}” to Your Library`, 'ok');
        state.remote = null;
        await refreshPlaylists({ silent: true });
        openPlaylist(created.id, { push: false });
      } catch (e) { toast(errMsg(e), 'error'); }
    } }, 'Save to Your Library'));
  }

  const cm = !remote && pl.cloud_id ? cloudMeta.get(pl.cloud_id) : null;
  const kind = remote ? (pl.cloud ? 'Shared playlist' : pl.platform ? `${PLATFORMS[pl.platform]?.label || ''} playlist` : 'Playlist')
    : cm?.collaborative || pl.cloud_role === 'editor' ? 'Collaborative playlist' : pl.cloud_role === 'follower' ? 'Followed playlist' : pl.cloud_id ? 'Shared playlist' : 'Playlist';
  if (!remote && cloudOn() && pl.cloud_role !== 'follower') actions.push(h('button', { class: 'ab-icon', title: pl.cloud_id ? 'Copy link' : 'Share', html: icon('share'), onclick: () => sharePlaylist(pl) }));
  renderCollection(box, {
    id,
    kind,
    title: remote ? pl.title : pl.name,
    tracks,
    cover: remote ? pl.cloud?.cover || null : pl.cover,
    description: remote ? pl.cloud?.description || '' : pl.description,
    line: [remote && pl.cloud ? h('button', { onclick: () => openProfile(pl.cloud.owner) }, pl.cloud.ownerName || 'Someone') : h('b', {}, remote ? 'Opened link' : pl.cloud_role === 'follower' || pl.cloud_role === 'editor' ? (cm?.ownerName || 'Shared with you') : 'You'),
      remote ? null : syncBadge(pl)].filter(Boolean),
    added: remote ? null : pl.added || {},
    created: pl.created,
    editable: remote ? null : () => editPlaylistDetails(pl),
    smart,
    actions,
    menu: remote ? () => [{ label: 'Add to queue', icon: 'plus', onClick: () => enqueue(tracks) }] : () => playlistMenuItems(pl),
    removeMany: remote ? null : (indices) => removeWithUndo(pl, indices),
    reorder: remote ? null : (indices, to) => call('move_many_in_playlist', { id: pl.id, indices, to }).catch((e) => toast(errMsg(e), 'error')),
    rowMenu: remote ? null : (t, i) => ['sep', { label: 'Remove from this playlist', icon: 'trash', onClick: () => removeWithUndo(pl, [i]) }],
    status: matchStatus(tracks),
    empty: h('div', { class: 'empty' }, h('p', {}, 'Let’s find something for your playlist'), h('p', { class: 'small' }, 'Search for songs, then drag them here or use ⋯ → Add to playlist.'),
      h('button', { class: 'btn mt-12', onclick: () => { showView('search'); $('#search-input').focus(); } }, 'Find songs')),
    after: recommendSection(pl, remote),
  });
}
const smartOn = new Map();

// Liked Songs ---------------------------------------------------------------------

async function refreshFavorites() {
  try { state.favorites = await call('list_favorites'); } catch (e) { toast(errMsg(e), 'error'); }
  state.favKeys = new Set(state.favorites.map((t) => t.key));
  if (state.view === 'favorites') renderFavorites();
  renderLibrary();
  // Refresh heart states elsewhere.
  for (const row of $$('.track[data-key]')) {
    const btn = row.querySelector('.like-btn');
    if (!btn) continue;
    const liked = state.favKeys.has(row.dataset.key);
    btn.classList.toggle('liked', liked);
    btn.title = liked ? 'Remove from Liked Songs' : 'Save to Liked Songs';
    btn.innerHTML = icon(liked ? 'heartFill' : 'heart');
  }
  renderPlayback();
  markPlaying();
}

function renderFavorites() {
  const box = $('#fav-body');
  renderCollection(box, {
    id: 'liked',
    kind: 'Playlist',
    title: 'Liked Songs',
    tracks: state.favorites,
    art: h('div', { class: 'hero-art liked', html: icon('heartFill') }),
    line: [h('b', {}, 'You')],
    actions: [downloadRing(state.favorites)],
    removeMany: async (indices) => { await likeMany(indices.map((i) => state.favorites[i]), false); clearSelection(); },
    removeLabel: 'Remove from Liked Songs',
    status: matchStatus(state.favorites),
    empty: h('div', { class: 'empty' }, h('p', {}, 'Songs you like will appear here'), h('p', { class: 'small' }, 'Save songs by tapping the heart icon.')),
    after: likedRecs(),
  });
  // The liked gradient is the header color here.
  box.style.setProperty('--hero', 'color-mix(in srgb, var(--accent) 55%, #2a1a6a)');
}

function likedRecs() {
  if (state.favorites.length < 3) return null;
  return recommendSection({ id: 'liked', name: 'Liked Songs', tracks: state.favorites.slice(0, 200) }, true);
}

// Spotify matching -------------------------------------------------------------
// Spotify songs are matched to their YouTube Music recording in the background
// right after import, so they start as fast as any other song.
state.matchProgress = null;

function spotifyOnly(t) {
  return t.versions.length > 0 && t.versions.every((v) => v.platform === 'spotify');
}

function matchStatus(tracks) {
  const el = h('div', { class: 'match-status', id: 'match-status' });
  el._pending = tracks.filter(spotifyOnly).length;
  paintMatchStatus(el);
  return el;
}

function paintMatchStatus(el = $('#match-status')) {
  if (!el) return;
  const p = state.matchProgress;
  if (p && !p.finished && el._pending > 0) {
    el.hidden = false;
    el.replaceChildren(h('span', { class: 'spinner' }), h('span', {}, `Getting Spotify songs ready to play instantly · ${p.done} of ${p.total}`),
      h('div', { class: 'match-bar' }, h('i', { style: `width:${(p.done / Math.max(1, p.total)) * 100}%` })));
  } else {
    el.hidden = true;
  }
}

listen('match-progress', async (e) => {
  const p = e.payload;
  const wasRunning = state.matchProgress && !state.matchProgress.finished;
  state.matchProgress = p;
  paintMatchStatus();
  if (p.finished && p.scope === 'other' && state.remote) {
    // An opened Spotify link: swap in the matched tracks quietly.
    state.remote.tracks = await call('apply_matches', { tracks: state.remote.tracks }).catch(() => state.remote.tracks);
  }
  if (p.finished && wasRunning && p.scope === 'library' && p.total >= 10) {
    toast(`${p.matched} Spotify songs are ready to play instantly${p.matched < p.total ? ` (${p.total - p.matched} not found on YouTube Music)` : ''}`, 'ok', 5000);
  }
});

// Matches arriving don't change what rows show: update the data, keep the page.
listen('library-matched', async (e) => {
  if (e.payload === 'favorites') {
    state.favorites = await call('list_favorites').catch(() => state.favorites);
  } else {
    await refreshPlaylists({ silent: true });
  }
});

// Recommendations ------------------------------------------------------------
// Kept per playlist so re-rendering (likes, adds) doesn't refetch.
const recState = new Map();

function recommendSection(pl, remote) {
  const id = pl.id || 'remote';
  const tracks = pl.tracks;
  const box = h('div', { class: 'rec-section' });
  if (!tracks.length) return box;
  let rs = recState.get(id);
  if (!rs || rs.src !== tracks.length) {
    rs = { nonce: rs?.nonce || 1, list: null, loading: false, error: null, src: tracks.length, added: new Set(rs?.added || []) };
    if (recState.size > 20) recState.delete(recState.keys().next().value);
    recState.set(id, rs);
  }
  const refresh = () => { rs.nonce++; rs.list = null; load(); };
  const render = () => {
    const head = h('div', { class: 'rec-head' },
      h('div', {}, h('h2', {}, 'Recommended'), h('span', { class: 'meta' }, remote ? 'Songs that fit this playlist' : 'Based on what’s in this playlist')),
      h('div', { class: 'actions' },
        rs.list?.length ? h('button', { class: 'btn small', html: `${icon('play')}Play all`, onclick: () => playList(rs.list.map((r) => r.track), 0) }) : null,
        h('button', { class: 'btn small', html: `${icon('refresh')}Refresh`, disabled: rs.loading, onclick: refresh })));
    if (rs.loading && !rs.list) {
      box.replaceChildren(head, h('div', { class: 'track-list', role: 'listbox', 'aria-label': 'Songs' }, ...skeletonRows(5)));
      return;
    }
    if (rs.error) {
      box.replaceChildren(head, h('div', { class: 'empty' }, h('p', {}, rs.error)));
      return;
    }
    const visible = (rs.list || []).filter((r) => !rs.added.has(r.track.key));
    const list = h('div', { class: 'track-list stagger', role: 'listbox', 'aria-label': 'Songs' }, ...visible.map((r, i) => {
      const add = remote ? null : h('button', { class: 'icon-btn small rec-add', title: `Add to ${pl.name}`, html: icon('plus'), onclick: async (e) => {
        e.stopPropagation();
        // The playlist grows by one: keep this list instead of refetching
        // (set before the call; the re-render can arrive before it returns).
        rs.added.add(r.track.key);
        rs.src++;
        try {
          await call('add_to_playlist', { id: pl.id, tracks: [r.track] });
          toast(`Added to ${pl.name}`, 'ok', 1400);
        } catch (err) {
          rs.added.delete(r.track.key);
          rs.src--;
          toast(errMsg(err), 'error');
          render();
        }
      } });
      return trackRow(r.track, i, { onPlay: () => playTrack(r.track), sub: `Because of ${r.because}`, action: add });
    }));
    box.replaceChildren(head, list);
    setTimeout(() => list.classList.remove('stagger'), 700);
    markPlaying();
  };
  const load = async () => {
    rs.loading = true;
    rs.error = null;
    render();
    try {
      rs.list = await call('recommend_for_playlist', { tracks, nonce: rs.nonce, count: 20 });
    } catch (e) {
      rs.error = errMsg(e);
    }
    rs.loading = false;
    if (box.isConnected) render();
  };
  if (rs.list) render();
  else load();
  return box;
}

// History ----------------------------------------------------------------------

async function refreshHistory() {
  try { state.history = await call('list_history', { limit: 300 }); } catch (e) { toast(errMsg(e), 'error'); }
  const list = $('#history-list');
  if (!state.history.length) {
    $('#history-head').hidden = true;
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Nothing played yet.')));
    return;
  }
  tableHead($('#history-head'));
  list.replaceChildren(...state.history.map((hEntry, i) => {
    const row = trackRow(hEntry.track, i, {
      sub: `${timeAgo(hEntry.played_at)}${hEntry.platform ? ' from ' + PLATFORMS[hEntry.platform]?.label : ''}`,
      onPlay: () => playTrack(hEntry.track, hEntry.platform),
    });
    return row;
  }));
  markPlaying();
}
$('#history-clear').addEventListener('click', async () => {
  try {
    const entries = await call('list_history', { limit: 5000 });
    if (!entries.length) return;
    await call('clear_history');
    undoToast(`Cleared ${entries.length} song${entries.length === 1 ? '' : 's'} from history`, () => call('restore_history', { entries }));
  } catch (e) { toast(errMsg(e), 'error'); }
});
