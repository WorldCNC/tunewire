'use strict';

// ===========================================================================
// Social: account, sharing, shared & collaborative playlists, Listen
// Together, friends' activity, profiles, comparing Wrapped.
// ===========================================================================

// Account ---------------------------------------------------------------------------
function avatarEl(src, name, cls = 'avatar') {
  const el = h('span', { class: cls, title: name || '' });
  if (src) el.append(h('img', { src, alt: '', 'data-px': 96, referrerpolicy: 'no-referrer', onerror: (e) => { e.target.remove(); el.textContent = (name || '?')[0].toUpperCase(); } }));
  else el.textContent = (name || '?')[0].toUpperCase();
  return el;
}

function paintAccountButton() {
  const b = $('#nav-account');
  if (!b) return;
  b.hidden = !cloudOn();
  b.replaceChildren(signedIn() ? avatarEl(cloud.session.avatar, cloud.session.name, 'avatar tb-avatar') : h('span', { html: icon('user') }));
  b.title = signedIn() ? cloud.session.name : 'Sign in';
}
onCloudChange(paintAccountButton);
onCloudChange(() => { if (state.view === 'settings') renderSettings(); syncCloudPlaylists(); renderFriends(); });

$('#nav-account')?.addEventListener('click', (e) => {
  const anchor = e.currentTarget;
  if (!signedIn()) return requireAccount({ why: 'Sign in to Tunewire' });
  openMenu(anchor, [
    { label: cloud.session.anonymous ? 'Guest' : cloud.session.name, heading: true },
    cloud.session.anonymous ? { label: 'Sign in with an account', icon: 'user', onClick: () => signOut().then(() => requireAccount({ why: 'Sign in to Tunewire' })) } : { label: 'Your profile', icon: 'user', onClick: () => openProfile(cloud.session.user_id) },
    { label: 'Friends', icon: 'friends', onClick: () => { setPanelOpen(true); showPanel('friends'); } },
    { label: 'Listen together', icon: 'friends', onClick: () => startJam() },
    'sep',
    { label: 'Sign out', icon: 'x', onClick: () => signOut() },
  ]);
});

async function signOut() {
  jam.channel && leaveJam();
  await call('cloud_sign_out').catch(() => {});
  cloud.session = null;
  cloudChanged();
  toast('Signed out', 'info', 1500);
}

function accountSettingsRows() {
  if (!cloudOn()) return [settingRow('Account', 'Online features (sharing, Listen Together, friends) aren’t available in this build.', h('span'))];
  if (!signedIn()) {
    return [settingRow('Account', 'Sign in to share playlists, make collaborative playlists, listen together and see what friends play. Everything else works without an account.',
      h('button', { class: 'btn small', onclick: () => requireAccount({ why: 'Sign in to Tunewire' }) }, 'Sign in'))];
  }
  const s = cloud.session;
  const share = h('input', { type: 'checkbox', checked: state.settings.share_activity !== false });
  share.addEventListener('change', async () => {
    await updateSettings({ share_activity: share.checked });
    db('PATCH', `profiles?id=eq.${s.user_id}`, { share_activity: share.checked }).catch(() => {});
    if (!share.checked) db('DELETE', `now_playing?user_id=eq.${s.user_id}`).catch(() => {});
  });
  return [
    settingRow(h('span', { style: 'display:flex;align-items:center;gap:10px' }, avatarEl(s.avatar, s.name), s.anonymous ? 'Guest account' : s.name), s.anonymous ? 'A guest account works for joining sessions. Sign in to keep shared playlists and friends.' : 'Signed in.',
      h('div', { class: 'actions' }, s.anonymous ? null : h('button', { class: 'btn small', onclick: () => openProfile(s.user_id) }, 'Profile'), h('button', { class: 'btn small', onclick: signOut }, 'Sign out'))),
    h('div', { class: 'setting' }, h('div', { class: 'label' }, 'Share what I’m listening to', h('small', {}, 'People who follow you see the song you’re playing in their Friends panel.')), h('label', { class: 'switch' }, share, h('span'))),
  ];
}

// Sharing ---------------------------------------------------------------------------
function shareSong(track) {
  copyText(songLink(track));
  toast('Song link copied. Anyone can open it, even without Tunewire.', 'ok', 2500);
}

const PL_LINK = (id) => `${SHARE_BASE}#p=${id}`;
const INVITE_LINK = (code) => `${SHARE_BASE}#c=${code}`;
const JAM_LINK = (code) => `${SHARE_BASE}#j=${code}`;
const PROFILE_LINK = (id) => `${SHARE_BASE}#u=${id}`;

