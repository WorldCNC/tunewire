'use strict';

// ===========================================================================
// Playlist sync: shared, collaborative and followed playlists, and with
// "Sync my library" every playlist plus Liked Songs on all your devices.
//
// Three-way: for each linked playlist sync.json keeps the songs (keys) and
// details as they were at the last sync, the "base". Local vs base is what
// changed here; it goes out as small operations (syncops.js). Server version
// vs base version says whether anything changed elsewhere; then the server's
// copy (which already includes ours) is taken. Because the base and the
// local list are both on disk, an edit made offline still goes out after a
// restart, and a pull never throws away an edit that hasn't been sent.
//
// Every device has an id that rides along with its changes, so a device
// skips only its own changes, not those from your other devices. All runs
// are serialized (one chain), and changes are applied to local songs only
// if they're still what the change was computed from.
// ===========================================================================

const SYNC_META = 'id,owner,kind,local_id,title,description,folder,pinned,visibility,collaborative,version,deleted_at,cover_md5,created_at,updated_at';
const EMPTY_MD5 = 'd41d8cd98f00b204e9800998ecf8427e';
const COVER_MAX = 400000; // the server's limit on a cover
const cloudMeta = new Map();  // cloud id -> { owner, collaborative, visibility, ownerName, editors }
const syncInfo = new Map();   // local id -> { status }
const watchers = new Map();   // cloud id -> Channel (other people's playlists)
const syncs = { device: '', users: {}, loaded: null, chain: Promise.resolve(), owner: null, ownerFor: null, lastAll: 0, failed: new Set(), offline: false };

const libSyncOn = () => signedIn() && !!state.settings?.sync_library;

