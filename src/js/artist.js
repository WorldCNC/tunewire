'use strict';

// ===========================================================================
// Artist & album pages (YouTube Music's own API: one request each)
// ===========================================================================

/** SoundCloud profile of an artist, taken from a track's versions. */
function soundcloudLink(track) {
  return track?.versions?.find((v) => v.platform === 'soundcloud' && v.artist_url)?.artist_url || null;
}

let artistSeq = 0;
async function openArtist(name, track = null, { browseId = null, thumbnail = null, push = true } = {}) {
  if (!name && !browseId) return;
  const primary = (name || '').split(/,| & | x | feat\.? /i)[0].trim();
  const seq = ++artistSeq;
  state.artistName = primary;
  showView('artist', { push, reopen: () => openArtist(name, track, { browseId, thumbnail, push: false }) });
  const body = $('#artist-body');
  body.style.setProperty('--hero', '#3a3a3a');
  body.replaceChildren(
    h('div', { class: 'artist-hero' }, h('div', {}, h('h1', {}, primary || 'Artist'))),
    h('div', { class: 'action-bar' }, h('span', { class: 'big-play', html: icon('play') })),
    h('div', { class: 'track-list popular' }, ...skeletonRows(5)));
  let a;
  try {
    a = await call('ytm_artist', { name: primary, browseId: browseId || artistBrowseId(track) });
  } catch (e) {
    if (seq !== artistSeq) return;
    body.replaceChildren(h('div', { class: 'page-head' }, h('h1', {}, primary)),
      h('div', { class: 'empty' }, h('p', {}, errMsg(e)), h('button', { class: 'btn', onclick: () => { $('#search-input').value = primary; syncSearchClear(); doSearch(primary); } }, `Search for “${primary}” instead`)));
    return;
  }
  if (seq !== artistSeq) return;
  state.artistName = a.name;
  renderLibrary();
  const art = a.banner || thumbnail || a.thumbnail;
  const hero = h('div', { class: 'artist-hero' },
    h('div', {},
      h('div', { class: 'verified' }, h('span', { html: icon('verified'), style: 'color:#3d91f4;display:grid' }), 'Artist'),
      h('h1', {}, a.name),
      h('div', { class: 'audience' }, a.audience || (a.subscribers ? `${a.subscribers} subscribers` : ''))));
  if (art) hero.style.backgroundImage = `url("${art.replace(/"/g, '%22')}")`;

  let allSongs = null;
  const playAll = async (shuffle = false) => {
    let list = a.top_songs;
    if (a.all_songs) {
      try { allSongs = allSongs || (await call('ytm_collection', { id: a.all_songs })).tracks; list = allSongs; } catch { /* top songs only */ }
    }
    if (shuffle) list = [...list].sort(() => Math.random() - 0.5);
    playCollection(list, 0, `artist:${a.browse_id}`);
  };
  const follow = h('button', { class: 'ab-follow' });
  const paintFollow = () => { follow.textContent = isFollowing(a.name) ? 'Following' : 'Follow'; };
  follow.addEventListener('click', async () => {
    try {
      const now = await call('follow_artist', { name: a.name, url: `https://www.youtube.com/channel/${a.browse_id}`, thumbnail: a.thumbnail || null });
      await refreshFollowed();
      paintFollow();
      renderLibrary();
      toast(now ? `Following ${a.name}. New releases show up under New releases.` : `Unfollowed ${a.name}`, 'ok', 2200);
    } catch (e) { toast(errMsg(e), 'error'); }
  });
  paintFollow();
  const bar = h('div', { class: 'action-bar' },
    h('button', { class: 'big-play', title: `Play ${a.name}`, html: icon('play'), onclick: () => playAll() }),
    h('button', { class: 'ab-icon', title: 'Shuffle', html: icon('shuffle'), onclick: () => playAll(true) }),
    follow,
    h('button', { class: 'ab-icon', title: `Start ${a.name} radio`, html: icon('radio'), disabled: !a.top_songs.length, onclick: () => startRadio(a.top_songs[0]) }),
    (() => {
      const b = h('button', { class: 'ab-icon', title: 'More options', html: icon('dots') });
      b.addEventListener('click', () => openMenu(b, [
        { label: 'Add top songs to queue', icon: 'plus', onClick: () => enqueue(a.top_songs) },
        { label: 'Copy link to artist', icon: 'link', onClick: () => copyText(`https://music.youtube.com/channel/${a.browse_id}`) },
      ]));
      return b;
    })());

  // Popular: top 5, "See more" shows up to 10 from all their songs.
  const popular = h('div', { class: 'track-list popular' });
  let expanded = false;
  const renderPopular = async () => {
    let list = a.top_songs;
    if (expanded && a.all_songs) {
      try { allSongs = allSongs || (await call('ytm_collection', { id: a.all_songs })).tracks; list = allSongs.slice(0, 10); } catch { /* keep top */ }
    }
    if (seq !== artistSeq) return;
    const shown = expanded ? list.slice(0, 10) : list.slice(0, 5);
    popular.replaceChildren(...shown.map((t, i) => trackRow(t, i, { onPlay: () => playCollection(list, i, `artist:${a.browse_id}`) })));
    popular._ctx = {};
    more.textContent = expanded ? 'Show less' : 'See more';
    markPlaying();
  };
  const more = h('button', { class: 'btn small show-more', onclick: () => { expanded = !expanded; renderPopular(); } }, 'See more');

  // Discography: Albums / Singles & EPs.
  const discBox = h('div');
  const tabs = [['Popular releases', [...a.albums.slice(0, 4), ...a.singles.slice(0, 4)]], ['Albums', a.albums], ['Singles and EPs', a.singles]].filter(([, c]) => c.length);
  let disc = tabs[0]?.[0];
  const renderDisc = () => {
    const cards = tabs.find(([n]) => n === disc)?.[1] || [];
    discBox.replaceChildren(
      h('div', { class: 'disc-tabs' }, ...tabs.map(([n]) => h('button', { class: `chip-btn${n === disc ? ' active' : ''}`, onclick: () => { disc = n; renderDisc(); } }, n))),
      h('div', { class: 'card-row' }, ...cards.slice(0, 14).map((c, i) => cardTile(c, i))));
  };
  if (tabs.length) renderDisc();

  const sections = [hero, bar,
    a.top_songs.length ? h('div', {}, h('div', { class: 'section-title', style: 'margin-top:8px' }, 'Popular'), popular, a.top_songs.length > 5 || a.all_songs ? more : null) : null,
    tabs.length ? h('div', {}, h('div', { class: 'section-title' }, 'Discography'), discBox) : null];
  for (const shelf of a.shelves) {
    if (!shelf.cards.length || /live performance/i.test(shelf.title)) continue;
    sections.push(h('div', {}, h('div', { class: 'section-title' }, shelf.title), cardRow(shelf.cards, 7)));
  }
  if (a.related.length) sections.push(h('div', {}, h('div', { class: 'section-title' }, 'Fans also like'), cardRow(a.related, 7)));
  const scUrl = soundcloudLink(track);
  const scBox = scUrl ? h('div') : null;
  if (scBox) sections.push(scBox);
  if (a.description || a.audience) {
    const about = h('div', { class: 'about-card' }, h('div', {}, h('div', { class: 'about-aud' }, a.audience || ''), a.description ? h('p', {}, a.description) : null));
    if (art) about.style.backgroundImage = `url("${art.replace(/"/g, '%22')}")`;
    about.addEventListener('click', () => openModal({ title: a.name, subtitle: a.audience, wide: true, body: h('p', { style: 'white-space:pre-wrap;line-height:1.6' }, a.description || '') }));
    sections.push(h('div', {}, h('div', { class: 'section-title' }, 'About'), about));
  }
  body.replaceChildren(...sections.filter(Boolean));
  renderPopular();
  heroColor(body, a.thumbnail || art, () => seq === artistSeq);

  if (scBox) {
    call('artist_soundcloud', { url: scUrl, name: a.name }).then((tracks) => {
      if (seq !== artistSeq || !tracks.length) return;
      const list = h('div', { class: 'track-list' }, ...tracks.slice(0, 10).map((t, i) => trackRow(t, i, { onPlay: () => playList(tracks, i) })));
      list._ctx = {};
      scBox.replaceChildren(h('div', { class: 'section-title' }, 'On SoundCloud'), list);
      markPlaying();
      const partial = tracks.flatMap((t) => t.versions.filter((v) => v.partial).map((v) => v.url)).slice(0, 10);
      if (partial.length) {
        call('complete_metadata', { urls: partial }).then((metas) => {
          if (seq !== artistSeq || !metas.length) return;
          applyMetas(tracks, metas);
          list.replaceChildren(...tracks.slice(0, 10).map((t, i) => trackRow(t, i, { onPlay: () => playList(tracks, i) })));
          markPlaying();
        }).catch(() => {});
      }
    }).catch(() => scBox.remove());
  }
}

