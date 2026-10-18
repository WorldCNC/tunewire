'use strict';

// ===========================================================================
// Track rows & actions
// ===========================================================================

/** Default version index for a track according to the user's platform order. */
function sortedVersions(track) {
  const order = state.settings?.platform_order || ['ytmusic', 'soundcloud', 'youtube', 'spotify'];
  // Uploads standing in for another version (Spotify matches) aren't sources of their own.
  return track.versions.filter((v) => !v.matched_for).sort((a, b) =>
    (a.preview - b.preview) || (order.indexOf(a.platform) - order.indexOf(b.platform)));
}

function playingKey() {
  return state.playback?.current?.track?.key;
}

async function playTrack(track, platform = null) {
  state.loadingKey = track.key;
  markPlaying();
  try {
    await call('play_track', { track, platform });
  } catch (e) {
    toast(errMsg(e), 'error');
  }
}

async function playList(tracks, start = 0) {
  if (!tracks.length) return;
  try {
    await call('play_tracks', { tracks, start });
  } catch (e) {
    toast(errMsg(e), 'error');
  }
}

async function enqueue(tracks, next = false) {
  try {
    await call('add_to_queue', { tracks, next });
    toast(tracks.length === 1 ? `${next ? 'Playing next' : 'Added to queue'}: ${tracks[0].title}` : `Added ${tracks.length} tracks to queue`, 'ok', 2200);
  } catch (e) {
    toast(errMsg(e), 'error');
  }
}

async function toggleFavorite(track) {
  try {
    const liked = await callInOrder(`fav:${track.key}`, 'toggle_favorite', { track });
    toast(liked ? `Liked “${track.title}”` : `Removed “${track.title}” from favorites`, 'ok', 2000);
  } catch (e) {
    toast(errMsg(e), 'error');
  }
}

function addToPlaylistMenu(anchor, tracks) {
  const items = [{ label: 'Add to playlist', heading: true }];
  for (const pl of state.playlists) {
    items.push({
      label: pl.name, icon: 'list',
      onClick: async () => {
        try {
          const n = await call('add_to_playlist', { id: pl.id, tracks });
          toast(n ? `Added ${n === 1 ? '' : n + ' tracks '}to ${pl.name}` : `Already in ${pl.name}`, n ? 'ok' : 'info', 2200);
        } catch (e) { toast(errMsg(e), 'error'); }
      },
    });
  }
  if (state.playlists.length) items.push('sep');
  items.push({
    label: 'New playlist…', icon: 'plus',
    onClick: async () => {
      const name = await promptModal({ title: 'New playlist', value: tracks.length === 1 ? tracks[0].title : '', okLabel: 'Create' });
      if (!name) return;
      try {
        await call('create_playlist', { name, tracks });
        toast(`Created ${name}`, 'ok', 2000);
      } catch (e) { toast(errMsg(e), 'error'); }
    },
  });
  openMenu(anchor, items);
}

/**
 * One row of a track table.
 * opts.onPlay: custom play action (e.g. play a whole list from this index)
 * opts.extraMenu: extra menu items
 * opts.sub: secondary line under the title (e.g. "played 3 min ago")
 */
const prefetched = new Map();
function prefetchTrack(track) {
  const now = Date.now();
  if (now - (prefetched.get(track.key) || 0) < 10 * 60 * 1000) return;
  prefetched.set(track.key, now);
  // Skipped (the backend was busy): the next hover may ask again.
  call('prefetch_track', { track, platform: null })
    .then((ran) => { if (ran === false) prefetched.delete(track.key); })
    .catch(() => prefetched.delete(track.key));
}

function trackAlbum(track) {
  return track.versions.find((v) => v.album)?.album || '';
}

