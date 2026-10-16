'use strict';

// ===========================================================================
// Playlists
// ===========================================================================

async function refreshPlaylists() {
  try { state.playlists = await call('list_playlists'); } catch (e) { toast(errMsg(e), 'error'); }
  renderPlaylistList();
  if (state.selectedPlaylist && state.selectedPlaylist !== 'remote') {
    if (!state.playlists.some((p) => p.id === state.selectedPlaylist)) state.selectedPlaylist = null;
    renderPlaylistDetail();
  }
}

function renderPlaylistList() {
  const list = $('#playlist-list');
  const rows = [];
  const item = (id, name, count, tracks, ico) => {
    const art = h('span', { class: 'pl-ico', html: tracks.find((t) => t.thumbnail) ? '' : icon(ico) });
    const th = tracks.find((t) => t.thumbnail)?.thumbnail;
    if (th) art.style.backgroundImage = `url("${th.replace(/"/g, '%22')}")`;
    return h('button', { class: `pl-item${state.selectedPlaylist === id ? ' active' : ''}`, onclick: () => { state.selectedPlaylist = id; renderPlaylistList(); renderPlaylistDetail(); } },
      art, h('span', { class: 'pl-name' }, name), h('span', { class: 'pl-count' }, String(count)));
  };
  if (state.remote) rows.push(item('remote', state.remote.title, state.remote.tracks.length, state.remote.tracks, 'link'));
  for (const pl of state.playlists) rows.push(item(pl.id, pl.name, pl.tracks.length, pl.tracks, 'list'));
  if (!rows.length) rows.push(h('p', { class: 'meta', style: 'padding:6px' }, 'No playlists yet.'));
  list.replaceChildren(...rows);

  // Sidebar shortcut list
  $('#side-playlists').replaceChildren(...state.playlists.slice(0, 12).map((pl) => h('button', {
    class: state.view === 'playlists' && state.selectedPlaylist === pl.id ? 'active' : '', title: pl.name,
    onclick: () => { state.selectedPlaylist = pl.id; showView('playlists'); renderPlaylistList(); renderPlaylistDetail(); },
  }, pl.name)));
  if (!state.playlists.length) $('#side-playlists').replaceChildren(h('div', { class: 'none' }, 'None yet'));
}

function playlistArt(tracks) {
  const thumbs = [...new Set(tracks.map((t) => t.thumbnail).filter(Boolean))].slice(0, 4);
  const art = h('div', { class: `pl-art${thumbs.length < 4 ? ' single' : ''}` });
  if (!thumbs.length) { art.innerHTML = `<div style="display:grid;place-items:center;color:var(--text-faint)">${icon('music')}</div>`; return art; }
  for (const t of thumbs.length < 4 ? thumbs.slice(0, 1) : thumbs) art.append(h('div', { style: `background-image:url("${t.replace(/"/g, '%22')}")` }));
  return art;
}