/** Grid of album cards (charts, releases); covers load lazily, a few at a time. */
function albumGrid(albums) {
  const grid = h('div', { class: 'card-row' });
  const pending = [];
  albums.forEach((a, i) => {
    const art = h('div', { class: 'tile-art', html: icon('disc') });
    art.append(h('button', { class: 'tile-play', title: `Play ${a.title}`, html: icon('play'), onclick: (e) => { e.stopPropagation(); call('ytm_collection', { url: a.url }).then((c) => playList(c.tracks, 0)).catch((er) => toast(errMsg(er), 'error')); } }));
    const card = h('div', { class: 'tile', style: `--i:${i}`, role: 'button', tabindex: '0', title: a.title },
      art, a.isNew ? h('span', { class: 'new-badge' }, 'New') : null, h('div', { class: 'tile-title' }, a.title), a.subtitle ? h('div', { class: 'tile-sub' }, a.subtitle) : null);
    card.addEventListener('click', () => openAlbum(a.url, a.title));
    if (a.thumbnail) setArt(art, a.thumbnail);
    else pending.push([a.url, art]);
    grid.append(card);
  });
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
  const play = el.querySelector('.tile-play');
  el.replaceChildren(h('img', { src: url, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => { e.target.remove(); el.prepend(...h('div', { html: icon('disc') }).childNodes); } }));
  if (play) el.append(play);
}