/** Publish (or update) a local playlist online and return its id. */
async function publishPlaylist(pl, { visibility, collaborative } = {}) {
  if (!(await requireAccount({ why: 'Sign in to share playlists' }))) return null;
  if (cloud.session.anonymous) { toast('Sign in with Discord or Google to share playlists', 'warn'); return null; }
  const meta = { title: pl.name, description: pl.description || '', cover: pl.cover || null };
  if (visibility) meta.visibility = visibility;
  if (collaborative !== undefined) meta.collaborative = collaborative;
  if (pl.cloud_id && pl.cloud_role === 'owner') {
    await db('PATCH', `playlists?id=eq.${pl.cloud_id}`, meta);
    await rpc('playlist_op', { pid: pl.cloud_id, op: { replace: pl.tracks.map(slimTrack) } });
    pushedSig.set(pl.id, playlistSig(pl));
    return pl.cloud_id;
  }
  const [row] = await db('POST', 'playlists', { ...meta, visibility: visibility || 'unlisted', collaborative: !!collaborative, tracks: pl.tracks.map(slimTrack) }, { prefer: 'return=representation' });
  await call('set_playlist_cloud', { id: pl.id, cloudId: row.id, role: 'owner' });
  pushedSig.set(pl.id, playlistSig(pl));
  return row.id;
}