function renderPlaylistDetail() {
  const box = $('#playlist-detail');
  const id = state.selectedPlaylist;
  if (!id) {
    box.replaceChildren(h('div', { class: 'empty' }, h('span', { class: 'big', html: icon('list') }), h('p', {}, 'Select a playlist, create one, or load a playlist link.'), h('p', { class: 'muted small' }, 'SoundCloud sets, YouTube / YouTube Music playlists and Spotify playlists & albums are supported.')));
    return;
  }
  const remote = id === 'remote';
  const pl = remote ? state.remote : state.playlists.find((p) => p.id === id);
  if (!pl) return;
  const tracks = pl.tracks;
  const total = tracks.reduce((s, t) => s + (t.duration || 0), 0);

  const actions = h('div', { class: 'actions' },
    h('button', { class: 'btn primary', html: `${icon('play')}Play`, onclick: () => playList(tracks, 0) }),
    h('button', { class: 'btn', html: `${icon('shuffle')}Shuffle`, onclick: () => playList([...tracks].sort(() => Math.random() - 0.5), 0) }),
    h('button', { class: 'btn', html: `${icon('plus')}Queue all`, onclick: () => enqueue(tracks) }));
  if (remote) {
    actions.append(h('button', {
      class: 'btn', html: `${icon('save')}Save as local`, onclick: async () => {
        const name = await promptModal({ title: 'Save playlist', value: pl.title, okLabel: 'Save' });
        if (!name) return;
        try {
          const created = await call('create_playlist', { name, tracks, sourceUrl: pl.url });
          state.selectedPlaylist = created.id;
          toast(`Saved ${name}`, 'ok');
        } catch (e) { toast(errMsg(e), 'error'); }
      },
    }));
  } else {
    if (pl.source_url) {
      const syncBtn = h('button', { class: 'btn', html: `${icon('refresh')}Sync`, title: `Update from ${pl.source_url}`, onclick: () => syncPlaylist(pl.id, syncBtn) });
      actions.append(syncBtn);
    }
    actions.append(
      h('button', { class: 'btn', html: `${icon('edit')}Rename`, onclick: async () => {
        const name = await promptModal({ title: 'Rename playlist', value: pl.name, okLabel: 'Rename' });
        if (name) call('rename_playlist', { id: pl.id, name }).catch((e) => toast(errMsg(e), 'error'));
      } }),
      h('button', { class: 'btn danger-ghost', html: `${icon('trash')}Delete`, onclick: async () => {
        if (await confirmModal({ title: 'Delete playlist?', message: `“${pl.name}” and its ${tracks.length} tracks will be removed. This cannot be undone.`, okLabel: 'Delete', danger: true })) {
          call('delete_playlist', { id: pl.id }).then(() => { state.selectedPlaylist = null; }).catch((e) => toast(errMsg(e), 'error'));
        }
      } }));
  }

  const header = h('div', { class: 'pl-header' }, playlistArt(tracks),
    h('div', { class: 'pl-meta' },
      h('div', { class: 'pl-kicker' }, remote ? `Remote playlist${pl.platform ? ' · ' + (PLATFORMS[pl.platform]?.label || '') : ''}` : 'Playlist'),
      h('h1', {}, remote ? pl.title : pl.name),
      h('div', { class: 'meta', style: 'margin-bottom:10px' }, `${tracks.length} tracks, ${fmtTime(total)}`),
      matchStatus(tracks),
      actions));

  const head = h('div', { class: 'table-head' });
  if (tracks.length) tableHead(head);
  const list = h('div', { class: 'track-list' });
  tracks.forEach((t, i) => {
    const extra = remote ? [] : [
      'sep',
      { label: 'Move up', icon: 'up', onClick: () => i > 0 && call('move_in_playlist', { id: pl.id, from: i, to: i - 1 }) },
      { label: 'Move down', icon: 'down', onClick: () => i < tracks.length - 1 && call('move_in_playlist', { id: pl.id, from: i, to: i + 1 }) },
      { label: 'Remove from playlist', icon: 'trash', onClick: () => call('remove_from_playlist', { id: pl.id, index: i }).catch((e) => toast(errMsg(e), 'error')) },
    ];
    list.append(trackRow(t, i, { onPlay: () => playList(tracks, i), extraMenu: extra }));
  });
  if (!tracks.length) list.append(h('div', { class: 'empty' }, h('p', {}, 'This playlist is empty. Add songs from search results with the ⋯ menu.')));
  box.replaceChildren(header, head, list, recommendSection(pl, remote));
  markPlaying();
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

listen('match-progress', (e) => {
  const p = e.payload;
  const wasRunning = state.matchProgress && !state.matchProgress.finished;
  state.matchProgress = p;
  paintMatchStatus();
  if (p.finished && wasRunning && p.total >= 10) {
    toast(`${p.matched} Spotify songs are ready to play instantly${p.matched < p.total ? ` (${p.total - p.matched} not found on YouTube Music)` : ''}`, 'ok', 5000);
  }
});

// Recommendations ------------------------------------------------------------
// Kept per playlist so re-rendering (likes, adds) doesn't refetch.
const recState = new Map();

function recommendSection(pl, remote) {
  const id = remote ? 'remote' : pl.id;
  const tracks = pl.tracks;
  const box = h('div', { class: 'rec-section' });
  if (!tracks.length) return box;
  let rs = recState.get(id);
  if (!rs || rs.src !== tracks.length) {
    rs = { nonce: rs?.nonce || 1, list: null, loading: false, error: null, src: tracks.length, added: new Set(rs?.added || []) };
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
      box.replaceChildren(head, h('div', { class: 'track-list' }, ...skeletonRows(5)));
      return;
    }
    if (rs.error) {
      box.replaceChildren(head, h('div', { class: 'empty' }, h('p', {}, rs.error)));
      return;
    }
    const visible = (rs.list || []).filter((r) => !rs.added.has(r.track.key));
    const list = h('div', { class: 'track-list stagger' }, ...visible.map((r, i) => {
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

$('#side-new-playlist').addEventListener('click', () => $('#new-playlist').click());
$('#new-playlist').addEventListener('click', async () => {
  const name = await promptModal({ title: 'New playlist', okLabel: 'Create', placeholder: 'My playlist' });
  if (!name) return;
  try {
    const pl = await call('create_playlist', { name });
    state.selectedPlaylist = pl.id;
  } catch (e) { toast(errMsg(e), 'error'); }
});

$('#remote-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const url = $('#remote-url').value.trim();
  if (!url) return;
  const btn = e.submitter || $('#remote-form button');
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>';
  try {
    state.remote = await call('load_playlist', { url });
    state.selectedPlaylist = 'remote';
    renderPlaylistList();
    renderPlaylistDetail();
    $('#remote-url').value = '';
  } catch (er) {
    toast(errMsg(er), 'error');
  } finally {
    btn.disabled = false;
    btn.textContent = 'Load';
  }
});

// ===========================================================================
// Favorites & history
// ===========================================================================

async function refreshFavorites() {
  try { state.favorites = await call('list_favorites'); } catch (e) { toast(errMsg(e), 'error'); }
  state.favKeys = new Set(state.favorites.map((t) => t.key));
  $('#fav-count').textContent = state.favorites.length || '';
  $('#fav-meta').textContent = `${state.favorites.length} song${state.favorites.length === 1 ? '' : 's'} · ${fmtTime(state.favorites.reduce((s, t) => s + (t.duration || 0), 0))}`;
  const list = $('#favorites-list');
  if (!state.favorites.length) {
    $('#fav-head').hidden = true;
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'No liked songs yet.'), h('p', { class: 'small' }, 'Use the heart on any row, or press L while a song plays.')));
  } else {
    tableHead($('#fav-head'));
    list.replaceChildren(...state.favorites.map((t, i) => trackRow(t, i, { onPlay: () => playList(state.favorites, i) })));
  }
  // Refresh heart states elsewhere.
  for (const row of $$('.track[data-key]')) {
    const btn = row.querySelector('.t-actions button[title="Like"], .t-actions button[title="Unlike"]');
    if (!btn) continue;
    const liked = state.favKeys.has(row.dataset.key);
    btn.classList.toggle('liked', liked);
    btn.title = liked ? 'Unlike' : 'Like';
    btn.innerHTML = icon(liked ? 'heartFill' : 'heart');
  }
  renderPlayback();
  markPlaying();
}
$('#fav-play').addEventListener('click', () => playList(state.favorites, 0));
$('#fav-shuffle').addEventListener('click', () => playList([...state.favorites].sort(() => Math.random() - 0.5), 0));
$('#fav-queue').addEventListener('click', () => state.favorites.length && enqueue(state.favorites));

async function refreshHistory() {
  try { state.history = await call('list_history', { limit: 300 }); } catch (e) { toast(errMsg(e), 'error'); }
  const list = $('#history-list');
  if (state.view === 'search' && $('#search-body').hidden) renderHero();
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
  if (await confirmModal({ title: 'Clear history?', message: 'Your local playback history will be erased.', okLabel: 'Clear', danger: true })) {
    call('clear_history').catch((e) => toast(errMsg(e), 'error'));
  }
});