function fmtAdded(ms) {
  if (!ms) return '';
  const d = (Date.now() - ms) / 1000;
  if (d < 3600) return `${Math.max(1, Math.round(d / 60))} min ago`;
  if (d < 86400) return `${Math.round(d / 3600)} hour${Math.round(d / 3600) === 1 ? '' : 's'} ago`;
  if (d < 86400 * 7) return `${Math.round(d / 86400)} day${Math.round(d / 86400) === 1 ? '' : 's'} ago`;
  if (d < 86400 * 28) return `${Math.round(d / (86400 * 7))} week${Math.round(d / (86400 * 7)) === 1 ? '' : 's'} ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

/**
 * One row of a track table.
 * opts.onPlay: custom play action (e.g. play a whole list from this index)
 * opts.extraMenu: extra menu items
 * opts.sub: extra note after the artist (e.g. "Because of …", "3 min ago")
 * opts.added: when it was added (shows the Date added column)
 * opts.action: an extra button next to the heart
 * opts.noCover / opts.noAlbum: album pages
 */
function trackRow(track, index, opts = {}) {
  const versions = sortedVersions(track);
  const cls = ['track'];
  if (opts.added !== undefined) cls.push('has-added');
  if (opts.noCover) cls.push('no-cover');
  // No listeners per row: one set of delegated listeners (below) serves
  // every row, so long lists build fast and hold little memory.
  const row = h('div', { class: cls.join(' '), dataset: { key: track.key }, style: `--i:${index}`, draggable: 'true', role: 'option', 'aria-selected': 'false', 'aria-label': `${track.title}${track.artist ? `, ${track.artist}` : ''}` });
  row._track = track;
  row._index = opts.index ?? index;
  row._opts = opts;
  // The first row is where Tab enters a list (arrows move from there).
  row.tabIndex = index === 0 ? 0 : -1;
  const liked = state.favKeys.has(track.key);

  const num = h('div', { class: 't-num' },
    h('span', { class: 'n' }, String(index + 1)),
    h('button', { class: 'icon-btn', 'data-act': 'play', title: `Play ${track.title}`, 'aria-label': `Play ${track.title}`, html: icon('play') }));
  num.insertAdjacentHTML('beforeend', EQ);

  // Source icons: shown on hover (or always, with the setting on).
  const srcs = h('span', { class: 't-srcs' });
  for (const v of versions) {
    const d = pdot(v.platform, { preview: v.preview, title: `Play from ${PLATFORMS[v.platform]?.label}${v.preview ? ' (30 s preview)' : ''}` });
    d.dataset.act = 'src';
    d.dataset.platform = v.platform;
    srcs.append(d);
  }

  const artistsEl = h('span', { class: 't-artists' });
  if (track.artist) {
    track.artist.split(/, | & /).forEach((n, i) => {
      if (i) artistsEl.append(', ');
      artistsEl.append(h('button', { 'data-act': 'artist', 'data-i': i }, n));
    });
  } else {
    artistsEl.append(h('span', { class: 'faint' }, 'Unknown artist'));
  }
  const album = trackAlbum(track);
  const albumEl = h('div', { class: 't-album' }, album && !opts.noAlbum
    ? (track.versions.some((v) => v.platform === 'ytmusic' && v.album) ? h('button', { 'data-act': 'album' }, album) : album)
    : '');

  row.append(...[
    num,
    h('div', { class: 't-titlecell' }, opts.noCover ? null : thumbEl(track.thumbnail),
      h('div', { class: 't-main' },
        h('div', { class: 't-title', title: track.title },
          state.downloadKeys.has(track.key) ? h('span', { class: 'dl-mark', title: 'Downloaded', html: icon('saved') }) : null,
          track.title),
        h('div', { class: 't-sub' }, artistsEl, opts.sub ? h('span', { class: 'faint' }, `· ${opts.sub}`) : null, srcs))),
    albumEl,
    opts.added !== undefined ? h('div', { class: 't-added' }, fmtAdded(opts.added)) : null,
    h('div', { class: 't-like' }, opts.action || null,
      h('button', { class: `icon-btn small like-btn${liked ? ' liked' : ''}`, 'data-act': 'like', title: liked ? 'Remove from Liked Songs' : 'Save to Liked Songs', 'aria-pressed': String(liked), html: icon(liked ? 'heartFill' : 'heart') })),
    h('div', { class: 't-dur' }, fmtTime(track.duration)),
    h('div', { class: 't-more' }, h('button', { class: 'icon-btn small', 'data-act': 'menu', title: `More options for ${track.title}`, 'aria-haspopup': 'menu', html: icon('dots') }))].filter(Boolean));
  return row;
}

function rowPlay(row) {
  return row._opts?.onPlay ? row._opts.onPlay() : playTrack(row._track);
}

function rowMenuItems(row) {
  const track = row._track;
  const versions = sortedVersions(track);
  const anchor = row.querySelector('[data-act="menu"]') || row;
  return [
    { label: 'Add to queue', icon: 'plus', onClick: () => enqueue([track]) },
    { label: 'Play next', icon: 'playNext', onClick: () => enqueue([track], true) },
    { label: 'Add to playlist…', icon: 'list', onClick: () => addToPlaylistMenu(anchor, [track]) },
    { label: state.favKeys.has(track.key) ? 'Remove from Liked Songs' : 'Save to Liked Songs', icon: 'heart', onClick: () => toggleFavorite(track) },
    'sep',
    { label: 'Start radio', icon: 'radio', onClick: () => startRadio(track) },
    track.artist ? { label: 'Go to artist', icon: 'user', onClick: () => openArtist(track.artist, track) } : null,
    trackAlbum(track) && track.versions.some((v) => v.platform === 'ytmusic') ? { label: 'Go to album', icon: 'disc', onClick: () => openAlbumOf(track) } : null,
    { label: `Choose source (${versions.length})…`, icon: 'layers', onClick: () => openVersions(track) },
    state.downloadKeys.has(track.key)
      ? { label: 'Remove download', icon: 'trash', onClick: () => deleteDownload(track.key) }
      : (versions.every((v) => v.platform === 'local') ? null : { label: 'Download', icon: 'download', onClick: () => downloadTracks([track]) }),
    versions[0]?.platform !== 'local' ? { label: 'Share song', icon: 'share', onClick: () => shareSong(track) } : null,
    versions[0]?.platform !== 'local' && cloudOn() ? { label: 'Send to a friend…', icon: 'send', onClick: () => sendSongToFriend(track) } : null,
    jam.code && !jam.host ? { label: 'Suggest for the session', icon: 'friends', onClick: () => jamAdd(track) } : null,
    ...(row._opts?.extraMenu || []),
  ].filter(Boolean);
}

// Delegated row events ---------------------------------------------------------
const rowOf = (el) => { const r = el?.closest?.('.track'); return r && r._track ? r : null; };

document.addEventListener('click', (e) => {
  const row = rowOf(e.target);
  if (!row) return;
  const act = e.target.closest('[data-act]');
  if (act && row.contains(act)) {
    e.stopPropagation();
    const t = row._track;
    switch (act.dataset.act) {
      case 'play': rowPlay(row); break;
      case 'src': playTrack(t, act.dataset.platform); break;
      case 'like': toggleFavorite(t); break;
      case 'menu': openRowMenu(row, act); break;
      case 'album': openAlbumOf(t); break;
      case 'artist': { const i = Number(act.dataset.i); openArtist(act.textContent, i === 0 ? t : null); break; }
      default: break;
    }
    return;
  }
  selectRow(row, e);
});
document.addEventListener('dblclick', (e) => {
  const row = rowOf(e.target);
  if (!row || e.target.closest('[data-act]')) return;
  e.preventDefault();
  rowPlay(row);
});
document.addEventListener('contextmenu', (e) => {
  const row = rowOf(e.target);
  if (!row) return;
  e.preventDefault();
  if (!row.classList.contains('selected')) selectRow(row, {});
  openRowMenu(row, row, e);
});
document.addEventListener('dragstart', (e) => { const row = rowOf(e.target); if (row) startTrackDrag(row, e); });
document.addEventListener('dragend', (e) => { if (rowOf(e.target)) endTrackDrag(); });

// Keyboard: arrows move, Enter plays, Shift+F10 / Menu key opens the menu,
// Home/End jump, Ctrl+A selects all. One row per list is tabbable.
document.addEventListener('keydown', (e) => {
  const row = rowOf(e.target);
  if (!row || e.target !== row) return;
  const k = e.key;
  if (k === 'Enter') { e.preventDefault(); e.stopImmediatePropagation(); rowPlay(row); return; }
  if (k === 'ContextMenu' || (k === 'F10' && e.shiftKey)) {
    e.preventDefault(); e.stopImmediatePropagation();
    if (!row.classList.contains('selected')) selectRow(row, {});
    const r = row.getBoundingClientRect();
    openRowMenu(row, row, { clientX: r.left + 48, clientY: r.top + r.height / 2 });
    return;
  }
  const rows = listRows(row.parentElement);
  let target = null;
  if (k === 'ArrowDown') target = rows[rows.indexOf(row) + 1];
  if (k === 'ArrowUp') target = rows[rows.indexOf(row) - 1];
  if (k === 'Home') target = rows[0];
  if (k === 'End') target = rows[rows.length - 1];
  if (k === 'PageDown') target = rows[Math.min(rows.length - 1, rows.indexOf(row) + 10)];
  if (k === 'PageUp') target = rows[Math.max(0, rows.indexOf(row) - 10)];
  if (target !== null) {
    e.preventDefault(); e.stopImmediatePropagation();
    if (target) { selectRow(target, { shiftKey: e.shiftKey }); target.scrollIntoView({ block: 'nearest' }); }
    return;
  }
  if ((e.ctrlKey || e.metaKey) && k.toLowerCase() === 'a') { e.preventDefault(); e.stopImmediatePropagation(); selectAll(row.parentElement); }
});

// Hover prefetch: a pointer resting on a song resolves its stream so the
// click that usually follows starts sooner. Rows sliding under a still
// pointer while scrolling don't count.
let hoverTimer = null;
let lastScroll = 0;
document.addEventListener('scroll', () => {
  lastScroll = performance.now();
  clearTimeout(hoverTimer);
}, { capture: true, passive: true });
document.addEventListener('mouseover', (e) => {
  const row = rowOf(e.target);
  if (!row || row.contains(e.relatedTarget)) return;
  clearTimeout(hoverTimer);
  if (performance.now() - lastScroll < 300) return;
  hoverTimer = setTimeout(() => prefetchTrack(row._track), 250);
}, { passive: true });
document.addEventListener('mouseout', (e) => {
  const row = rowOf(e.target);
  if (row && !row.contains(e.relatedTarget)) clearTimeout(hoverTimer);
}, { passive: true });

/** The row keyboard focus lands on when tabbing into a list. */
function setRovingRow(list, row) {
  for (const r of list.querySelectorAll(':scope > .track[tabindex="0"]')) if (r !== row) r.tabIndex = -1;
  if (row) row.tabIndex = 0;
}

function openRowMenu(row, anchor, ev) {
  const rows = selectedRows();
  row.classList.add('menu-open');
  const onClose = () => row.classList.remove('menu-open');
  if (rows.length > 1 && rows.includes(row)) {
    openMenu(anchor, bulkMenuItems(rows), { onClose, at: ev });
    return;
  }
  openMenu(anchor, rowMenuItems(row), { onClose, at: ev });
}

// Multi-select ------------------------------------------------------------------
// Click selects, Ctrl/Cmd-click adds, Shift-click selects a range (in one list).
const sel = { list: null, anchor: null };

function listRows(list) {
  return list ? [...list.children].filter((r) => r._track) : [];
}
function selectedRows() {
  return listRows(sel.list).filter((r) => r.classList.contains('selected'));
}
function selectRow(row, e = {}) {
  const list = row.parentElement;
  if (sel.list !== list) clearSelection();
  sel.list = list;
  const rows = listRows(list);
  if (e.shiftKey && sel.anchor && rows.includes(sel.anchor)) {
    const [a, b] = [rows.indexOf(sel.anchor), rows.indexOf(row)].sort((x, y) => x - y);
    if (!(e.ctrlKey || e.metaKey)) rows.forEach((r) => r.classList.remove('selected'));
    rows.slice(a, b + 1).forEach((r) => r.classList.add('selected'));
  } else if (e.ctrlKey || e.metaKey) {
    row.classList.toggle('selected');
    sel.anchor = row;
  } else {
    rows.forEach((r) => r.classList.toggle('selected', r === row));
    sel.anchor = row;
  }
  for (const r of rows) r.setAttribute('aria-selected', r.classList.contains('selected') ? 'true' : 'false');
  setRovingRow(list, row);
  row.focus({ preventScroll: true });
  updateSelectionBar();
}
function selectAll(list) {
  sel.list = list;
  listRows(list).forEach((r) => { r.classList.add('selected'); r.setAttribute('aria-selected', 'true'); });
  updateSelectionBar();
}
function clearSelection() {
  for (const r of $$('.track.selected')) { r.classList.remove('selected'); r.setAttribute('aria-selected', 'false'); }
  sel.list = null;
  sel.anchor = null;
  updateSelectionBar();
}

function bulkMenuItems(rows) {
  const tracks = rows.map((r) => r._track);
  const ctx = sel.list?._ctx;
  const allLiked = tracks.every((t) => state.favKeys.has(t.key));
  return [
    { label: `${tracks.length} songs selected`, heading: true },
    { label: 'Play', icon: 'play', onClick: () => playList(tracks, 0) },
    { label: 'Add to queue', icon: 'plus', onClick: () => enqueue(tracks) },
    { label: 'Play next', icon: 'playNext', onClick: () => enqueue(tracks, true) },
    { label: 'Add to playlist…', icon: 'list', onClick: () => addToPlaylistMenu(rows[0].querySelector('.t-more button') || rows[0], tracks) },
    { label: allLiked ? 'Remove from Liked Songs' : 'Save to Liked Songs', icon: 'heart', onClick: () => likeMany(tracks, !allLiked) },
    { label: 'Download', icon: 'download', onClick: () => downloadTracks(tracks) },
    ctx?.removeMany ? 'sep' : null,
    ctx?.removeMany ? { label: ctx.removeLabel || 'Remove from this playlist', icon: 'trash', onClick: () => ctx.removeMany(rows.map((r) => r._index)) } : null,
  ].filter(Boolean);
}

async function likeMany(tracks, like) {
  let n = 0;
  for (const t of tracks) {
    if (state.favKeys.has(t.key) !== like) {
      try { await call('toggle_favorite', { track: t }); n++; } catch { /* keep going */ }
    }
  }
  toast(like ? `Saved ${n} songs to Liked Songs` : `Removed ${n} songs from Liked Songs`, 'ok', 2200);
}

function updateSelectionBar() {
  const bar = $('#selection-bar');
  if (!bar) return;
  const rows = selectedRows();
  if (rows.length < 2) { bar.hidden = true; return; }
  const tracks = rows.map((r) => r._track);
  const ctx = sel.list?._ctx;
  const b = (label, ic, fn) => h('button', { html: `${icon(ic)}${esc(label)}`, onclick: (e) => { e.stopPropagation(); fn(e); } });
  bar.replaceChildren(...[
    h('span', { class: 'sel-count' }, `${rows.length} selected`),
    b('Play', 'play', () => playList(tracks, 0)),
    b('Queue', 'plus', () => enqueue(tracks)),
    b('Add to playlist', 'list', (e) => addToPlaylistMenu(e.currentTarget, tracks)),
    b('Like', 'heart', () => likeMany(tracks, !tracks.every((t) => state.favKeys.has(t.key)))),
    ctx?.removeMany ? b('Remove', 'trash', () => ctx.removeMany(rows.map((r) => r._index))) : null,
    h('button', { title: 'Clear selection (Esc)', html: icon('x'), onclick: clearSelection })].filter(Boolean));
  bar.hidden = false;
}

// Drag and drop -----------------------------------------------------------------
// Songs can be dragged onto a playlist in Your Library, onto the queue, or
// within a playlist to reorder it.
let trackDrag = null;

function startTrackDrag(row, e) {
  const rows = row.classList.contains('selected') && selectedRows().length > 1 ? selectedRows() : [row];
  trackDrag = { rows, tracks: rows.map((r) => r._track), list: row.parentElement };
  document.body.classList.add('dragging-tracks');
  rows.forEach((r) => r.classList.add('dragging'));
  e.dataTransfer.effectAllowed = 'copyMove';
  e.dataTransfer.setData('text/plain', rows.map((r) => `${r._track.artist} - ${r._track.title}`).join('\n'));
  const ghost = h('div', { class: 'drag-ghost' }, rows.length > 1 ? `${rows.length} songs` : row._track.title);
  document.body.append(ghost);
  try { e.dataTransfer.setDragImage(ghost, 12, 12); } catch { /* optional */ }
  setTimeout(() => ghost.remove(), 0);
}
function endTrackDrag() {
  if (trackDrag) trackDrag.rows.forEach((r) => r.classList.remove('dragging'));
  setDropMark(null, null);
  document.body.classList.remove('dragging-tracks');
  trackDrag = null;
  $$('.drop-before, .drop-after, .drop-target').forEach((x) => x.classList.remove('drop-before', 'drop-after', 'drop-target'));
}

// Reordering inside a list that allows it (list._ctx.reorder).
let dropMark = { row: null, cls: null };
function setDropMark(row, cls) {
  if (dropMark.row === row && dropMark.cls === cls) return; // unchanged: no repaint
  dropMark.row?.classList.remove('drop-before', 'drop-after');
  if (row) row.classList.add(cls);
  dropMark = { row, cls };
}
document.addEventListener('dragover', (e) => {
  if (!trackDrag) return;
  const row = e.target.closest?.('.track');
  const list = row?.parentElement;
  if (!row || list !== trackDrag.list || !list._ctx?.reorder) { setDropMark(null, null); return; }
  e.preventDefault();
  const r = row.getBoundingClientRect();
  setDropMark(row, e.clientY < r.top + r.height / 2 ? 'drop-before' : 'drop-after');
});
document.addEventListener('drop', (e) => {
  if (!trackDrag) return;
  const row = e.target.closest?.('.track');
  const list = row?.parentElement;
  if (!row || list !== trackDrag.list || !list._ctx?.reorder) return;
  e.preventDefault();
  const r = row.getBoundingClientRect();
  const before = e.clientY < r.top + r.height / 2;
  const moving = trackDrag.rows.map((x) => x._index);
  let to = row._index + (before ? 0 : 1);
  to -= moving.filter((i) => i < to).length;
  list._ctx.reorder(moving, to);
  endTrackDrag();
});

/** Column header row. sortKey/onSort make Title/Artist/Time clickable. */
function tableHead(el, { sort, onSort, added = false, albumLabel = 'Album' } = {}) {
  const col = (label, key, cls = '') => {
    if (!onSort || !key) return h('span', { class: cls }, label);
    const active = sort?.key === key;
    const b = h('button', { class: `sortable ${cls}${active ? ' sorted' : ''}`, title: `Sort by ${String(label).toLowerCase()}` }, label);
    if (active) b.insertAdjacentHTML('beforeend', icon(sort.dir === 1 ? 'up' : 'down'));
    b.addEventListener('click', () => onSort(key));
    return b;
  };
  el.className = `table-head${added ? ' has-added' : ''}`;
  const clock = h('span', { class: 'r', title: 'Duration', html: icon('clock') });
  el.replaceChildren(...[
    h('span', { class: 'text-center' }, '#'),
    col('Title', 'title'),
    col(albumLabel, albumLabel ? 'album' : null, 'c-album'),
    added ? col('Date added', 'added', 'c-added') : null,
    h('span'),
    onSort ? (() => { const b = col('', 'duration', 'r'); b.prepend(...h('span', { html: icon('clock') }).childNodes); return b; })() : clock,
    h('span')].filter(Boolean));
  el.hidden = false;
}

/** Grey cards shaped like tiles (artists, albums, playlists). */
function skeletonCards(n = 6) {
  return h('div', { class: 'card-row' }, ...Array.from({ length: n }, () => h('div', { class: 'tile skeleton', 'aria-hidden': 'true' }, h('div', { class: 'tile-art sk' }), h('div', { class: 'sk w-80' }), h('div', { class: 'sk w-50' }))));
}

/** Grey stat cards (Your stats while counting). */
function skeletonStats(n = 4) {
  return h('div', { class: 'stat-grid' }, ...Array.from({ length: n }, () => h('div', { class: 'stat-card skeleton', 'aria-hidden': 'true' }, h('div', { class: 'sk sk-num' }), h('div', { class: 'sk w-60' }))));
}

/** A loading area for assistive tech: busy, with a label. */
function loadingBox(label, ...children) {
  return h('div', { class: 'loading-box', role: 'status', 'aria-busy': 'true', 'aria-label': label }, ...children);
}

function skeletonRows(n = 8) {
  return Array.from({ length: n }, () => h('div', { class: 'track skeleton' },
    h('div'),
    h('div', { class: 't-titlecell' }, h('div', { class: 'thumb' }),
      h('div', { class: 'grow' }, h('div', { class: 'sk', style: `width:${40 + Math.random() * 40}%` }), h('div', { class: 'sk', style: `width:${20 + Math.random() * 25}%` }))),
    h('div', {}, h('div', { class: 'sk', style: `width:${30 + Math.random() * 40}%` })), h('div'), h('div'), h('div')));
}

// Rows currently marked playing/loading: only they and the new ones are
// touched (not every row on the page) on each playback update.
let markedRows = [];
function markPlaying() {
  lastMark = `${playingKey()}|${state.playback?.status}|${state.loadingKey}`;
  const key = playingKey();
  const status = state.playback?.status;
  const keys = [key, state.loadingKey].filter(Boolean);
  const rows = keys.length ? $$(keys.map((k) => `.track[data-key="${CSS.escape(k)}"]`).join(',')) : [];
  for (const row of markedRows) {
    if (rows.includes(row)) continue;
    row.classList.remove('playing', 'loading');
    row.querySelector('.t-num .spinner')?.remove();
  }
  for (const row of rows) {
    const k = row.dataset.key;
    row.classList.toggle('playing', k === key && status !== 'idle');
    const loading = (status === 'loading' && k === key) || (k === state.loadingKey && status !== 'playing');
    row.classList.toggle('loading', loading);
    const num = row.querySelector('.t-num');
    const sp = num?.querySelector('.spinner');
    if (loading && !sp) num.append(h('span', { class: 'spinner' }));
    if (!loading && sp) sp.remove();
  }
  markedRows = rows;
}

/** After rows were added: re-mark only if one of them is the playing/loading song. */
function markPlayingIn(rows) {
  const keys = [playingKey(), state.loadingKey].filter(Boolean);
  if (!keys.length) return;
  const sel = keys.map((k) => `.track[data-key="${CSS.escape(k)}"]`).join(',');
  if (rows.some((r) => (r.matches?.(sel)) || r.querySelector?.(sel))) markPlaying();
}

async function copyText(text) {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    toast('Copied to clipboard', 'ok', 1500);
  } catch {
    openModal({ title: 'Copy', body: h('textarea', { class: 'input', rows: 4, readonly: true }, text) });
  }
}

// ===========================================================================
// Version picker
// ===========================================================================

/** opts.queueUid: picking a version for an existing queue entry. */
function openVersions(track, opts = {}) {
  const versions = sortedVersions(track);
  let selected = opts.current || versions[0]?.platform;
  const list = h('div', { class: 'versions' });
  const render = () => {
    list.replaceChildren(...versions.map((v, i) => {
      const tags = h('span', { class: 'v-name' }, PLATFORMS[v.platform]?.label || v.platform,
        i === 0 ? h('span', { class: 'tag' }, 'default') : null,
        v.preview ? h('span', { class: 'tag warn' }, '30 s preview') : null,
        v.platform === 'spotify' && !state.spotifyUser ? h('span', { class: 'tag' }, 'via YouTube Music') : null);
      const match = v.platform === 'spotify' && !state.spotifyUser && track.versions.find((x) => x.matched_for === 'spotify');
      const sub = match
        ? `Plays “${match.title}” from ${PLATFORMS[match.platform]?.label}${match.isrc_match ? ' (same recording)' : ' (closest match)'}`
        : [v.title, v.artist].filter(Boolean).join(' · ');
      const el = h('button', { class: `version ${v.platform}${v.platform === selected ? ' selected' : ''}` },
        h('span', { class: 'vlogo', html: plogo(v.platform) }),
        h('div', { class: 'min-w-0' }, tags, h('div', { class: 'v-title' }, sub)),
        h('div', { class: 'v-right' }, v.preview ? '0:30' : fmtTime(v.duration)));
      el.addEventListener('click', () => { selected = v.platform; render(); });
      el.addEventListener('dblclick', () => { selected = v.platform; go(); });
      return el;
    }));
  };
  render();
  const go = async () => {
    m.close();
    if (opts.queueUid != null) {
      try {
        await call('set_item_platform', { uid: opts.queueUid, platform: selected });
        const idx = state.queue.items.findIndex((i) => i.uid === opts.queueUid);
        if (idx >= 0 && (idx === state.queue.current || opts.playNow)) await call('play_index_cmd', { index: idx });
      } catch (e) { toast(errMsg(e), 'error'); }
    } else {
      playTrack(track, selected);
    }
  };
  const note = track.versions.some((v) => v.platform === 'spotify') && !state.spotifyUser
    ? h('p', { class: 'muted small mt-12 mx-2'}, 'To play Spotify’s own audio, log in under Settings → Spotify (Premium). Until then Spotify plays the same recording from YouTube Music.')
    : null;
  const footer = opts.queueUid != null
    ? [h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), h('button', { class: 'btn primary', onclick: go }, 'Use this source')]
    : [
      h('button', { class: 'btn', html: `${icon('plus')}Queue`, onclick: () => { m.close(); enqueueVersion(track, selected, false); } }),
      h('button', { class: 'btn', html: `${icon('playNext')}Play next`, onclick: () => { m.close(); enqueueVersion(track, selected, true); } }),
      h('button', { class: 'btn primary', onclick: go, html: `${icon('play')}Play` }),
    ];
  const m = openModal({ title: '', body: h('div', {}, list, note), footer });
  const art = thumbEl(track.thumbnail, 'art');
  m.modal.querySelector('header').replaceWith(h('div', { class: 'modal-hero' }, art,
    h('div', { class: 'min-w-0' }, h('div', { class: 'kicker' }, ''),
      h('h2', {}, track.title), h('div', { class: 'muted' }, track.artist || 'Unknown artist'))));
  m.modal.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); go(); }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const i = versions.findIndex((v) => v.platform === selected);
      selected = versions[(i + (e.key === 'ArrowDown' ? 1 : -1) + versions.length) % versions.length].platform;
      render();
    }
  });
  setTimeout(() => m.modal.querySelector('.btn.primary')?.focus(), 30);
}

async function enqueueVersion(track, platform, next) {
  // Put the chosen version first so it becomes the default for this entry.
  const t = { ...track, versions: [...track.versions].sort((a, b) => (b.platform === platform) - (a.platform === platform)) };
  await enqueue([t], next);
  const snapshot = await call('get_queue');
  const item = next
    ? snapshot.items[(snapshot.current ?? -1) + 1]
    : snapshot.items[snapshot.items.length - 1];
  if (item) await call('set_item_platform', { uid: item.uid, platform }).catch(() => {});
}