// Saved state ------------------------------------------------------------------------------
function syncLoad() {
  if (!syncs.loaded) {
    syncs.loaded = (async () => {
      let saved = null;
      try { saved = JSON.parse((await call('load_sync_state')) || 'null'); } catch { saved = null; }
      syncs.users = saved?.users && typeof saved.users === 'object' ? saved.users : {};
      syncs.device = typeof saved?.device === 'string' && /^[0-9a-f]{8,64}$/.test(saved.device) ? saved.device : randomHex(12);
      // What we knew about each playlist's sharing, before the first fetch.
      for (const u of Object.values(syncs.users)) {
        for (const [id, b] of Object.entries(u.pl || {})) if (!cloudMeta.has(id)) cloudMeta.set(id, { owner: b.owner, visibility: b.visibility, collaborative: b.collaborative });
      }
      if (!saved?.device) syncSave();
    })();
  }
  return syncs.loaded;
}
function randomHex(n) {
  const b = new Uint8Array(n);
  crypto.getRandomValues(b);
  return [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
}
let syncSaveTimer = null;
function syncSave() {
  clearTimeout(syncSaveTimer);
  syncSaveTimer = setTimeout(() => {
    call('save_sync_state', { data: JSON.stringify({ device: syncs.device, users: syncs.users }) }).catch((e) => console.error('sync state', e));
  }, 400);
}
/** This account's bases: { pl: { cloudId: base }, liked: cloudId|null }. */
function bases() {
  const u = (syncs.users[me()] ||= { pl: {}, liked: null });
  u.pl ||= {};
  return u;
}

// Serialized runs ---------------------------------------------------------------------------
/** Run `fn` after every sync step before it; resolves/rejects with its result. */
function syncTask(fn) {
  const p = syncs.chain.then(async () => { await syncLoad(); return fn(); });
  syncs.chain = p.catch(() => {});
  return p;
}
const syncLog = (e) => console.error('sync', e);

/** Everything: list the server's rows, then reconcile, discover and upload. */
function syncSoon(minGap = 20000) {
  if (!signedIn() || Date.now() - syncs.lastAll < minGap) return;
  syncs.lastAll = Date.now();
  syncTask(syncAll).catch(syncLog);
}
let localTimer = null;
/** Only what changed here (after an edit): no listing. */
function syncLocalSoon(delay = 500) {
  if (!signedIn()) return;
  clearTimeout(localTimer);
  localTimer = setTimeout(() => syncTask(syncLocal).catch(syncLog), delay);
}
/** Only rows changed since the last pass (the minute-by-minute safety net). */
function syncChangedSoon() {
  if (!signedIn()) return;
  if (Date.now() - syncs.lastAll > 10 * 60000 || !bases().since) { syncSoon(0); return; }
  syncTask(syncChanged).catch(syncLog);
}
/** Remember the newest change seen (server time, compared as text: same format). */
function noteSince(rows) {
  const U = bases();
  for (const r of rows) if (r?.updated_at && (!U.since || r.updated_at > U.since)) U.since = r.updated_at;
}
const oneTimers = new Map();
function syncOneSoon(cloudId, delay = 300) {
  clearTimeout(oneTimers.get(cloudId));
  oneTimers.set(cloudId, setTimeout(() => { oneTimers.delete(cloudId); syncTask(() => syncOne(cloudId)).catch(syncLog); }, delay));
}

// Local side --------------------------------------------------------------------------------
async function readLibrary() {
  const [playlists, favorites] = await Promise.all([call('list_playlists'), call('list_favorites')]);
  return { playlists, favorites };
}
/** Linked playlists here: { cloudId, localId, role, kind }. */
function localLinks(lib) {
  const out = lib.playlists
    .filter((p) => p.cloud_id && UUID_RE.test(p.cloud_id))
    .map((p) => ({ cloudId: p.cloud_id, localId: p.id, role: p.cloud_role || 'follower', kind: 'playlist' }));
  const liked = bases().liked;
  if (liked && libSyncOn()) out.push({ cloudId: liked, localId: 'liked', role: 'owner', kind: 'liked' });
  return out;
}
function localOf(link, lib) {
  if (link.kind === 'liked') return { tracks: lib.favorites, name: 'Liked Songs', description: '', folder: null, pinned: false, cover: null };
  return lib.playlists.find((p) => p.id === link.localId) || null;
}
/** Put songs from the server here, if the local list is still `expect`. */
function setLocalTracks(link, tracks, added, expect) {
  if (link.kind === 'liked') return call('replace_favorites', { tracks, expect });
  return call('replace_playlist_tracks', { id: link.localId, tracks, added, expect });
}
const addedOf = (cloudTracks) => Object.fromEntries((cloudTracks || []).filter((t) => t?._at).map((t) => [t.key, t._at]));
const baseFromRow = (row, extra) => ({
  v: row.version, owner: row.owner, kind: row.kind || 'playlist', visibility: row.visibility, collaborative: !!row.collaborative,
  name: row.title, description: row.description || '', folder: row.folder || null, pinned: !!row.pinned, coverMd5: row.cover_md5 ?? EMPTY_MD5, ...extra,
});

function rememberRow(row) {
  const prev = cloudMeta.get(row.id) || {};
  cloudMeta.set(row.id, { ...prev, owner: row.owner, collaborative: row.collaborative, visibility: row.visibility, ownerName: row.profiles?.name || prev.ownerName });
}
/** Linked to an online copy that other people can open (not just your private sync copy). */
function isShared(pl) {
  if (!pl?.cloud_id) return false;
  const m = cloudMeta.get(pl.cloud_id);
  return pl.cloud_role !== 'owner' || !m || m.visibility !== 'private' || !!m.collaborative;
}

// Server side -------------------------------------------------------------------------------
/** Rows (meta only) by id, in URL-sized batches. */
async function fetchRows(ids) {
  const out = new Map();
  for (let i = 0; i < ids.length; i += 80) {
    const chunk = ids.slice(i, i + 80);
    const rows = await db('GET', `playlists?id=in.(${chunk.join(',')})&select=${SYNC_META},profiles!playlists_owner_fkey(name)`);
    for (const r of rows || []) out.set(r.id, r);
  }
  return out;
}
/** All of your own rows (paged: the API returns at most 1000 at a time). */
async function fetchOwnRows() {
  const out = [];
  for (let offset = 0; ; offset += 1000) {
    const rows = await db('GET', `playlists?owner=eq.${me()}&select=${SYNC_META}&order=id&limit=1000&offset=${offset}`);
    out.push(...(rows || []));
    if (!rows || rows.length < 1000) return out;
  }
}
/** Songs (and the cover) of one playlist. */
async function fetchSongs(cloudId, withCover = false) {
  const [r] = await db('GET', `playlists?id=eq.${cloudId}&select=id,version,tracks,cover_md5${withCover ? ',cover' : ''}`);
  return r || null;
}
const op = (cloudId, o) => rpc('playlist_op', { pid: cloudId, op: { ...o, client: syncs.device } });
const patchRow = (cloudId, patch) => db('PATCH', `playlists?id=eq.${cloudId}&select=${SYNC_META}`, { ...patch, last_client: syncs.device }, { prefer: 'return=representation' }).then((r) => r?.[0] || null);

// Status ----------------------------------------------------------------------------------
function setSync(localId, status) {
  syncInfo.set(localId, { status });
  if (status === 'error') syncs.failed.add(localId); else syncs.failed.delete(localId);
  paintSyncBadges(localId);
}
const SYNC_TEXT = { synced: 'Synced', syncing: 'Syncing…', offline: 'Offline · will sync', error: 'Couldn’t sync' };
function syncBadge(pl) {
  if (!pl?.cloud_id || !cloudOn()) return null;
  const el = h('span', { class: 'sync-badge', dataset: { pl: pl.id }, role: 'status' });
  paintSyncBadge(el, pl);
  return el;
}
function paintSyncBadge(el, pl) {
  const info = syncInfo.get(pl.id) || { status: signedIn() ? 'synced' : 'offline' };
  el.className = `sync-badge ${info.status}`;
  const meta = cloudMeta.get(pl.cloud_id) || {};
  const people = (meta.editors || []).slice(0, 4);
  el.replaceChildren(...[
    h('span', { class: 'sync-dot', 'aria-hidden': 'true' }),
    h('span', {}, SYNC_TEXT[info.status] || 'Synced'),
    info.status === 'error' ? h('button', { class: 'link-btn', onclick: (e) => { e.stopPropagation(); retryFailed(); } }, 'Retry') : null,
    people.length ? h('span', { class: 'avatar-stack', title: people.map((p) => p.name).join(', ') }, ...people.map((p) => avatarEl(p.avatar, p.name, 'avatar xs'))) : null].filter(Boolean));
}
function paintSyncBadges(localId) {
  const pl = state.playlists.find((p) => p.id === localId);
  if (!pl) return;
  for (const el of $$(`.sync-badge[data-pl="${CSS.escape(localId)}"]`)) paintSyncBadge(el, pl);
}
function retryFailed() { syncSoon(0); }

let failToast = null;
let lastReport = '';
/** One message per run, not one per playlist, and only when it's news. */
function reportRun() {
  const n = syncs.failed.size;
  const now = `${syncs.offline}|${n}`;
  if (now === lastReport) return;
  lastReport = now;
  failToast?.close();
  failToast = null;
  if (syncs.offline) {
    failToast = toast('Offline: your playlist changes will sync when you’re back online', 'info', 6000);
  } else if (n) {
    failToast = toast(`${n} playlist${n === 1 ? '' : 's'} didn’t sync`, 'warn', 10000, { action: { label: 'Retry', onClick: retryFailed } });
  }
}
window.addEventListener('online', () => syncSoon(0));

/** Reconcile one playlist; never throws (marks it offline/error instead). */
async function runOne(link, row, lib) {
  try {
    await reconcile(link, row, lib);
    if (link.kind !== 'liked') setSync(link.localId, 'synced');
  } catch (e) {
    const offline = e?.kind === 'network' || !navigator.onLine;
    if (offline) syncs.offline = true;
    console.warn('sync', link.kind, link.cloudId, e?.message || e);
    if (link.kind !== 'liked') setSync(link.localId, offline ? 'offline' : 'error');
    else if (!offline) syncs.failed.add('liked');
  }
}

// Reconciling one playlist -------------------------------------------------------------------
/**
 * Bring one linked playlist and its online copy together. `row` is the
 * server's row (details + version) if we just fetched it; without it only
 * local changes are sent (and the server's copy taken if that shows that
 * someone else changed it too).
 */
async function reconcile(link, row, lib) {
  const U = bases();
  const owner = link.role === 'owner';
  let base = U.pl[link.cloudId];
  const local = localOf(link, lib);
  if (!local) return;
  if (row) rememberRow(row);

  if (row?.deleted_at) {
    // Deleted on another device…
    if (base) { await deletedElsewhere(link, local); return; }
    // …or deleted here, then brought back (Undo, a backup): bring it back online too.
    row = (await patchRow(link.cloudId, { deleted_at: null })) || { ...row, deleted_at: null };
  }

  const meta = cloudMeta.get(link.cloudId) || {};
  const canPush = owner || (link.role === 'editor' && meta.collaborative !== false);
  const rawKeys = local.tracks.map((t) => t.key);
  const snap = uniqKeys(local.tracks);
  let server = null; // { version, tracks, cover? } once fetched

  // First sync of this link here: songs on both sides are kept (nothing is
  // treated as removed), and the server's order and details win.
  const first = !base;
  if (first) {
    server = await fetchSongs(link.cloudId);
    if (!server) throw { kind: 'storage', message: 'playlist not found' };
    const here = new Set(snap);
    const r = row || { version: server.version, owner: meta.owner, visibility: meta.visibility, collaborative: meta.collaborative, title: local.name, description: local.description, folder: local.folder, pinned: local.pinned, cover_md5: server.cover_md5 };
    base = baseFromRow(r, {
      v: server.version, keys: uniqKeys(server.tracks).filter((k) => here.has(k)), role: link.role,
      // Details: the server's copy wins on a first sync (as "changed there").
      ...(owner ? { name: local.name, description: local.description || '', folder: local.folder || null, pinned: !!local.pinned } : {}),
      coverSig: coverSig(local.cover), coverMd5: owner ? server.cover_md5 : (local.cover ? null : EMPTY_MD5),
    });
  }

  // 1. Changes made here go out.
  const ops = canPush ? planOps(base.keys, local.tracks, slimTrack, { keepOrder: first }) : [];
  let version = base.v;
  let refused = false;
  if (ops.length) setSync(link.localId, 'syncing');
  for (const o of ops) {
    try {
      version = (await op(link.cloudId, o)).version;
    } catch (e) {
      if (e?.kind === 'network' || owner) throw e;
      refused = true; // not an editor anymore: take the server's copy
      break;
    }
  }
  const patched = new Set();
  if (owner && link.kind === 'playlist') {
    const patch = {};
    if (local.name !== base.name) patch.title = local.name;
    if ((local.description || '') !== (base.description || '')) patch.description = local.description || '';
    if ((local.folder || null) !== (base.folder || null)) patch.folder = local.folder || null;
    if (!!local.pinned !== !!base.pinned) patch.pinned = !!local.pinned;
    const sig = coverSig(local.cover);
    if (sig !== base.coverSig && (local.cover || '').length < COVER_MAX) patch.cover = local.cover || null;
    if (Object.keys(patch).length) {
      const r = await patchRow(link.cloudId, patch);
      if (r) rememberRow(r);
      if ('title' in patch) base.name = local.name;
      if ('description' in patch) base.description = local.description || '';
      if ('folder' in patch) base.folder = local.folder || null;
      if ('pinned' in patch) base.pinned = !!local.pinned;
      if ('cover' in patch) base.coverMd5 = r?.cover_md5 ?? base.coverMd5;
      for (const k of Object.keys(patch)) patched.add(k);
    }
    base.coverSig = sig;
  }

  // 2. Changes made elsewhere come in.
  const expected = base.v + ops.length;
  const changedThere = first
    ? !sameKeys(uniqKeys(server.tracks), snap)
    : refused || base.v < 0 || (ops.length ? version !== expected : row != null && row.version !== base.v);
  // Followers (and editors whose edits were refused) always match the server.
  const forced = (!canPush || refused) && !sameKeys(snap, base.keys);
  if (changedThere || forced) {
    if (!server || ops.length) server = await fetchSongs(link.cloudId);
    if (!server) throw { kind: 'storage', message: 'playlist not found' };
    const tracks = (server.tracks || []).map(fromCloudTrack);
    const keys = uniqKeys(tracks);
    if (!sameKeys(keys, snap)) {
      setSync(link.localId, 'syncing');
      try {
        await setLocalTracks(link, tracks, addedOf(server.tracks), rawKeys);
      } catch (e) {
        if (e?.kind !== 'conflict') throw e;
        // Edited here while we were busy: send that next time, then take theirs.
        U.pl[link.cloudId] = { ...base, v: -1, keys: snap };
        syncSave();
        syncLocalSoon(300);
        return;
      }
    }
    base.keys = keys;
    base.v = server.version;
  } else {
    base.keys = snap;
    base.v = version;
  }

  // Details: theirs where they changed (owner: where we didn't change them too).
  if (row && link.kind === 'playlist') {
    const fields = [['title', 'name', ''], ['description', 'description', '']];
    if (owner) fields.push(['folder', 'folder', null], ['pinned', 'pinned', false]);
    const lpatch = {};
    for (const [col, key, empty] of fields) {
      if (patched.has(col)) continue;
      const remote = row[col] ?? empty;
      const theirs = owner ? remote !== (base[key] ?? empty) : remote !== (local[key] ?? empty);
      if (theirs && remote !== (local[key] ?? empty)) lpatch[key] = key === 'folder' ? remote || '' : remote;
      base[key] = remote;
    }
    const coverChanged = !patched.has('cover') && (row.cover_md5 ?? EMPTY_MD5) !== base.coverMd5;
    if (coverChanged) {
      const r = await fetchSongs(link.cloudId, true);
      lpatch.cover = r?.cover || '';
      base.coverMd5 = r?.cover_md5 ?? row.cover_md5;
      base.coverSig = coverSig(r?.cover || null);
    }
    if (Object.keys(lpatch).length) await call('update_playlist_meta', { id: link.localId, patch: lpatch });
    Object.assign(base, { owner: row.owner, visibility: row.visibility, collaborative: !!row.collaborative });
  }
  base.role = link.role;
  base.kind = link.kind;
  U.pl[link.cloudId] = base;
  syncSave();
}

/** The online copy was deleted on another device (or by its owner). */
async function deletedElsewhere(link, local) {
  delete bases().pl[link.cloudId];
  syncSave();
  if (link.kind === 'liked') { bases().liked = null; return; }
  await call('delete_playlist', { id: link.localId });
  const snapshot = { ...local, cloud_id: null, cloud_role: null };
  undoToast(`“${local.name}” was deleted on another device`, () => call('restore_playlist', { playlist: snapshot, index: null }));
}

/** Deleted here: tombstone it (yours) or leave it (someone else's). */
async function deletedHere(cloudId, base) {
  if ((base.role || 'owner') === 'owner') await patchRow(cloudId, { deleted_at: new Date().toISOString() });
  else await db('DELETE', `playlist_members?playlist_id=eq.${cloudId}&user_id=eq.${me()}`);
  delete bases().pl[cloudId];
  syncSave();
}

/**
 * Bases whose playlist is gone here. If many vanish at once (a damaged or
 * replaced library), nothing is deleted online without asking.
 */
async function handleGoneHere(gone) {
  const total = Object.keys(bases().pl).length;
  if (gone.length > 3 && gone.length > total / 4) {
    if (syncs.askedGone) return;
    syncs.askedGone = true;
    toast(`${gone.length} synced playlists are missing on this computer. They’re kept online.`, 'warn', 15000, { action: { label: 'Get them back', onClick: () => {
      for (const [id] of gone) delete bases().pl[id];
      syncSave();
      syncs.askedGone = false;
      syncSoon(0);
    } } });
    return;
  }
  for (const [id, base] of gone) {
    try { await deletedHere(id, base); } catch (e) { if (e?.kind === 'network') syncs.offline = true; else console.warn('sync delete', e?.message || e); }
  }
}

/** Make a local playlist from one of your rows (created on another device). */
async function createHere(row) {
  const full = await fetchSongs(row.id, true);
  if (!full) return;
  const tracks = (full.tracks || []).map(fromCloudTrack);
  const now = Date.now();
  const pl = {
    id: `pl${now.toString(16)}${randomHex(3)}`, name: row.title, tracks, source_url: null,
    created: Date.parse(row.created_at) || now, updated: now, description: row.description || '',
    cover: full.cover || null, pinned: !!row.pinned, folder: row.folder || null, added: addedOf(full.tracks),
    cloud_id: row.id, cloud_role: 'owner',
  };
  await call('restore_playlist', { playlist: pl, index: null });
  bases().pl[row.id] = baseFromRow(row, { v: full.version, keys: uniqKeys(tracks), role: 'owner', coverSig: coverSig(pl.cover), coverMd5: full.cover_md5 });
  syncSave();
}

/** Put a local playlist online (private sync copy, or shared). */
async function uploadPlaylist(pl, { visibility = 'private', collaborative = false } = {}) {
  const body = {
    title: pl.name, description: pl.description || '', cover: pl.cover && pl.cover.length < COVER_MAX ? pl.cover : null,
    folder: pl.folder || null, pinned: !!pl.pinned, visibility, collaborative, local_id: pl.id,
    tracks: uniqTracks(pl.tracks).map(slimTrack), last_client: syncs.device,
  };
  let row;
  try {
    [row] = await db('POST', `playlists?select=${SYNC_META}`, body, { prefer: 'return=representation' });
  } catch (e) {
    if (e?.status !== 409) throw e;
    // Already up (the answer to an earlier upload got lost): use that one.
    [row] = await db('GET', `playlists?owner=eq.${me()}&local_id=eq.${encodeURIComponent(pl.id)}&deleted_at=is.null&select=${SYNC_META}`);
    if (!row) throw e;
  }
  rememberRow(row);
  pl.cloud_id = row.id;
  pl.cloud_role = 'owner';
  const live = state.playlists.find((p) => p.id === pl.id);
  if (live) Object.assign(live, { cloud_id: row.id, cloud_role: 'owner' });
  await call('set_playlist_cloud', { id: pl.id, cloudId: row.id, role: 'owner' });
  const fresh = !row.local_id || row.version === 1;
  // A row we found again may differ: no base, so the next sync merges.
  if (fresh) bases().pl[row.id] = baseFromRow(row, { keys: uniqKeys(pl.tracks), role: 'owner', name: pl.name, description: pl.description || '', folder: pl.folder || null, pinned: !!pl.pinned, coverSig: coverSig(body.cover), coverMd5: row.cover_md5 });
  syncSave();
  return row;
}

/** Liked Songs: find or make your one online copy. */
async function ensureLiked(own, lib) {
  const U = bases();
  if (U.liked && own.some((r) => r.id === U.liked && !r.deleted_at)) return;
  if (U.liked) { delete U.pl[U.liked]; U.liked = null; }
  const row = own.find((r) => r.kind === 'liked' && !r.deleted_at);
  if (row) {
    U.liked = row.id;
    await runOne({ cloudId: row.id, localId: 'liked', role: 'owner', kind: 'liked' }, row, lib);
    return;
  }
  const [r] = await db('POST', `playlists?select=${SYNC_META}`, { title: 'Liked Songs', kind: 'liked', visibility: 'private', tracks: uniqTracks(lib.favorites).map(slimTrack), last_client: syncs.device }, { prefer: 'return=representation' });
  U.liked = r.id;
  U.pl[r.id] = baseFromRow(r, { keys: uniqKeys(lib.favorites), role: 'owner', kind: 'liked' });
  syncSave();
}

// Passes ----------------------------------------------------------------------------------
async function syncAll() {
  if (!signedIn()) { stopWatching(); return; }
  const U = bases();
  const on = libSyncOn();
  syncs.offline = false;
  let lib, own = [], rows;
  try {
    lib = await readLibrary();
    if (on) own = await fetchOwnRows();
    const links = localLinks(lib);
    const ownIds = new Set(own.map((r) => r.id));
    rows = await fetchRows(links.map((l) => l.cloudId).filter((id) => !ownIds.has(id)));
    for (const r of own) rows.set(r.id, r);
    noteSince(rows.values());
  } catch (e) {
    if (e?.kind === 'network') {
      syncs.offline = true;
      for (const p of state.playlists) if (p.cloud_id) setSync(p.id, 'offline');
    }
    reportRun();
    return;
  }
  const links = localLinks(lib);
  for (const link of links) {
    const row = rows.get(link.cloudId);
    if (otherAccounts(link, row)) continue;
    if (!row) {
      // Not readable anymore (unshared, deleted for everyone).
      await lostLink(link, lib);
      continue;
    }
    await runOne(link, row, lib);
  }
  const linked = new Set(links.map((l) => l.cloudId));
  const gone = Object.entries(U.pl).filter(([id, b]) => !linked.has(id) && b.kind !== 'liked' && !rows.get(id)?.deleted_at);
  for (const [id, b] of Object.entries(U.pl)) if (!linked.has(id) && rows.get(id)?.deleted_at && b.kind !== 'liked') delete U.pl[id];
  await handleGoneHere(gone);

  if (on) {
    for (const row of own) {
      if (row.kind === 'liked' || row.deleted_at || linked.has(row.id) || U.pl[row.id]) continue;
      const mine = row.local_id && lib.playlists.find((p) => p.id === row.local_id && !p.cloud_id);
      try {
        if (mine) {
          await call('set_playlist_cloud', { id: mine.id, cloudId: row.id, role: 'owner' });
          mine.cloud_id = row.id;
          mine.cloud_role = 'owner';
          await runOne({ cloudId: row.id, localId: mine.id, role: 'owner', kind: 'playlist' }, row, lib);
        } else {
          await createHere(row);
        }
      } catch (e) { if (e?.kind === 'network') syncs.offline = true; else console.warn('sync create', e?.message || e); }
    }
    if (mergeAllowed()) {
      await uploadNew(lib);
      if (U.merge !== false) {
        try { await ensureLiked(own, lib); } catch (e) { if (e?.kind === 'network') syncs.offline = true; else syncs.failed.add('liked'); }
      }
    }
  }
  watchLinks(links);
  syncSave();
  reportRun();
}

async function syncChanged() {
  if (!signedIn()) return;
  const U = bases();
  const since = encodeURIComponent(U.since);
  let changed;
  try {
    const own = libSyncOn() ? await db('GET', `playlists?owner=eq.${me()}&updated_at=gt.${since}&select=${SYNC_META}&order=updated_at&limit=1000`) : [];
    const lib0 = await readLibrary();
    const others = localLinks(lib0).filter((l) => l.role !== 'owner').map((l) => l.cloudId);
    const theirs = [];
    for (let i = 0; i < others.length; i += 80) {
      theirs.push(...(await db('GET', `playlists?id=in.(${others.slice(i, i + 80).join(',')})&updated_at=gt.${since}&select=${SYNC_META},profiles!playlists_owner_fkey(name)`)));
    }
    changed = [...own, ...theirs];
  } catch (e) {
    if (e?.kind === 'network') { syncs.offline = true; reportRun(); }
    return;
  }
  if (!changed.length) return;
  const lib = await readLibrary();
  const links = localLinks(lib);
  for (const row of changed) {
    const link = links.find((l) => l.cloudId === row.id);
    if (link) await runOne(link, row, lib);
    else if (row.owner === me() && !row.deleted_at && !U.pl[row.id]) { syncSoon(0); break; } // new elsewhere
  }
  noteSince(changed);
  syncSave();
  reportRun();
}

/** After local edits: send what changed, pick up deletes and new playlists. */
async function syncLocal() {
  if (!signedIn()) return;
  const U = bases();
  const lib = await readLibrary();
  const links = localLinks(lib);
  syncs.offline = false;
  let needAll = false;
  for (const link of links) {
    const base = U.pl[link.cloudId];
    if (!base && otherAccounts(link, null)) continue;
    if (!base) { needAll = true; continue; }
    const local = localOf(link, lib);
    const pl = link.kind === 'playlist' ? local : null;
    const same = sameKeys(uniqKeys(local.tracks), base.keys) && base.v >= 0 && (!pl || link.role !== 'owner' || (
      pl.name === base.name && (pl.description || '') === (base.description || '') && (pl.folder || null) === (base.folder || null)
      && !!pl.pinned === !!base.pinned && coverSig(pl.cover) === base.coverSig));
    if (!same) await runOne(link, null, lib);
  }
  const linked = new Set(links.map((l) => l.cloudId));
  await handleGoneHere(Object.entries(U.pl).filter(([id, b]) => !linked.has(id) && b.kind !== 'liked'));
  if (libSyncOn() && mergeAllowed()) await uploadNew(lib);
  watchLinks(localLinks(await readLibrary()));
  reportRun();
  if (needAll) syncSoon(0);
}

/**
 * This computer's library already synced with another account: ask before
 * adding it to this one. Until answered, nothing local goes up.
 */
function mergeAllowed() {
  const U = bases();
  if (U.merge != null) return true;
  const other = Object.entries(syncs.users).some(([uid, u]) => uid !== me() && Object.keys(u?.pl || {}).length);
  if (!other) { U.merge = true; syncSave(); return true; }
  if (!syncs.askingMerge) askMerge();
  return false;
}
function askMerge() {
  syncs.askingMerge = true;
  const answer = (merge) => {
    const U = bases();
    U.merge = merge;
    U.mergeAt = Date.now();
    syncSave();
    syncs.askingMerge = false;
    m.close();
    syncSoon(0);
  };
  const m = openModal({
    title: `Add this computer’s playlists to ${cloud.session?.name || 'this account'}?`,
    subtitle: 'This computer’s library is already synced with another account. You can add its playlists and Liked Songs to this account too, or keep them separate: then only playlists you make from now on are added.',
    footer: [h('button', { class: 'btn', onclick: () => answer(false) }, 'Keep separate'), h('button', { class: 'btn primary', onclick: () => answer(true) }, 'Add them')],
    onClose: () => { syncs.askingMerge = false; },
  });
}

/** Playlists made here (sync on): up they go, privately. */
async function uploadNew(lib) {
  const U = bases();
  // "Keep separate": only playlists made after that answer.
  const from = U.merge === false ? U.mergeAt || 0 : 0;
  for (const pl of lib.playlists.filter((p) => !p.cloud_id && (p.created || 0) >= from)) {
    try { await uploadPlaylist(pl); setSync(pl.id, 'synced'); } catch (e) {
      if (e?.kind === 'network') { syncs.offline = true; return; }
      console.warn('sync upload', pl.name, e?.message || e);
      setSync(pl.id, 'error');
    }
  }
}

/** One playlist, right now (a live change, a link just followed). */
async function syncOne(cloudId) {
  if (!signedIn()) return;
  const lib = await readLibrary();
  const link = localLinks(lib).find((l) => l.cloudId === cloudId);
  const [row] = await db('GET', `playlists?id=eq.${cloudId}&select=${SYNC_META},profiles!playlists_owner_fkey(name)`).catch((e) => { if (e?.kind === 'network') syncs.offline = true; return [undefined]; });
  if (syncs.offline) return;
  if (link) {
    if (otherAccounts(link, row)) return;
    if (!row) await lostLink(link, lib);
    else await runOne(link, row, lib);
  } else if (row && row.owner === me() && libSyncOn() && !row.deleted_at && !bases().pl[cloudId]) {
    // New on another device (or Liked Songs appeared): a full pass places it.
    syncSoon(0);
  }
}

/**
 * Linked by another account that signed in on this computer: left alone
 * (signing in with a second account must not unlink or delete the first
 * one's playlists).
 */
function otherAccounts(link, row) {
  if (link.role === 'owner' && row && row.owner !== me()) return true;
  if (row || bases().pl[link.cloudId]) return false;
  return Object.entries(syncs.users).some(([uid, u]) => uid !== me() && u?.pl?.[link.cloudId]);
}

/** A linked copy we can't read anymore. Yours: put it up again (sync on). Theirs: keep a copy. */
async function lostLink(link, lib) {
  const local = localOf(link, lib);
  delete bases().pl[link.cloudId];
  syncSave();
  if (link.kind === 'liked') { bases().liked = null; return; }
  await call('set_playlist_cloud', { id: link.localId, cloudId: null, role: null });
  syncInfo.delete(link.localId);
  if (link.role !== 'owner' && local) toast(`“${local.name}” isn’t shared anymore. Your copy stays in Your Library.`, 'info', 4000);
}

// Live updates ------------------------------------------------------------------------------
function onChangeRow(rec) {
  if (!rec || rec.client === syncs.device) return;
  const base = bases().pl[rec.playlist_id];
  const o = rec.op || {};
  if (base && !o.meta && !o.replace && rec.version === base.v + 1) {
    syncTask(() => fastApply(rec)).catch(syncLog);
    return;
  }
  syncOneSoon(rec.playlist_id);
}
/** A change that follows straight on what we have: apply it without downloading the playlist. */
async function fastApply(rec) {
  const base = bases().pl[rec.playlist_id];
  if (!base || rec.version !== base.v + 1) return syncOne(rec.playlist_id);
  const lib = await readLibrary();
  const link = localLinks(lib).find((l) => l.cloudId === rec.playlist_id);
  const local = link && localOf(link, lib);
  if (!local || !sameKeys(uniqKeys(local.tracks), base.keys)) return syncOne(rec.playlist_id);
  const tracks = applyOp(local.tracks, rec.op, fromCloudTrack);
  if (!tracks) return syncOne(rec.playlist_id);
  try {
    await setLocalTracks(link, tracks, addedOf(rec.op.add), local.tracks.map((t) => t.key));
  } catch (e) {
    if (e?.kind === 'conflict') return syncOne(rec.playlist_id);
    throw e;
  }
  base.keys = uniqKeys(tracks);
  base.v = rec.version;
  syncSave();
}

function watchLinks(links) {
  if (!signedIn()) return stopWatching();
  // Yours (all devices, all editors): one channel.
  if (syncs.ownerFor !== me()) {
    syncs.owner?.close();
    syncs.ownerFor = me();
    const ch = new Channel(`lib:${me()}`, { changes: [{ event: 'INSERT', schema: 'public', table: 'playlist_changes', filter: `owner=eq.${me()}` }] });
    ch.on('change', (d) => onChangeRow(d?.record));
    // Joining takes a moment (and reconnects miss changes): catch up once
    // changes are coming in, after each (re)connect.
    ch.on('status', (s) => { if (s === 'live') syncSoon(0); });
    syncs.owner = ch;
  }
  // Other people's (followed, collaborating): one channel each.
  const want = new Set(links.filter((l) => l.role !== 'owner').map((l) => l.cloudId));
  for (const [id, ch] of watchers) if (!want.has(id)) { ch.close(); watchers.delete(id); }
  for (const id of want) {
    if (watchers.has(id)) continue;
    const ch = new Channel(`pl:${id}`, { changes: [{ event: 'INSERT', schema: 'public', table: 'playlist_changes', filter: `playlist_id=eq.${id}` }] });
    ch.on('change', (d) => onChangeRow(d?.record));
    ch.on('status', (s) => { if (s === 'live' && bases().pl[id]) syncOneSoon(id, 300); });
    watchers.set(id, ch);
    const link = links.find((l) => l.cloudId === id);
    if (link) loadEditors(id, link.localId);
  }
}
function stopWatching() {
  syncs.owner?.close();
  syncs.owner = null;
  syncs.ownerFor = null;
  for (const ch of watchers.values()) ch.close();
  watchers.clear();
}

/** Who edits a collaborative playlist (avatars next to the sync status). */
async function loadEditors(cloudId, localId) {
  const meta = cloudMeta.get(cloudId);
  if (!meta?.collaborative) return;
  try {
    const rows = await db('GET', `playlist_members?playlist_id=eq.${cloudId}&role=eq.editor&select=user_id,profiles(name,avatar)`);
    const [owner] = await db('GET', `profiles?id=eq.${meta.owner}&select=name,avatar`);
    meta.editors = [owner, ...rows.map((r) => r.profiles)].filter(Boolean);
    meta.ownerName = owner?.name || meta.ownerName;
    paintSyncBadges(localId);
  } catch { /* offline */ }
}

// Turning library sync on and off ------------------------------------------------------------
async function setLibrarySync(on) {
  await updateSettings({ sync_library: on, sync_asked: true });
  if (on) {
    toast('Syncing your library…', 'info', 2000);
    syncSoon(0);
    return;
  }
  // This computer stops; your other devices and the online copies stay as they are.
  await syncTask(async () => {
    const U = bases();
    const lib = await readLibrary();
    for (const pl of lib.playlists) {
      if (pl.cloud_role !== 'owner' || !pl.cloud_id || isShared(pl)) continue;
      await call('set_playlist_cloud', { id: pl.id, cloudId: null, role: null });
      delete U.pl[pl.cloud_id];
      syncInfo.delete(pl.id);
    }
    if (U.liked) { delete U.pl[U.liked]; U.liked = null; }
    syncSave();
  }).catch(syncLog);
  toast('This computer stopped syncing. Your other devices keep their playlists.', 'info', 3500);
}

/** Signed in before library sync existed: ask once. */
function askLibrarySync() {
  // New accounts get this question in "What friends can see".
  if (!signedIn() || !state.settings || state.settings.sync_asked || !state.settings.cloud_privacy_asked) return;
  const input = h('input', { type: 'checkbox', checked: true });
  const m = openModal({
    title: 'Your playlists on all your devices?',
    subtitle: 'Tunewire can keep your playlists and Liked Songs in sync on every computer you sign in to.',
    body: h('div', { class: 'privacy-list' },
      h('label', { class: 'setting' },
        h('div', { class: 'label' }, 'Sync my library', h('small', {}, 'Private online copies only you can see. Change it any time in Settings → Account.')),
        h('span', { class: 'switch' }, input, h('span')))),
    footer: [h('button', { class: 'btn primary', onclick: async () => { m.close(); await setLibrarySync(input.checked); } }, 'Continue')],
    onClose: () => { if (!state.settings.sync_asked) updateSettings({ sync_asked: true }); },
  });
}

// When to sync ----------------------------------------------------------------------------
listen('library-changed', (e) => {
  const what = typeof e.payload === 'string' ? e.payload : '';
  if (what === 'playlists' || what === 'favorites') syncLocalSoon();
});
onCloudChange(() => {
  if (!signedIn()) { stopWatching(); return; }
  syncSoon(0);
  setTimeout(askLibrarySync, 1500);
});
// Safety net for missed live updates: what changed, every minute and when the
// window comes back (a full pass at most every 10 minutes).
// The safety net: every minute, or every 15 s while live updates are down
// (Realtime unreachable or still warming up).
let changedAt = 0;
setInterval(() => {
  if (Date.now() - changedAt < (syncs.owner?.live ? 59000 : 14000)) return;
  changedAt = Date.now();
  syncChangedSoon();
}, 15000);
window.addEventListener('focus', syncChangedSoon);