async function sharePlaylist(pl) {
  try {
    const id = await publishPlaylist(pl);
    if (!id) return;
    copyText(PL_LINK(id));
    toast(`Link to “${pl.name}” copied. Changes you make are shared automatically.`, 'ok', 3500);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function makeCollaborative(pl) {
  try {
    const id = await publishPlaylist(pl, { collaborative: true });
    if (!id) return;
    const [row] = await db('GET', `playlists?id=eq.${id}&select=invite_code`);
    copyText(INVITE_LINK(row.invite_code));
    toast('Invite link copied. Friends who open it can add, remove and reorder songs with you.', 'ok', 4500);
    watchPlaylist(pl.id, id);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function setVisibility(pl, visibility) {
  try { await publishPlaylist(pl, { visibility }); toast({ public: 'Shown on your profile', unlisted: 'Anyone with the link can open it', private: 'Only you and collaborators can open it' }[visibility], 'ok', 2200); } catch (e) { toast(errMsg(e), 'error'); }
}

async function unsharePlaylist(pl) {
  if (!(await confirmModal({ title: 'Stop sharing?', message: 'The link stops working and collaborators lose access. Your playlist stays in Your Library.', okLabel: 'Stop sharing', danger: true }))) return;
  try {
    await db('DELETE', `playlists?id=eq.${pl.cloud_id}`);
    await call('set_playlist_cloud', { id: pl.id, cloudId: null, role: null });
    toast('No longer shared', 'info', 1800);
  } catch (e) { toast(errMsg(e), 'error'); }
}

/** Extra items for a playlist's ⋯ menu. */
function shareMenuItems(pl) {
  if (!cloudOn()) return [];
  const linked = pl.cloud_id;
  if (linked && pl.cloud_role !== 'owner') {
    return ['sep', { label: 'Copy link', icon: 'link', onClick: () => { copyText(PL_LINK(pl.cloud_id)); toast('Link copied', 'ok', 1500); } },
      { label: pl.cloud_role === 'editor' ? 'Leave collaborative playlist' : 'Unfollow', icon: 'x', onClick: () => leaveCloudPlaylist(pl) }];
  }
  const meta = cloudMeta.get(pl.cloud_id) || {};
  return ['sep',
    { label: linked ? 'Copy link' : 'Share link', icon: 'link', onClick: () => sharePlaylist(pl) },
    { label: meta.collaborative ? 'Copy invite link' : 'Make collaborative', icon: 'friends', onClick: () => makeCollaborative(pl) },
    linked ? { label: 'Who can open it', heading: true } : null,
    linked ? { label: `${meta.visibility === 'public' ? '✓ ' : ''}Public (on your profile)`, onClick: () => setVisibility(pl, 'public') } : null,
    linked ? { label: `${!meta.visibility || meta.visibility === 'unlisted' ? '✓ ' : ''}Anyone with the link`, onClick: () => setVisibility(pl, 'unlisted') } : null,
    linked ? { label: `${meta.visibility === 'private' ? '✓ ' : ''}Only collaborators`, onClick: () => setVisibility(pl, 'private') } : null,
    linked ? { label: 'Stop sharing', icon: 'x', onClick: () => unsharePlaylist(pl) } : null,
  ].filter(Boolean);
}

// Keeping linked playlists in sync ---------------------------------------------------------
const cloudMeta = new Map();    // cloud id -> row meta (owner, collaborative, visibility, ownerName)
const pushedSig = new Map();    // local id -> signature last pushed (owner)
const watchers = new Map();     // local id -> Channel
const playlistSig = (pl) => `${pl.name}|${pl.description || ''}|${(pl.cover || '').length}|${pl.tracks.map((t) => t.key).join(',')}`;

// Edits made here: owners push the whole list (debounced); collaborators send
// precise operations so nobody overwrites anyone.
commandHooks.push((cmd, args, result, before) => {
  if (!signedIn() || !args?.id) return;
  const pl = state.playlists.find((p) => p.id === args.id);
  if (!pl?.cloud_id || pl.cloud_role === 'follower') return;
  const meta = cloudMeta.get(pl.cloud_id) || {};
  const ops = [];
  if (cmd === 'add_to_playlist') ops.push({ add: args.tracks.map(slimTrack) });
  if (cmd === 'remove_from_playlist' && before?.[args.index]) ops.push({ remove: [before[args.index].key] });
  if (cmd === 'remove_many_from_playlist' && before) ops.push({ remove: args.indices.map((i) => before[i]?.key).filter(Boolean) });
  if (cmd === 'move_many_in_playlist' && before) ops.push({ move: args.indices.map((i) => before[i]?.key).filter(Boolean), to: args.to });
  if (cmd === 'move_in_playlist' && before?.[args.from]) ops.push({ move: [before[args.from].key], to: args.to });
  if (pl.cloud_role === 'editor' || meta.collaborative) {
    for (const op of ops) rpc('playlist_op', { pid: pl.cloud_id, op }).catch((e) => toast(`Couldn’t sync: ${errMsg(e)}`, 'warn'));
    if (cmd === 'update_playlist_meta' && pl.cloud_role === 'owner') schedulePush(pl.id);
  } else if (pl.cloud_role === 'owner') {
    schedulePush(pl.id);
  }
});

const pushTimers = new Map();
function schedulePush(localId) {
  clearTimeout(pushTimers.get(localId));
  pushTimers.set(localId, setTimeout(async () => {
    const pl = state.playlists.find((p) => p.id === localId);
    if (!pl?.cloud_id || pl.cloud_role !== 'owner' || pushedSig.get(pl.id) === playlistSig(pl)) return;
    try {
      await db('PATCH', `playlists?id=eq.${pl.cloud_id}`, { title: pl.name, description: pl.description || '', cover: pl.cover || null });
      if (!(cloudMeta.get(pl.cloud_id) || {}).collaborative) await rpc('playlist_op', { pid: pl.cloud_id, op: { replace: pl.tracks.map(slimTrack) } });
      pushedSig.set(pl.id, playlistSig(pl));
    } catch (e) { /* offline: next change retries */ }
  }, 1500));
}

/** Take the server's copy into the local playlist when it differs. */
async function pullPlaylist(localId, row) {
  const pl = state.playlists.find((p) => p.id === localId);
  if (!pl || !row) return;
  cloudMeta.set(row.id, { owner: row.owner, collaborative: row.collaborative, visibility: row.visibility });
  const remote = (row.tracks || []).map(fromCloudTrack);
  if (remote.map((t) => t.key).join(',') === pl.tracks.map((t) => t.key).join(',')) return;
  const added = Object.fromEntries((row.tracks || []).filter((t) => t._at).map((t) => [t.key, t._at]));
  await call('replace_playlist_tracks', { id: localId, tracks: remote, added });
  pushedSig.set(localId, playlistSig({ ...pl, tracks: remote }));
}

function watchPlaylist(localId, cloudId) {
  if (watchers.has(localId) || !signedIn()) return;
  const ch = new Channel(`pl:${cloudId}`, { changes: [{ event: 'UPDATE', schema: 'public', table: 'playlists', filter: `id=eq.${cloudId}` }] });
  ch.on('change', (d) => { if (d?.record) pullPlaylist(localId, d.record); });
  watchers.set(localId, ch);
}

/** On sign-in/start: refresh followed & collaborative playlists and watch them live. */
async function syncCloudPlaylists() {
  if (!signedIn()) { for (const ch of watchers.values()) ch.close(); watchers.clear(); return; }
  const linked = state.playlists.filter((p) => p.cloud_id);
  if (!linked.length) return;
  try {
    const rows = await db('GET', `playlists?id=in.(${linked.map((p) => p.cloud_id).join(',')})&select=id,owner,tracks,collaborative,visibility,version`);
    for (const pl of linked) {
      const row = rows.find((r) => r.id === pl.cloud_id);
      if (!row) continue;
      cloudMeta.set(row.id, { owner: row.owner, collaborative: row.collaborative, visibility: row.visibility });
      if (pl.cloud_role !== 'owner' || row.collaborative) await pullPlaylist(pl.id, row);
      else pushedSig.set(pl.id, playlistSig(pl));
      if (pl.cloud_role !== 'owner' || row.collaborative) watchPlaylist(pl.id, pl.cloud_id);
    }
  } catch { /* offline */ }
}

async function leaveCloudPlaylist(pl) {
  try {
    await db('DELETE', `playlist_members?playlist_id=eq.${pl.cloud_id}&user_id=eq.${cloud.session.user_id}`);
    watchers.get(pl.id)?.close();
    watchers.delete(pl.id);
    await call('set_playlist_cloud', { id: pl.id, cloudId: null, role: null });
    toast('Left. Your copy stays in Your Library.', 'info', 2200);
  } catch (e) { toast(errMsg(e), 'error'); }
}

// Opening links ---------------------------------------------------------------------
/** tunewire://p/<id>, https://…/tunewire/#p=<id>, etc. → handled; false if not ours. */
function handleShareLink(url) {
  const m = String(url).match(/^tunewire:\/\/([pcsju])\/([^?#\s]+)/) || String(url).match(/worldcnc\.github\.io\/tunewire\/?#([pcsju])=([^&\s]+)/);
  if (!m) return false;
  const [, kind, code] = m;
  if (kind === 's') {
    try { const t = trackFromSongLink(code); playTrack(t); toast(`Playing “${t.title}” from a shared link`, 'info', 2500); } catch { toast('That song link is broken', 'error'); }
  }
  if (kind === 'p') openSharedPlaylist(code);
  if (kind === 'c') joinCollaborative(code);
  if (kind === 'j') joinJam(code);
  if (kind === 'u') openProfile(code);
  return true;
}
listen('deep-link', (e) => handleShareLink(e.payload));

async function openSharedPlaylist(id) {
  const local = state.playlists.find((p) => p.cloud_id === id);
  if (local) return openPlaylist(local.id);
  if (!cloudOn()) return toast('Online features aren’t available in this build', 'warn');
  try {
    const rows = await db('GET', `playlists?id=eq.${id}&select=id,title,description,cover,tracks,owner,collaborative,visibility,profiles(name,avatar)`);
    const row = rows[0];
    if (!row) return toast('This playlist isn’t shared anymore, or it’s private', 'warn');
    cloudMeta.set(row.id, { owner: row.owner, collaborative: row.collaborative, visibility: row.visibility, ownerName: row.profiles?.name });
    state.remote = { title: row.title, url: PL_LINK(row.id), platform: null, tracks: (row.tracks || []).map(fromCloudTrack), cloud: { id: row.id, owner: row.owner, ownerName: row.profiles?.name, ownerAvatar: row.profiles?.avatar, description: row.description, cover: row.cover } };
    state.remote.tracks = await call('apply_matches', { tracks: state.remote.tracks }).catch(() => state.remote.tracks);
    openPlaylist('remote');
  } catch (e) { toast(errMsg(e), 'error'); }
}

/** Follow a shared playlist: a local copy that keeps getting its updates. */
async function followShared(remote) {
  if (!(await requireAccount({ why: 'Sign in to follow playlists' }))) return;
  try {
    if (remote.cloud.owner !== cloud.session.user_id) await db('POST', 'playlist_members', { playlist_id: remote.cloud.id, role: 'follower' }, { prefer: 'resolution=ignore-duplicates' }).catch(() => {});
    const pl = await call('create_playlist', { name: remote.title, tracks: remote.tracks, sourceUrl: null });
    await call('set_playlist_cloud', { id: pl.id, cloudId: remote.cloud.id, role: remote.cloud.owner === cloud.session.user_id ? 'owner' : 'follower' });
    await refreshPlaylists({ silent: true });
    watchPlaylist(pl.id, remote.cloud.id);
    state.remote = null;
    openPlaylist(pl.id, { push: false });
    toast(`Following “${pl.name}”. You’ll get its updates.`, 'ok', 2500);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function joinCollaborative(code) {
  if (!(await requireAccount({ why: 'Sign in to join this playlist' }))) return;
  try {
    const id = await rpc('join_playlist', { code });
    const existing = state.playlists.find((p) => p.cloud_id === id);
    if (existing) return openPlaylist(existing.id);
    const [row] = await db('GET', `playlists?id=eq.${id}&select=id,title,tracks,owner,collaborative,visibility`);
    const pl = await call('create_playlist', { name: row.title, tracks: (row.tracks || []).map(fromCloudTrack), sourceUrl: null });
    await call('set_playlist_cloud', { id: pl.id, cloudId: id, role: 'editor' });
    cloudMeta.set(id, { owner: row.owner, collaborative: true, visibility: row.visibility });
    await refreshPlaylists({ silent: true });
    watchPlaylist(pl.id, id);
    openPlaylist(pl.id);
    toast(`You joined “${row.title}”. Your changes show up for everyone.`, 'ok', 3000);
  } catch (e) { toast(errMsg(e), 'error'); }
}

// Listen Together -----------------------------------------------------------------------
const jam = { code: null, host: false, channel: null, people: [], lastState: null, timer: null, applying: false };

function jamCode() {
  const a = 'abcdefghjkmnpqrstuvwxyz23456789';
  const b = new Uint8Array(10);
  crypto.getRandomValues(b);
  return [...b].map((x) => a[x % a.length]).join('');
}

async function startJam() {
  if (jam.channel) { setPanelOpen(true); showPanel('together'); return; }
  if (!(await requireAccount({ guestOk: true, why: 'Listen together' }))) return;
  openJamChannel(jamCode(), true);
  copyText(JAM_LINK(jam.code));
  toast('Session started. Invite link copied: friends who open it hear what you play.', 'ok', 4000);
}

async function joinJam(code) {
  if (jam.code === code) { setPanelOpen(true); showPanel('together'); return; }
  if (!(await requireAccount({ guestOk: true, why: 'Join the listening session' }))) return;
  if (jam.channel) leaveJam();
  openJamChannel(code, false);
  toast('Joined. You’ll hear what the host plays.', 'ok', 2500);
}

function openJamChannel(code, host) {
  jam.code = code;
  jam.host = host;
  jam.channel = new Channel(`jam:${code}`, { presenceKey: cloud.session.user_id });
  const me = { name: cloud.session.name, avatar: cloud.session.avatar || null, host };
  jam.channel.track(me);
  jam.channel.on('presence', (p) => {
    const before = jam.people.length;
    jam.people = presenceList(p);
    if (jam.host && jam.people.length > before) jamBroadcastState(true);
    renderTogether();
  });
  jam.channel.on('b:state', (st) => { if (!jam.host) applyJamState(st); });
  jam.channel.on('b:hello', () => { if (jam.host) jamBroadcastState(true); });
  jam.channel.on('b:add', (m) => {
    if (!jam.host) return;
    enqueue([m.track]);
    toast(`${m.from} added “${m.track.title}”`, 'info', 2500);
  });
  jam.channel.on('b:react', (m) => floatReaction(m.emoji, m.from));
  jam.channel.on('b:end', () => { if (!jam.host) { toast('The host ended the session', 'info', 2500); leaveJam(); } });
  jam.channel.on('status', (s) => { if (s === 'joined' && !jam.host) jam.channel.broadcast('hello', {}); if (s === 'error') setTimeout(() => jam.channel && !jam.channel.joined && jam.channel.ws?.close(), 1500); });
  clearInterval(jam.timer);
  jam.timer = setInterval(() => { if (jam.host) jamBroadcastState(false); }, 4000);
  setPanelOpen(true);
  showPanel('together');
  renderTogether();
  paintJamButton();
}

function leaveJam() {
  if (jam.host) jam.channel?.broadcast('end', {});
  setTimeout(() => jam.channel?.close(), jam.host ? 300 : 0);
  clearInterval(jam.timer);
  Object.assign(jam, { code: null, host: false, channel: null, people: [], lastState: null });
  renderTogether();
  paintJamButton();
  if (state.panel === 'together') showPanel('queue');
}

function jamBroadcastState(force) {
  const p = state.playback;
  if (!jam.channel || !p) return;
  const t = p.current?.track;
  const st = { track: t ? slimTrack(t) : null, playing: p.status === 'playing', position: p.position || 0, at: Date.now(), upcoming: state.queue.items.slice((state.queue.current ?? -1) + 1, (state.queue.current ?? -1) + 6).map((i) => ({ title: i.track.title, artist: i.track.artist })) };
  const sig = `${st.track?.key}|${st.playing}|${Math.round(st.position / 4)}`;
  if (!force && sig === jam.lastState) return;
  jam.lastState = sig;
  jam.channel.broadcast('state', st);
}
// Host: share every real change right away.
let jamLastSig = '';
function onPlaybackForJam() {
  if (!jam.host || !jam.channel) return;
  const p = state.playback;
  const sig = `${p?.current?.track?.key}|${p?.status}`;
  if (sig !== jamLastSig) { jamLastSig = sig; jamBroadcastState(true); }
}

async function applyJamState(st) {
  jam.remote = st;
  renderTogether();
  if (jam.applying || !st.track) return;
  const p = state.playback;
  const cur = p?.current?.track?.key;
  const target = st.position + (st.playing ? (Date.now() - st.at) / 1000 : 0);
  jam.applying = true;
  try {
    if (cur !== st.track.key) {
      await call('play_track', { track: fromCloudTrack(st.track), platform: null });
      // Seek once it's loaded (a few hundred ms usually).
      for (let i = 0; i < 40; i++) {
        await new Promise((r) => setTimeout(r, 250));
        if (state.playback?.status === 'playing' && state.playback.current?.track?.key === st.track.key) break;
      }
      const now = st.position + (st.playing ? (Date.now() - st.at) / 1000 : 0);
      if (now > 2) await call('seek', { position: now });
    } else if (st.playing && p.status !== 'playing') {
      await call('resume');
    } else if (!st.playing && p.status === 'playing') {
      await call('pause');
    }
    if (cur === st.track.key && Math.abs((state.playback?.position || 0) - target) > 1.5) await call('seek', { position: target });
    if (!st.playing && cur === st.track.key) await call('pause');
  } catch { /* next state fixes it */ }
  jam.applying = false;
}

function jamAdd(track) {
  if (!jam.channel) return;
  if (jam.host) return enqueue([track]);
  jam.channel.broadcast('add', { track: slimTrack(track), from: cloud.session.name });
  toast(`Added “${track.title}” to the session`, 'ok', 1800);
}

function sendReaction(emoji) {
  jam.channel?.broadcast('react', { emoji, from: cloud.session.name });
  floatReaction(emoji, 'You');
}
function floatReaction(emoji, from) {
  const el = h('div', { class: 'reaction' }, emoji, h('small', {}, from));
  el.style.left = `${55 + Math.random() * 30}%`;
  document.body.append(el);
  setTimeout(() => el.remove(), 2600);
}

function paintJamButton() {
  const b = $('#btn-jam');
  if (b) { b.classList.toggle('on', !!jam.channel); b.hidden = !cloudOn(); }
}

function renderTogether() {
  const box = $('#together-panel');
  if (!box) return;
  if (!jam.channel) {
    box.replaceChildren(h('div', { class: 'empty' },
      h('p', {}, 'Listen together'),
      h('p', { class: 'small' }, 'Start a session and send the link: everyone hears the same song at the same moment, and anyone can add songs.'),
      h('div', { class: 'actions', style: 'justify-content:center;margin-top:12px' },
        h('button', { class: 'btn primary', onclick: startJam }, 'Start a session'),
        h('button', { class: 'btn', onclick: async () => { const c = await promptModal({ title: 'Join a session', placeholder: 'Code or link', okLabel: 'Join' }); if (c && !handleShareLink(c)) joinJam(c.trim()); } }, 'Join'))));
    return;
  }
  const st = jam.remote;
  box.replaceChildren(
    h('div', { class: 'jam-head' },
      h('b', {}, jam.host ? 'You’re hosting' : 'Listening together'),
      h('span', { class: 'meta' }, `${jam.people.length} ${jam.people.length === 1 ? 'person' : 'people'}`)),
    h('div', { class: 'jam-people' }, ...jam.people.map((p) => h('div', { class: 'jam-person' }, avatarEl(p.avatar, p.name), h('span', {}, p.name), p.host ? h('em', {}, 'host') : null))),
    h('div', { class: 'actions' },
      h('button', { class: 'btn small', html: `${icon('link')}Copy invite link`, onclick: () => { copyText(JAM_LINK(jam.code)); toast('Invite link copied', 'ok', 1500); } }),
      h('button', { class: 'btn small danger-ghost', onclick: leaveJam }, jam.host ? 'End session' : 'Leave')),
    !jam.host && st?.track ? h('div', { class: 'npv-card' }, h('h4', {}, 'Now playing'), h('div', { class: 'npv-source' }, `${st.track.title} — ${st.track.artist}`)) : null,
    !jam.host && st?.upcoming?.length ? h('div', { class: 'npv-card' }, h('h4', {}, 'Up next'), h('div', { class: 'npv-source' }, ...st.upcoming.map((u) => h('div', {}, `${u.title} — ${u.artist}`)))) : null,
    h('div', { class: 'jam-hint meta' }, jam.host ? 'Guests follow what you play. Their added songs go into your queue.' : 'Add songs with ⋯ → Add to session. The host controls playback.'),
    h('div', { class: 'reactions-bar' }, ...['🔥', '❤️', '😂', '🕺', '👏', '😮'].map((e) => h('button', { onclick: () => sendReaction(e) }, e))));
}

// Friends -----------------------------------------------------------------------------------
const friends = { list: [], timer: null, lastPublished: '' };

async function renderFriends() {
  const box = $('#friends-panel');
  if (!box || box.hidden) return;
  if (!cloudOn()) { box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Not available in this build.'))); return; }
  if (!signedIn() || cloud.session.anonymous) {
    box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'See what your friends are listening to'), h('p', { class: 'small' }, 'Follow friends and their current song shows up here.'),
      h('button', { class: 'btn primary', style: 'margin-top:12px', onclick: () => requireAccount({ why: 'Sign in to see friends' }) }, 'Sign in')));
    return;
  }
  const search = h('input', { class: 'input', placeholder: 'Find people by name', spellcheck: 'false' });
  const results = h('div', { class: 'people-results' });
  search.addEventListener('input', debounce(async () => {
    const q = search.value.trim();
    if (q.length < 2) { results.replaceChildren(); return; }
    try {
      const rows = await db('GET', `profiles?name=ilike.*${encodeURIComponent(q.replace(/[*,()]/g, ''))}*&select=id,name,avatar&limit=8`);
      results.replaceChildren(...rows.filter((r) => r.id !== cloud.session.user_id).map((r) => personRow(r)));
    } catch { /* offline */ }
  }, 300));
  const list = h('div', { class: 'friends-list' }, h('div', { class: 'loading-line' }, h('span', { class: 'spinner' })));
  box.replaceChildren(h('div', { class: 'friends-tools' }, search, h('button', { class: 'icon-btn', title: 'Copy link to your profile', html: icon('link'), onclick: () => { copyText(PROFILE_LINK(cloud.session.user_id)); toast('Profile link copied', 'ok', 1500); } })), results, list);
  try {
    const follows = await db('GET', `follows?follower=eq.${cloud.session.user_id}&select=followee,profiles!follows_followee_fkey(id,name,avatar)`);
    const ids = follows.map((f) => f.followee);
    const playing = ids.length ? await db('GET', `now_playing?user_id=in.(${ids.join(',')})&select=user_id,track,playing,updated_at`) : [];
    const byUser = new Map(playing.map((p) => [p.user_id, p]));
    const rows = follows.map((f) => ({ ...f.profiles, np: byUser.get(f.followee) }))
      .sort((a, b) => (b.np?.updated_at || '').localeCompare(a.np?.updated_at || ''));
    if (!rows.length) { list.replaceChildren(h('p', { class: 'meta', style: 'padding:12px 4px' }, 'You don’t follow anyone yet. Search above, or send friends your profile link.')); return; }
    list.replaceChildren(...rows.map((r) => friendRow(r)));
  } catch (e) { list.replaceChildren(h('p', { class: 'meta' }, errMsg(e))); }
}

function personRow(r) {
  const btn = h('button', { class: 'btn small' }, 'Follow');
  btn.addEventListener('click', async () => {
    try { await db('POST', 'follows', { followee: r.id }, { prefer: 'resolution=ignore-duplicates' }); btn.textContent = 'Following'; btn.disabled = true; renderFriends(); } catch (e) { toast(errMsg(e), 'error'); }
  });
  return h('div', { class: 'friend' }, avatarEl(r.avatar, r.name), h('button', { class: 'friend-name', onclick: () => openProfile(r.id) }, r.name), btn);
}

function friendRow(r) {
  const np = r.np;
  const fresh = np && Date.now() - Date.parse(np.updated_at) < 15 * 60 * 1000;
  const t = np?.track;
  return h('div', { class: 'friend' }, avatarEl(r.avatar, r.name),
    h('div', { class: 'friend-main' },
      h('button', { class: 'friend-name', onclick: () => openProfile(r.id) }, r.name),
      t ? h('button', { class: 'friend-track', title: 'Play', onclick: () => playTrack(fromCloudTrack(t)) }, `${fresh && np.playing ? '▶ ' : ''}${t.title} · ${t.artist}`) : h('span', { class: 'meta' }, 'Nothing played yet')),
    np ? h('span', { class: 'meta friend-when' }, fresh && np.playing ? 'now' : timeAgo(Date.parse(np.updated_at))) : null);
}

/** Tell followers what's playing (throttled, only real changes). */
function publishActivity() {
  if (!signedIn() || cloud.session.anonymous || state.settings?.share_activity === false) return;
  const p = state.playback;
  const t = p?.current?.track;
  const sig = `${t?.key}|${p?.status === 'playing'}`;
  if (!t || sig === friends.lastPublished || p.status === 'loading') return;
  friends.lastPublished = sig;
  db('POST', 'now_playing', { user_id: cloud.session.user_id, track: slimTrack(t), playing: p.status === 'playing', updated_at: new Date().toISOString() }, { prefer: 'resolution=merge-duplicates' }).catch(() => {});
}

// Profiles ------------------------------------------------------------------------------------
let profileSeq = 0;
async function openProfile(userId, { push = true } = {}) {
  if (!cloudOn()) return;
  const seq = ++profileSeq;
  showView('profile', { push, reopen: () => openProfile(userId, { push: false }) });
  const body = $('#profile-body');
  body.replaceChildren(h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Loading profile…'));
  try {
    const [[p], lists, followers, following] = await Promise.all([
      db('GET', `profiles?id=eq.${userId}&select=id,name,avatar`),
      db('GET', `playlists?owner=eq.${userId}&visibility=eq.public&select=id,title,cover,tracks->0->thumbnail&order=updated_at.desc`),
      db('GET', `follows?followee=eq.${userId}&select=follower`),
      db('GET', `follows?follower=eq.${userId}&select=followee`),
    ]);
    if (seq !== profileSeq) return;
    if (!p) { body.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'This profile doesn’t exist.'))); return; }
    const mine = signedIn() && p.id === cloud.session.user_id;
    const isFollowing = signedIn() && followers.some((f) => f.follower === cloud.session.user_id);
    const follow = h('button', { class: 'ab-follow' }, isFollowing ? 'Following' : 'Follow');
    follow.addEventListener('click', async () => {
      if (!(await requireAccount({ why: 'Sign in to follow people' }))) return;
      try {
        if (follow.textContent === 'Following') { await db('DELETE', `follows?follower=eq.${cloud.session.user_id}&followee=eq.${p.id}`); follow.textContent = 'Follow'; }
        else { await db('POST', 'follows', { followee: p.id }, { prefer: 'resolution=ignore-duplicates' }); follow.textContent = 'Following'; }
      } catch (e) { toast(errMsg(e), 'error'); }
    });
    const art = h('div', { class: 'hero-art round' });
    if (p.avatar) art.append(h('img', { src: p.avatar, 'data-px': 300, alt: '', referrerpolicy: 'no-referrer' })); else art.innerHTML = icon('user');
    body.replaceChildren(
      h('div', { class: 'hero' }, art, h('div', { class: 'hero-meta' }, h('div', { class: 'hero-kicker' }, 'Profile'), h('h1', { class: 'hero-title' }, p.name),
        h('div', { class: 'hero-line' }, `${lists.length} public playlist${lists.length === 1 ? '' : 's'}`, h('span', { class: 'dot' }, `${followers.length} follower${followers.length === 1 ? '' : 's'}`), h('span', { class: 'dot' }, `${following.length} following`)))),
      h('div', { class: 'action-bar' }, mine ? null : follow,
        h('button', { class: 'ab-icon', title: 'Copy profile link', html: icon('link'), onclick: () => { copyText(PROFILE_LINK(p.id)); toast('Profile link copied', 'ok', 1500); } })),
      lists.length ? h('div', {}, h('div', { class: 'section-title' }, 'Public playlists'),
        h('div', { class: 'card-row' }, ...lists.map((l, i) => cardTile({ kind: 'playlist', title: l.title, subtitle: `Playlist · ${p.name}`, thumbnail: l.cover || l.thumbnail, browse_id: null, track: null, _cloud: l.id }, i))))
        : h('p', { class: 'meta' }, mine ? 'Make a playlist public (⋯ → Share link → Public) to show it here.' : 'No public playlists yet.'));
    $$('#profile-body .tile').forEach((tile, i) => { tile.onclick = (e) => { e.stopPropagation(); openSharedPlaylist(lists[i].id); }; const pb = tile.querySelector('.tile-play'); if (pb) pb.onclick = async (e) => { e.stopPropagation(); const [row] = await db('GET', `playlists?id=eq.${lists[i].id}&select=tracks`); playList(row.tracks.map(fromCloudTrack), 0); }; });
    heroColor(body, p.avatar);
  } catch (e) { if (seq === profileSeq) body.replaceChildren(h('div', { class: 'empty' }, h('p', {}, errMsg(e)))); }
}

// Wrapped: compare with a friend ------------------------------------------------------------------
async function compareWrapped(s, year) {
  if (!(await requireAccount({ why: 'Sign in to compare with friends' }))) return;
  const mine = { artists: s.top_artists.slice(0, 30).map((a) => a.name), songs: s.top_tracks.slice(0, 30).map((t) => `${t.track.artist} — ${t.track.title}`), minutes: Math.round(s.minutes) };
  try {
    await db('POST', 'wrapped', { user_id: cloud.session.user_id, year, data: mine, updated_at: new Date().toISOString() }, { prefer: 'resolution=merge-duplicates' });
    const follows = await db('GET', `follows?follower=eq.${cloud.session.user_id}&select=followee,profiles!follows_followee_fkey(id,name,avatar)`);
    const theirs = follows.length ? await db('GET', `wrapped?year=eq.${year}&user_id=in.(${follows.map((f) => f.followee).join(',')})`) : [];
    const m = openModal({
      title: `Compare your ${year}`,
      wide: true,
      body: theirs.length ? h('div', { class: 'import-list' }, ...theirs.map((w) => {
        const who = follows.find((f) => f.followee === w.user_id)?.profiles;
        const common = mine.artists.filter((a) => w.data.artists?.includes(a));
        const score = Math.round((common.length / Math.max(1, Math.min(mine.artists.length, w.data.artists?.length || 0))) * 100);
        return h('div', { class: 'import-row', style: 'align-items:flex-start' }, avatarEl(who?.avatar, who?.name),
          h('div', { style: 'flex:1;min-width:0' }, h('b', {}, `${who?.name || 'Friend'} · ${score}% match`),
            h('div', { class: 'meta' }, common.length ? `You both love ${common.slice(0, 5).join(', ')}${common.length > 5 ? ` and ${common.length - 5} more` : ''}` : 'Totally different taste this year'),
            h('div', { class: 'meta' }, `${w.data.minutes?.toLocaleString() || 0} minutes vs your ${mine.minutes.toLocaleString()}`)));
      })) : h('p', {}, 'None of the people you follow have shared their Wrapped yet. Yours is shared now, so they can compare with you.'),
      footer: [h('button', { class: 'btn primary', onclick: () => m.close() }, 'Done')],
    });
  } catch (e) { toast(errMsg(e), 'error'); }
}
