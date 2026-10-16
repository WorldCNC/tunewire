'use strict';

// ===========================================================================
// Track rows & actions
// ===========================================================================

/** Default version index for a track according to the user's platform order. */
function sortedVersions(track) {
  const order = state.settings?.platform_order || ['ytmusic', 'soundcloud', 'youtube', 'spotify'];
  return [...track.versions].sort((a, b) =>
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
    const liked = await call('toggle_favorite', { track });
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
function trackRow(track, index, opts = {}) {
  const versions = sortedVersions(track);
  const play = () => (opts.onPlay ? opts.onPlay() : playTrack(track));
  const row = h('div', { class: 'track', dataset: { key: track.key } });
  const liked = state.favKeys.has(track.key);

  const num = h('div', { class: 't-num' },
    h('span', { class: 'n' }, String(index + 1)),
    h('button', { class: 'icon-btn', title: 'Play', html: icon('play'), onclick: (e) => { e.stopPropagation(); play(); } }));
  num.insertAdjacentHTML('beforeend', EQ);

  const badges = h('div', { class: 'badges t-badges' });
  for (const v of versions) {
    badges.append(pdot(v.platform, {
      preview: v.preview,
      title: `Play from ${PLATFORMS[v.platform]?.label}${v.preview ? ' (30 s preview)' : ''}`,
      onClick: () => playTrack(track, v.platform),
    }));
  }

  const menuBtn = h('button', {
    class: 'icon-btn small', title: 'More', html: icon('dots'),
    onclick: (e) => {
      e.stopPropagation();
      row.classList.add('menu-open');
      const items = [
        { label: 'Play', icon: 'play', onClick: play },
        { label: 'Play next', icon: 'playNext', onClick: () => enqueue([track], true) },
        { label: 'Add to queue', icon: 'plus', onClick: () => enqueue([track]) },
        { label: 'Add to playlist…', icon: 'list', onClick: () => addToPlaylistMenu(menuBtn, [track]) },
        'sep',
        { label: `Choose source (${versions.length})…`, icon: 'layers', onClick: () => openVersions(track) },
        { label: liked ? 'Unlike' : 'Like', icon: 'heart', onClick: () => toggleFavorite(track) },
        track.artist ? { label: 'Go to artist', icon: 'user', onClick: () => openArtist(track.artist, track) } : null,
        state.downloadKeys.has(track.key)
          ? { label: 'Remove download', icon: 'trash', onClick: () => deleteDownload(track.key) }
          : (versions.every((v) => v.platform === 'local') ? null : { label: 'Download', icon: 'download', onClick: () => downloadTracks([track]) }),
        versions[0]?.platform !== 'local' ? { label: 'Copy link', icon: 'link', onClick: () => copyText(versions[0]?.url) } : null,
        ...(opts.extraMenu || []),
      ].filter(Boolean);
      openMenu(menuBtn, items, { onClose: () => row.classList.remove('menu-open') });
    },
  });
  const actions = h('div', { class: 't-actions' },
    h('button', { class: `icon-btn small like-btn${liked ? ' liked' : ''}`, title: liked ? 'Unlike' : 'Like', html: icon(liked ? 'heartFill' : 'heart'), onclick: (e) => { e.stopPropagation(); toggleFavorite(track); } }),
    menuBtn);

  const artist = h('div', { class: 't-artist' });
  if (track.artist) {
    artist.append(h('button', { title: `Go to ${track.artist}`, onclick: (e) => { e.stopPropagation(); openArtist(track.artist, track); } }, track.artist));
  } else {
    artist.append(h('span', { class: 'faint' }, 'Unknown'));
  }

  row.append(
    num,
    h('div', { class: 't-titlecell' }, thumbEl(track.thumbnail),
      h('div', { class: 't-main' },
        h('div', { class: 't-title', title: track.title },
          state.downloadKeys.has(track.key) ? h('span', { class: 'dl-mark', title: 'Downloaded', html: icon('saved') }) : null,
          track.title),
        opts.sub ? h('div', { class: 't-sub' }, opts.sub) : null)),
    artist,
    badges,
    h('div', { class: 't-dur' }, fmtTime(track.duration)),
    actions);

  // Single click selects; double click / Enter plays (desktop list conventions).
  row.tabIndex = -1;
  row.addEventListener('click', () => {
    for (const r of $$('.track.selected')) r.classList.remove('selected');
    row.classList.add('selected');
    row.focus({ preventScroll: true });
  });
  row.addEventListener('dblclick', (e) => { e.preventDefault(); play(); });
  row.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); play(); }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault(); e.stopPropagation();
      const sib = e.key === 'ArrowDown' ? row.nextElementSibling : row.previousElementSibling;
      if (sib?.classList.contains('track')) sib.click();
      sib?.scrollIntoView({ block: 'nearest' });
    }
  });
  row.addEventListener('contextmenu', (e) => { e.preventDefault(); row.click(); menuBtn.click(); });
  return row;
}