/** "Go to album" from a song. */
async function openAlbumOf(track) {
  const album = trackAlbum(track);
  if (!album) return;
  try {
    const id = await call('ytm_find_album', { artist: track.artist, album });
    if (id) return openAlbum({ browseId: id, title: album });
  } catch { /* fall through */ }
  $('#search-input').value = `${track.artist} ${album}`;
  syncSearchClear();
  doSearch(`${track.artist} ${album}`);
}

let albumSeq = 0;
/** Album or playlist page: openAlbum({browseId, title}) or openAlbum(url, title). */
async function openAlbum(target, title = 'Album', { push = true } = {}) {
  const seq = ++albumSeq;
  const browseId = typeof target === 'object' ? target.browseId : null;
  const url = typeof target === 'string' ? target : null;
  title = (typeof target === 'object' && target.title) || title;
  showView('album', { push, reopen: () => openAlbum(target, title, { push: false }) });
  const body = $('#album-body');
  body.style.setProperty('--hero', '#3a3a3a');
  body.replaceChildren(
    h('div', { class: 'hero' }, h('div', { class: 'hero-art', html: icon('disc') }), h('div', { class: 'hero-meta' }, h('div', { class: 'hero-kicker' }, browseId?.startsWith('VL') ? 'Playlist' : 'Album'), h('h1', { class: 'hero-title' }, title))),
    h('div', { class: 'action-bar' }, h('span', { class: 'big-play', html: icon('play') })),
    h('div', { class: 'track-list' }, ...skeletonRows(8)));
  let c;
  try {
    c = await call('ytm_collection', { id: browseId, url });
  } catch (e) {
    if (seq !== albumSeq) return;
    body.replaceChildren(h('div', { class: 'page-head' }, h('h1', {}, title)), h('div', { class: 'empty' }, h('p', {}, errMsg(e))));
    return;
  }
  if (seq !== albumSeq) return;
  const isAlbum = c.browse_id.startsWith('MPRE') || (!c.browse_id && c.kind !== 'Playlist');
  const art = h('div', { class: 'hero-art' });
  if (c.cover) art.append(h('img', { src: c.cover, alt: '', referrerpolicy: 'no-referrer' }));
  else art.innerHTML = icon('disc');
  const line = [];
  if (c.artist) line.push(c.artist_browse ? h('button', { onclick: () => openArtist(c.artist, null, { browseId: c.artist_browse }) }, c.artist) : h('b', {}, c.artist));
  if (c.year) line.push(h('span', { class: 'dot' }, c.year));
  const sourceUrl = url || (c.browse_id.startsWith('VL') ? `https://music.youtube.com/playlist?list=${c.browse_id.slice(2)}` : c.browse_id ? `https://music.youtube.com/browse/${c.browse_id}` : null);
  renderCollection(body, {
    id: `album:${c.browse_id || url}`,
    kind: c.kind || (isAlbum ? 'Album' : 'Playlist'),
    title: c.title || title,
    tracks: c.tracks,
    art,
    cover: c.cover,
    description: isAlbum ? '' : c.description,
    line,
    noCover: isAlbum,
    noAlbum: isAlbum,
    actions: [
      h('button', { class: 'ab-icon', title: 'Save to Your Library', html: icon('plus'), onclick: async () => {
        try {
          const created = await call('create_playlist', { name: c.title, tracks: c.tracks, sourceUrl });
          toast(`Saved “${c.title}” to Your Library`, 'ok');
          refreshPlaylists({ silent: true }).then(() => renderLibrary());
          void created;
        } catch (e) { toast(errMsg(e), 'error'); }
      } }),
      h('button', { class: 'ab-icon', title: 'Download', html: icon('download'), onclick: () => downloadTracks(c.tracks) }),
    ],
    menu: () => [
      { label: 'Add to queue', icon: 'plus', onClick: () => enqueue(c.tracks) },
      c.artist ? { label: 'Go to artist', icon: 'user', onClick: () => openArtist(c.artist, null, { browseId: c.artist_browse }) } : null,
      sourceUrl ? { label: 'Copy link', icon: 'link', onClick: () => copyText(sourceUrl) } : null,
    ].filter(Boolean),
  });
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
