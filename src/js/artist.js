'use strict';

// ===========================================================================
// Artist & album pages
// ===========================================================================

/** Best SoundCloud / YouTube page URLs for an artist, taken from a track's versions. */
function artistLinks(track) {
  const vs = track?.versions || [];
  const sc = vs.find((v) => v.platform === 'soundcloud' && v.artist_url)?.artist_url;
  const yt = vs.find((v) => (v.platform === 'youtube' || v.platform === 'ytmusic') && v.artist_url?.includes('youtube.com/'))?.artist_url;
  return { sc, yt };
}

let artistSeq = 0;
async function openArtist(name, track = null) {
  if (!name) return;
  const primary = name.split(/,| & | x | feat\.? /i)[0].trim();
  const seq = ++artistSeq;
  showView('artist');
  const body = $('#artist-body');
  body.replaceChildren(h('div', { class: 'page-head' }, h('div', {}, backButton(), h('h1', {}, primary))), h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Loading artist…'));
  const { sc, yt } = artistLinks(track);
  let page;
  try {
    page = await call('artist_page', { name: primary, soundcloudUrl: sc || null });
  } catch (e) {
    if (seq !== artistSeq) return;
    body.replaceChildren(h('div', { class: 'page-head' }, h('div', {}, backButton(), h('h1', {}, primary))),
      h('div', { class: 'empty' }, h('p', {}, errMsg(e)), h('button', { class: 'btn', onclick: () => { $('#search-input').value = primary; syncSearchClear(); doSearch(primary); } }, `Search for “${primary}” instead`)));
    return;
  }
  if (seq !== artistSeq) return;
  const all = [...page.songs, ...page.soundcloud];
  const header = h('div', { class: 'artist-head' },
    thumbEl(page.thumbnail, 'artist-photo'),
    h('div', { class: 'artist-meta' },
      backButton(),
      h('h1', {}, page.name),
      h('div', { class: 'meta' }, [page.songs.length && `${page.songs.length} songs on YouTube Music`, page.soundcloud.length && `${page.soundcloud.length} on SoundCloud`].filter(Boolean).join(' · ')),
      h('div', { class: 'actions' },
        h('button', { class: 'btn primary', html: `${icon('play')}Play`, onclick: () => playList(all, 0) }),
        h('button', { class: 'btn', html: `${icon('shuffle')}Shuffle`, onclick: () => playList([...all].sort(() => Math.random() - 0.5), 0) }))));

  const section = (title, tracks) => {
    if (!tracks.length) return null;
    const head = h('div', { class: 'table-head' });
    tableHead(head);
    const shown = tracks.slice(0, 10);
    const list = h('div', { class: 'track-list' }, ...shown.map((t, i) => trackRow(t, i, { onPlay: () => playList(tracks, i) })));
    const more = tracks.length > 10 ? h('button', { class: 'btn small show-more', onclick: (e) => { list.replaceChildren(...tracks.map((t, i) => trackRow(t, i, { onPlay: () => playList(tracks, i) }))); e.target.remove(); markPlaying(); } }, `Show all ${tracks.length}`) : null;
    return h('div', { class: 'artist-section' }, h('div', { class: 'section-title' }, title), head, list, more);
  };
  const albumsBox = h('div', { class: 'artist-section' }, h('div', { class: 'section-title' }, 'Releases'), h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Loading releases…'));
  let scBox = section('SoundCloud', page.soundcloud);
  body.replaceChildren(header, section('Songs', page.songs), scBox, albumsBox);
  markPlaying();

  // SoundCloud artist listings lack artwork/duration; fill them in.
  const partial = page.soundcloud.flatMap((t) => t.versions.filter((v) => v.partial).map((v) => v.url)).slice(0, 12);
  if (partial.length) {
    call('complete_metadata', { urls: partial }).then((metas) => {
      if (seq !== artistSeq || !metas.length) return;
      applyMetas(page.soundcloud, metas);
      const fresh = section('SoundCloud', page.soundcloud);
      if (scBox && fresh) { scBox.replaceWith(fresh); scBox = fresh; markPlaying(); }
    }).catch(() => {});
  }

  try {
    const albums = await call('artist_albums', { name: page.name, channelUrl: yt || null });
    if (seq !== artistSeq) return;
    if (!albums.length) { albumsBox.remove(); return; }
    albumsBox.replaceChildren(h('div', { class: 'section-title' }, 'Releases'), albumGrid(albums));
  } catch {
    albumsBox.remove();
  }
}

/** Grid of album cards; covers load lazily, a few at a time. */
function albumGrid(albums) {
  const grid = h('div', { class: 'album-grid' });
  const pending = [];
  for (const a of albums) {
    const art = h('div', { class: 'album-art', html: icon('disc') });
    const card = h('button', { class: 'album-card', title: a.title, onclick: () => openAlbum(a.url, a.title) },
      art, h('div', { class: 'album-title' }, a.title));
    if (a.thumbnail) setArt(art, a.thumbnail);
    else pending.push([a.url, art]);
    grid.append(card);
  }
  // Fill covers 3 at a time (each is a yt-dlp call).
  const worker = async () => {
    while (pending.length) {
      const [url, art] = pending.shift();
      if (!art.isConnected) continue;
      try { const c = await call('album_cover', { url }); if (c) setArt(art, c); } catch { /* keep placeholder */ }
    }
  };
  for (let i = 0; i < 3; i++) worker();
  return grid;
}

function setArt(el, url) {
  el.replaceChildren(h('img', { src: url, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => { e.target.remove(); el.innerHTML = icon('disc'); } }));
}

let albumSeq = 0;
async function openAlbum(url, title = 'Album') {
  const seq = ++albumSeq;
  showView('album');
  const body = $('#album-body');
  body.replaceChildren(h('div', { class: 'page-head' }, h('div', {}, backButton(), h('h1', {}, title))), h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Loading album…'));
  let pl;
  try {
    pl = await call('album_page', { url });
  } catch (e) {
    if (seq !== albumSeq) return;
    body.replaceChildren(h('div', { class: 'page-head' }, h('div', {}, backButton(), h('h1', {}, title))), h('div', { class: 'empty' }, h('p', {}, errMsg(e))));
    return;
  }
  if (seq !== albumSeq) return;
  const tracks = pl.tracks;
  const total = tracks.reduce((s, t) => s + (t.duration || 0), 0);
  const artist = tracks[0]?.artist || '';
  const header = h('div', { class: 'pl-header' },
    thumbEl(tracks[0]?.thumbnail, 'pl-art single'),
    h('div', { class: 'pl-meta' },
      backButton(),
      h('div', { class: 'pl-kicker' }, 'Album'),
      h('h1', {}, pl.title),
      h('div', { class: 'meta', style: 'margin-bottom:10px' },
        artist ? h('button', { class: 'link', onclick: () => openArtist(artist, tracks[0]) }, artist) : null,
        `${artist ? ' · ' : ''}${tracks.length} tracks, ${fmtTime(total)}`),
      h('div', { class: 'actions' },
        h('button', { class: 'btn primary', html: `${icon('play')}Play`, onclick: () => playList(tracks, 0) }),
        h('button', { class: 'btn', html: `${icon('shuffle')}Shuffle`, onclick: () => playList([...tracks].sort(() => Math.random() - 0.5), 0) }),
        h('button', { class: 'btn', html: `${icon('plus')}Queue`, onclick: () => enqueue(tracks) }),
        h('button', { class: 'btn', html: `${icon('download')}Download`, onclick: () => downloadTracks(tracks) }),
        h('button', { class: 'btn', html: `${icon('save')}Save`, onclick: async () => {
          const name = await promptModal({ title: 'Save as playlist', value: pl.title, okLabel: 'Save' });
          if (name) call('create_playlist', { name, tracks, sourceUrl: url }).then(() => toast(`Saved ${name}`, 'ok')).catch((e) => toast(errMsg(e), 'error'));
        } }))));
  const head = h('div', { class: 'table-head' });
  tableHead(head);
  body.replaceChildren(header, head, h('div', { class: 'track-list' }, ...tracks.map((t, i) => trackRow(t, i, { onPlay: () => playList(tracks, i) }))));
  markPlaying();
}

/** Patch tracks in place with completed metadata from the backend. */
function applyMetas(tracks, metas) {
  const byUrl = new Map(metas.map((m) => [m.url, m]));
  for (const t of tracks) {
    for (const v of t.versions) {
      const m = byUrl.get(v.url);
      if (!m) continue;
      if (m.artist && !v.artist) v.artist = m.artist;
      v.duration = v.duration ?? m.duration;
      // SoundCloud Go+ tracks only stream a 30-second preview.
      if (v.platform === 'soundcloud' && m.duration && Math.abs(m.duration - 30) < 0.6) v.preview = true;
      v.thumbnail = v.thumbnail ?? m.thumbnail;
      v.album = v.album ?? m.album;
      v.partial = false;
      t.thumbnail = t.thumbnail ?? m.thumbnail;
      t.duration = t.duration ?? m.duration;
    }
  }
}