/** Column header row. sortKey/onSort make Title/Artist/Time clickable. */
function tableHead(el, { sort, onSort } = {}) {
  const col = (label, key, cls = '') => {
    if (!onSort) return h('span', { class: cls }, label);
    const active = sort?.key === key;
    const b = h('button', { class: `sortable ${cls}${active ? ' sorted' : ''}`, title: `Sort by ${label.toLowerCase()}` }, label);
    if (active) b.insertAdjacentHTML('beforeend', icon(sort.dir === 1 ? 'up' : 'down'));
    b.addEventListener('click', () => onSort(key));
    return b;
  };
  el.replaceChildren(
    h('span', { class: 'r' }, '#'),
    col('Title', 'title'),
    col('Artist', 'artist', 'c-artist'),
    h('span', { class: 'c-src' }, 'Sources'),
    col('Time', 'duration', 'r'),
    h('span'));
  el.hidden = false;
}

function skeletonRows(n = 8) {
  return Array.from({ length: n }, () => h('div', { class: 'track skeleton' },
    h('div'), h('div', { class: 'thumb' }),
    h('div', {}, h('div', { class: 'sk', style: `width:${40 + Math.random() * 40}%` }), h('div', { class: 'sk', style: `width:${20 + Math.random() * 25}%` })),
    h('div'), h('div'), h('div')));
}

function markPlaying() {
  lastMark = `${playingKey()}|${state.playback?.status}|${state.loadingKey}`;
  const key = playingKey();
  const status = state.playback?.status;
  for (const row of $$('.track[data-key]')) {
    const k = row.dataset.key;
    row.classList.toggle('playing', k === key && status !== 'idle');
    const loading = (status === 'loading' && k === key) || (k === state.loadingKey && status !== 'playing');
    row.classList.toggle('loading', loading);
    const num = row.querySelector('.t-num');
    const sp = num?.querySelector('.spinner');
    if (loading && !sp) num.append(h('span', { class: 'spinner' }));
    if (!loading && sp) sp.remove();
  }
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
        v.platform === 'spotify' && !state.spotifyUser ? h('span', { class: 'tag' }, 'plays via YouTube Music') : null);
      const el = h('button', { class: `version ${v.platform}${v.platform === selected ? ' selected' : ''}` },
        h('span', { class: 'vlogo', html: plogo(v.platform) }),
        h('div', { style: 'min-width:0' }, tags, h('div', { class: 'v-title' }, [v.title, v.artist].filter(Boolean).join(' · '))),
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
    ? h('p', { class: 'muted small', style: 'margin:12px 2px 0' }, 'To play Spotify’s own audio, log in under Settings → Spotify (Premium). Until then Spotify plays the same recording from YouTube Music.')
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
    h('div', { style: 'min-width:0' }, h('div', { class: 'kicker' }, ''),
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
