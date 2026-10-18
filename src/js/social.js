'use strict';

// ===========================================================================
// Social: account, privacy, sharing, shared & collaborative playlists (with
// sync status), Listen Together, friends and follow requests, notifications,
// profiles, comparing Wrapped. Accounts are Discord only.
// ===========================================================================

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const me = () => cloud.session?.user_id;

// Account ---------------------------------------------------------------------------
function avatarEl(src, name, cls = 'avatar') {
  const el = h('span', { class: cls, title: name || '' });
  const url = mediaUrl(src);
  if (url) el.append(h('img', { src: url, alt: '', 'data-px': 96, referrerpolicy: 'no-referrer', onerror: (e) => { e.target.remove(); el.textContent = (name || '?')[0].toUpperCase(); } }));
  else el.textContent = (name || '?')[0].toUpperCase();
  return el;
}

function paintAccountButton() {
  const b = $('#nav-account');
  if (!b) return;
  b.hidden = !cloudOn();
  b.replaceChildren(signedIn() ? avatarEl(cloud.session.avatar, cloud.session.name, 'avatar tb-avatar') : h('span', { html: icon('user') }));
  if (notif.unread) b.append(h('em', { class: 'nav-count new', 'aria-hidden': 'true' }, String(Math.min(99, notif.unread))));
  b.title = signedIn() ? `${cloud.session.name}${notif.unread ? ` · ${notif.unread} new` : ''}` : 'Sign in';
  b.setAttribute('aria-label', b.title);
}
onCloudChange(paintAccountButton);
onCloudChange(() => {
  if (state.view === 'settings') renderSettings();
  renderFriends();
  startNotifications();
  if (!signedIn() && jam.code) leaveJam({ silent: true });
});

$('#nav-account')?.addEventListener('click', (e) => {
  const anchor = e.currentTarget;
  if (!signedIn()) return requireAccount({ why: 'Sign in to Tunewire' });
  openMenu(anchor, [
    { label: cloud.session.name, heading: true },
    { label: notif.unread ? `Notifications (${notif.unread})` : 'Notifications', icon: 'bell', onClick: () => openNotifications() },
    { label: 'Your profile', icon: 'user', onClick: () => openProfile(cloud.session.user_id) },
    { label: 'Friends', icon: 'friends', onClick: () => { setPanelOpen(true); showPanel('friends'); } },
    { label: 'Listen together', icon: 'friends', onClick: () => startJam() },
    'sep',
    { label: 'Sign out', icon: 'x', onClick: () => signOut() },
  ]);
});

async function signOut() {
  if (jam.code) await leaveJam({ silent: true });
  await call('cloud_sign_out').catch(() => {});
  cloud.session = null;
  cloud.profile = null;
  cloudChanged();
  toast('Signed out', 'info', 1500);
}

// Privacy ------------------------------------------------------------------------------
/** First run: ask before any error report is sent. */
function askPrivacyFirstRun() {
  if (!state.settings || state.settings.privacy_asked) return;
  const reports = h('input', { type: 'checkbox', checked: false });
  const m = openModal({
    title: 'Before you start',
    subtitle: 'Tunewire keeps your music and listening on this computer. One question:',
    body: h('div', { class: 'privacy-list' },
      h('label', { class: 'setting' },
        h('div', { class: 'label' }, 'Send error reports', h('small', {}, 'When something breaks, send the developer the app version, operating system and the error, with your folder names and links removed. Nothing about your music is sent.')),
        h('span', { class: 'switch' }, reports, h('span')))),
    footer: [h('button', { class: 'btn primary', onclick: async () => { await updateSettings({ privacy_asked: true, error_reports: reports.checked }); m.close(); } }, 'Continue')],
    onClose: () => { if (!state.settings.privacy_asked) updateSettings({ privacy_asked: true, error_reports: false }); },
  });
}

/** After signing in (once): what friends may see. Everything starts private. */
function askCloudPrivacy(force = false) {
  if (!signedIn() || (!force && state.settings?.cloud_privacy_asked)) return;
  const activity = h('input', { type: 'checkbox', checked: !!cloud.profile?.share_activity });
  const wrapped = h('input', { type: 'checkbox', checked: cloud.profile?.share_wrapped !== false });
  const stats = h('input', { type: 'checkbox', checked: !!cloud.profile?.share_stats });
  const library = h('input', { type: 'checkbox', checked: state.settings?.sync_asked ? !!state.settings.sync_library : true });
  const row = (label, help, input) => h('label', { class: 'setting' }, h('div', { class: 'label' }, label, h('small', {}, help)), h('span', { class: 'switch' }, input, h('span')));
  const save = async () => {
    await saveCloudPrivacy({ share_activity: activity.checked, share_wrapped: wrapped.checked, share_stats: stats.checked });
    await updateSettings({ cloud_privacy_asked: true });
    m.close();
    if (library.checked !== !!state.settings.sync_library || !state.settings.sync_asked) await setLibrarySync(library.checked);
  };
  const m = openModal({
    title: 'What friends can see',
    subtitle: 'People need your approval to follow you. Choose what approved followers see; you can change this any time in Settings → Account.',
    body: h('div', { class: 'privacy-list' },
      row('Share what I’m playing', 'Approved followers see your current song in their Friends panel.', activity),
      row('Share my listening', 'Your top artists and songs, recently played and minutes this week show on your profile for approved followers, and you join the friends’ leaderboard.', stats),
      row('Compare Wrapped with friends', 'Approved friends can compare their year in music with yours (only after you press “Compare”).', wrapped),
      row('Sync my library', 'Your playlists and Liked Songs on every device you sign in to, as private copies only you can see.', library)),
    footer: [h('button', { class: 'btn primary', onclick: save }, 'Save')],
    onClose: () => { if (!state.settings.cloud_privacy_asked) updateSettings({ cloud_privacy_asked: true }); },
  });
}

async function saveCloudPrivacy(patch) {
  try {
    await db('PATCH', `profiles?id=eq.${me()}`, patch);
    cloud.profile = { ...(cloud.profile || {}), ...patch };
    if ('share_activity' in patch) {
      await updateSettings({ share_activity: patch.share_activity });
      if (!patch.share_activity) db('DELETE', `now_playing?user_id=eq.${me()}`).catch(() => {});
      else publishActivity(true);
    }
    if (patch.share_wrapped === false) db('DELETE', `wrapped?user_id=eq.${me()}`).catch(() => {});
    if (patch.share_stats === false) db('DELETE', `profile_stats?user_id=eq.${me()}`).catch(() => {});
    if (patch.share_stats) publishStats(true);
  } catch (e) { toast(errMsg(e), 'error'); }
}

function accountSettingsRows() {
  if (!cloudOn()) return [settingRow('Account', 'Online features (sharing, Listen Together, friends) aren’t available in this build.', h('span'))];
  if (!signedIn()) {
    return [settingRow('Account', 'Sign in with Discord to share playlists, make collaborative playlists, listen together and see what friends play. Everything else works without an account.',
      h('button', { class: 'btn small primary', onclick: () => requireAccount({ why: 'Sign in to Tunewire' }) }, 'Sign in with Discord'))];
  }
  const s = cloud.session;
  const toggle = (label, help, key) => {
    const input = h('input', { type: 'checkbox', checked: key === 'share_wrapped' ? cloud.profile?.share_wrapped !== false : !!cloud.profile?.[key] });
    input.addEventListener('change', () => saveCloudPrivacy({ [key]: input.checked }));
    return h('div', { class: 'setting' }, h('div', { class: 'label' }, label, h('small', {}, help)), h('label', { class: 'switch' }, input, h('span')));
  };
  const syncInput = h('input', { type: 'checkbox', checked: !!state.settings?.sync_library });
  syncInput.addEventListener('change', () => setLibrarySync(syncInput.checked));
  return [
    settingRow(h('span', { class: 'row-gap' }, avatarEl(s.avatar, s.name), s.name), 'Signed in with Discord.',
      h('div', { class: 'actions' },
        h('button', { class: 'btn small', onclick: async () => {
          const name = await promptModal({ title: 'Your name', label: 'Friends see this name.', value: s.name, okLabel: 'Save' });
          if (!name) return;
          try { await db('PATCH', `profiles?id=eq.${s.user_id}`, { name: name.slice(0, 40) }); cloud.session.name = name.slice(0, 40); cloudChanged(); toast('Name saved', 'ok', 1500); } catch (e) { toast(errMsg(e), 'error'); }
        } }, 'Change name'),
        h('button', { class: 'btn small', onclick: () => openProfile(s.user_id) }, 'Profile'), h('button', { class: 'btn small', onclick: signOut }, 'Sign out'))),
    h('div', { class: 'setting' }, h('div', { class: 'label' }, 'Sync my library', h('small', {}, 'Your playlists and Liked Songs on every device you sign in to. The online copies are private: only you can see them.')), h('label', { class: 'switch' }, syncInput, h('span'))),
    toggle('Share what I’m listening to', 'Approved followers see the song you’re playing in their Friends panel.', 'share_activity'),
    toggle('Share my listening', 'Top artists and songs, recently played and minutes this week on your profile (approved followers only), and the friends’ leaderboard.', 'share_stats'),
    toggle('Compare Wrapped with friends', 'Approved friends can compare their year with yours. Turning this off removes what you shared.', 'share_wrapped'),
    settingRow('Followers', 'Everyone who follows you needed your approval. See and remove followers on your profile.', h('button', { class: 'btn small', onclick: () => openProfile(s.user_id) }, 'Manage')),
  ];
}

// Sharing ---------------------------------------------------------------------------
function shareSong(track) {
  copyText(songLink(track));
  toast('Song link copied. Anyone can open it, even without Tunewire.', 'ok', 2500);
}

const PL_LINK = (id) => shareLink('p', id);
const INVITE_LINK = (code) => shareLink('c', code);
const JAM_LINK = (code) => shareLink('j', code);
const PROFILE_LINK = (id) => shareLink('u', id);

/**
 * Share a playlist (or change who can open it) and return its online id.
 * With library sync it's online already (private): this only opens it up.
 */
async function publishPlaylist(pl, { visibility, collaborative } = {}) {
  if (!(await requireAccount({ why: 'Sign in to share playlists' }))) return null;
  return syncTask(async () => {
    // Ask the backend: the page's copy may not know about a link made a moment ago.
    try { pl = (await call('list_playlists')).find((p) => p.id === pl.id) || pl; } catch { /* keep */ }
    if (pl.cloud_id && pl.cloud_role === 'owner') {
      const m = cloudMeta.get(pl.cloud_id) || {};
      const patch = {};
      if (visibility) patch.visibility = visibility;
      else if (m.visibility === 'private' && !m.collaborative && !collaborative) patch.visibility = 'unlisted';
      if (collaborative !== undefined) patch.collaborative = collaborative;
      if (collaborative && m.visibility === 'private' && !m.collaborative && !visibility) patch.visibility = 'unlisted';
      if (Object.keys(patch).length) {
        const row = await patchRow(pl.cloud_id, patch);
        if (row) rememberRow(row);
      }
      syncLocalSoon(0); // anything not sent yet
      return pl.cloud_id;
    }
    const row = await uploadPlaylist(pl, { visibility: visibility || 'unlisted', collaborative: !!collaborative });
    setSync(pl.id, 'synced');
    return row.id;
  });
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
    const code = await rpc('playlist_invite', { pid: id });
    copyText(INVITE_LINK(code));
    toast('Invite link copied. Friends who open it can add, remove and reorder songs with you. Only send it to people you trust.', 'ok', 5000);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function resetInvite(pl) {
  if (!(await confirmModal({ title: 'Make a new invite link?', message: 'The old link stops working. People who already joined stay collaborators.', okLabel: 'New link' }))) return;
  try {
    copyText(INVITE_LINK(await rpc('rotate_playlist_invite', { pid: pl.cloud_id })));
    toast('New invite link copied', 'ok', 2500);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function setVisibility(pl, visibility) {
  try { await publishPlaylist(pl, { visibility }); toast({ public: 'Shown on your profile', unlisted: 'Anyone with the link can open it', private: 'Only you and collaborators can open it' }[visibility], 'ok', 2200); } catch (e) { toast(errMsg(e), 'error'); }
}

async function unsharePlaylist(pl) {
  if (!(await confirmModal({ title: 'Stop sharing?', message: 'The link stops working and collaborators lose access. Your playlist stays in Your Library.', okLabel: 'Stop sharing', danger: true }))) return;
  try {
    await syncTask(async () => {
      if (libSyncOn()) {
        // Still your synced copy: just private again, without the others.
        const row = await patchRow(pl.cloud_id, { visibility: 'private', collaborative: false });
        if (row) rememberRow(row);
        await db('DELETE', `playlist_members?playlist_id=eq.${pl.cloud_id}`);
        await rpc('rotate_playlist_invite', { pid: pl.cloud_id }).catch(() => {});
        paintSyncBadges(pl.id);
        return;
      }
      await db('DELETE', `playlists?id=eq.${pl.cloud_id}`);
      delete bases().pl[pl.cloud_id];
      syncSave();
      await call('set_playlist_cloud', { id: pl.id, cloudId: null, role: null });
      syncInfo.delete(pl.id);
    });
    if (state.view === 'playlist') renderPlaylistDetail();
    toast('No longer shared', 'info', 1800);
  } catch (e) { toast(errMsg(e), 'error'); }
}

/** Extra items for a playlist's ⋯ menu. */
function shareMenuItems(pl) {
  if (!cloudOn()) return [];
  // A private sync copy isn't shared: the menu offers sharing it.
  const linked = isShared(pl) ? pl.cloud_id : null;
  if (linked && pl.cloud_role !== 'owner') {
    return ['sep', { label: 'Copy link', icon: 'link', onClick: () => { copyText(PL_LINK(pl.cloud_id)); toast('Link copied', 'ok', 1500); } },
      { label: pl.cloud_role === 'editor' ? 'Leave collaborative playlist' : 'Unfollow', icon: 'x', onClick: () => leaveCloudPlaylist(pl) }];
  }
  const meta = cloudMeta.get(pl.cloud_id) || {};
  return ['sep',
    { label: linked ? 'Copy link' : 'Share link', icon: 'link', onClick: () => sharePlaylist(pl) },
    { label: meta.collaborative ? 'Copy invite link' : 'Make collaborative', icon: 'friends', onClick: () => makeCollaborative(pl) },
    meta.collaborative ? { label: 'New invite link (old one stops)', icon: 'refresh', onClick: () => resetInvite(pl) } : null,
    linked ? { label: 'Who can open it', heading: true } : null,
    linked ? { label: `${meta.visibility === 'public' ? '✓ ' : ''}Public (on your profile)`, onClick: () => setVisibility(pl, 'public') } : null,
    linked ? { label: `${!meta.visibility || meta.visibility === 'unlisted' ? '✓ ' : ''}Anyone with the link`, onClick: () => setVisibility(pl, 'unlisted') } : null,
    linked ? { label: `${meta.visibility === 'private' ? '✓ ' : ''}Only collaborators`, onClick: () => setVisibility(pl, 'private') } : null,
    linked ? { label: 'Stop sharing', icon: 'x', onClick: () => unsharePlaylist(pl) } : null,
  ].filter(Boolean);
}

// Leaving someone else's playlist (sync itself lives in sync.js).
async function leaveCloudPlaylist(pl) {
  try {
    await syncTask(async () => {
      await db('DELETE', `playlist_members?playlist_id=eq.${pl.cloud_id}&user_id=eq.${me()}`);
      delete bases().pl[pl.cloud_id];
      syncSave();
      await call('set_playlist_cloud', { id: pl.id, cloudId: null, role: null });
      syncInfo.delete(pl.id);
    });
    toast('Left. Your copy stays in Your Library.', 'info', 2200);
  } catch (e) { toast(errMsg(e), 'error'); }
}

// Opening links ---------------------------------------------------------------------
// Every kind of code has a fixed shape; anything else is refused before it
// gets near a database query. Links that change something (join a playlist
// or a session, play a song) ask first.
const LINK_CODES = {
  p: UUID_RE,
  u: UUID_RE,
  c: /^[0-9a-f]{24,64}$/,
  j: /^[a-z0-9]{8,24}$/,
  s: /^[A-Za-z0-9_-]{4,4000}$/,
};

/** tunewire://p/<id>, https://…/tunewire/#p=<id>, <share server>/p/<id> → {kind, code}. */
function parseShareLink(url) {
  const u = String(url).trim();
  let m = u.match(/^tunewire:\/\/([pcsju])\/([^?#\s/]+)\/?$/) || u.match(/^https:\/\/worldcnc\.github\.io\/tunewire\/?#([pcsju])=([^&\s]+)$/);
  const base = cloud.cfg?.share_url;
  if (!m && base && u.startsWith(`${base}/`)) m = u.slice(base.length).match(/^\/([pcsju])\/([^?#\s/]+)$/);
  if (!m) return null;
  let code = m[2];
  try { code = decodeURIComponent(code); } catch { return { kind: m[1], code: null }; }
  return { kind: m[1], code: LINK_CODES[m[1]].test(code) ? code : null };
}

/** Handled (true), or not one of our links (false). */
function handleShareLink(url) {
  const link = parseShareLink(url);
  if (!link) return false;
  if (!link.code) { toast('That Tunewire link is broken or incomplete', 'error'); return true; }
  const { kind, code } = link;
  if (kind === 's') confirmSongLink(code);
  if (kind === 'p') openSharedPlaylist(code);
  if (kind === 'c') confirmJoinCollaborative(code);
  if (kind === 'j') confirmJoinJam(code);
  if (kind === 'u') openProfile(code);
  return true;
}
listen('deep-link', (e) => handleShareLink(e.payload));

function confirmSongLink(code) {
  let t;
  try { t = trackFromSongLink(code); } catch { toast('That song link is broken', 'error'); return; }
  const m = openModal({
    title: 'Play a shared song?',
    body: h('div', { class: 'link-sheet' },
      thumbEl(t.thumbnail, 'thumb big'),
      h('div', {}, h('b', {}, t.title), h('div', { class: 'meta' }, t.artist || 'Unknown artist'),
        h('div', { class: 'link-sources' }, ...t.versions.map((v) => badge(v.platform, { short: true }))))),
    footer: [
      h('button', { class: 'btn', onclick: () => m.close() }, 'Not now'),
      h('button', { class: 'btn', onclick: () => { m.close(); enqueue([t]); } }, 'Add to queue'),
      h('button', { class: 'btn primary', autofocus: true, onclick: () => { m.close(); playTrack(t); } }, 'Play'),
    ],
  });
}

async function confirmJoinCollaborative(code) {
  const ok = await confirmModal({
    title: 'Join a collaborative playlist?',
    message: 'You were invited to edit a playlist together. Once you join, you can add, remove and reorder its songs, everyone in it sees your changes and your name, and it’s added to Your Library.',
    okLabel: 'Join playlist',
  });
  if (ok) joinCollaborative(code);
}

async function openSharedPlaylist(id) {
  const local = state.playlists.find((p) => p.cloud_id === id);
  if (local) return openPlaylist(local.id);
  if (!cloudOn()) return toast('Online features aren’t available in this build', 'warn');
  try {
    const rows = await db('GET', `playlists?id=eq.${id}&deleted_at=is.null&select=id,title,description,cover,tracks,owner,collaborative,visibility,version,profiles!playlists_owner_fkey(name,avatar)`);
    const row = rows[0];
    if (!row) return toast('This playlist isn’t shared anymore, or it’s private', 'warn');
    rememberRow(row);
    state.remote = { title: row.title, url: PL_LINK(row.id), platform: null, tracks: (row.tracks || []).map(fromCloudTrack), cloud: { id: row.id, owner: row.owner, ownerName: row.profiles?.name, ownerAvatar: row.profiles?.avatar, description: row.description, cover: row.cover } };
    state.remote.tracks = await call('apply_matches', { tracks: state.remote.tracks }).catch(() => state.remote.tracks);
    openPlaylist('remote');
  } catch (e) { toast(errMsg(e), 'error'); }
}

/** Follow a shared playlist: a local copy that keeps getting its updates. */
async function followShared(remote) {
  if (!(await requireAccount({ why: 'Sign in to follow playlists' }))) return;
  try {
    if (remote.cloud.owner !== me()) await db('POST', 'playlist_members', { playlist_id: remote.cloud.id, role: 'follower' }, { prefer: 'resolution=ignore-duplicates' }).catch(() => {});
    const pl = await call('create_playlist', { name: remote.title, tracks: remote.tracks, sourceUrl: null });
    await call('set_playlist_cloud', { id: pl.id, cloudId: remote.cloud.id, role: remote.cloud.owner === me() ? 'owner' : 'follower' });
    await refreshPlaylists({ silent: true });
    syncOneSoon(remote.cloud.id, 0);
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
    const [row] = await db('GET', `playlists?id=eq.${id}&select=id,title,description,cover,tracks,owner,collaborative,visibility,version`);
    rememberRow(row);
    const pl = await call('create_playlist', { name: row.title, tracks: (row.tracks || []).map(fromCloudTrack), sourceUrl: null });
    await call('update_playlist_meta', { id: pl.id, patch: { description: row.description || '', cover: row.cover || '' } }).catch(() => {});
    await call('set_playlist_cloud', { id: pl.id, cloudId: id, role: 'editor' });
    await refreshPlaylists({ silent: true });
    syncOneSoon(id, 0);
    openPlaylist(pl.id);
    toast(`You joined “${row.title}”. Your changes show up for everyone.`, 'ok', 3000);
  } catch (e) { toast(errMsg(e), 'error'); }
}

// Listen Together -----------------------------------------------------------------------
// The session is a database row only its host can change, so guests can't
// take over playback. Members are checked by the server; the host can hand
// over, remove people and approve suggested songs. Presence and reactions
// use a private Realtime channel only members can join.
const REACTIONS = ['🔥', '❤️', '😂', '🕺', '👏', '😮'];
const freshJam = () => ({
  code: null, row: null, host: false, channel: null, live: null, people: [], names: new Map(),
  chat: [], suggestions: [], votes: new Map(), remote: null, remoteAt: 0, applying: false,
  quality: 'waiting', lastSig: '', timers: [], reactedAt: new Map(),
});
let jam = freshJam();

function jamCode() {
  const a = 'abcdefghjkmnpqrstuvwxyz23456789';
  const b = new Uint8Array(12);
  crypto.getRandomValues(b);
  return [...b].map((x) => a[x % a.length]).join('');
}

async function startJam() {
  if (jam.code) { setPanelOpen(true); showPanel('together'); return; }
  if (!(await requireAccount({ why: 'Listen together' }))) return;
  const room = h('input', { type: 'checkbox', checked: false });
  const friends = h('input', { type: 'checkbox', checked: false });
  const approve = h('input', { type: 'checkbox', checked: true });
  const row = (label, help, input) => h('label', { class: 'setting' }, h('div', { class: 'label' }, label, h('small', {}, help)), h('span', { class: 'switch' }, input, h('span')));
  const m = openModal({
    title: 'Start listening together',
    subtitle: 'Everyone hears the same song at the same moment. You control playback; others can chat, suggest songs and vote to skip.',
    body: h('div', { class: 'privacy-list' },
      row('Use my room', 'Your own session link that stays the same every time.', room),
      row('Approve suggested songs', 'Songs guests suggest wait for you instead of going straight into your queue.', approve),
      row('Tell my followers', 'Approved followers get a notification and can join.', friends)),
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), h('button', { class: 'btn primary', autofocus: true, onclick: async (e) => {
      e.target.disabled = true;
      try {
        const code = await rpc('open_jam', { new_code: jamCode(), make_persistent: room.checked, room_title: '', friends: friends.checked });
        if (!approve.checked) await db('PATCH', `jams?code=eq.${code}`, { approve_adds: false });
        m.close();
        await openJam(code);
        copyText(JAM_LINK(code));
        toast('Session started. Invite link copied: send it to friends.', 'ok', 4000);
      } catch (err) { e.target.disabled = false; toast(errMsg(err), 'error'); }
    } }, 'Start')],
  });
}

async function confirmJoinJam(code) {
  if (jam.code === code) { setPanelOpen(true); showPanel('together'); return; }
  const ok = await confirmModal({
    title: 'Join a listening session?',
    message: 'Your player will follow what the host plays (your current song stops). Others in the session see your Discord name. You can leave at any time.',
    okLabel: 'Join',
  });
  if (ok) joinJam(code);
}

async function joinJam(code) {
  if (!LINK_CODES.j.test(code)) { toast('That session code isn’t valid', 'error'); return; }
  if (!(await requireAccount({ why: 'Join the listening session' }))) return;
  if (jam.code) await leaveJam({ silent: true });
  try {
    await rpc('join_jam', { jam_code: code });
    await openJam(code);
    toast('Joined. You’ll hear what the host plays.', 'ok', 2500);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function openJam(code) {
  const [row] = await db('GET', `jams?code=eq.${code}&select=*`);
  if (!row) throw { message: 'This session has ended' };
  jam = freshJam();
  jam.code = code;
  jam.row = row;
  jam.host = row.host === me();
  jam.channel = new Channel(`jam:${code}`, { private: true, presenceKey: me() });
  jam.channel.track({ name: cloud.session.name, avatar: cloud.session.avatar || null });
  jam.channel.on('presence', (p) => {
    const before = jam.people.length;
    jam.people = presenceList(p);
    for (const x of jam.people) jam.names.set(x.key, { name: x.name, avatar: x.avatar });
    if (jam.host && jam.people.length > before) publishJamState(true);
    renderTogether();
  });
  jam.channel.on('b:react', (m) => {
    // Only known reactions, at most a few a second per person.
    if (!REACTIONS.includes(m?.emoji)) return;
    const from = String(m.from || '').slice(0, 40);
    const last = jam.reactedAt.get(from) || 0;
    if (Date.now() - last < 300) return;
    jam.reactedAt.set(from, Date.now());
    floatReaction(m.emoji, from);
  });
  jam.live = new Channel(`jamdb:${code}`, { changes: [
    { event: '*', schema: 'public', table: 'jams', filter: `code=eq.${code}` },
    { event: '*', schema: 'public', table: 'jam_events', filter: `code=eq.${code}` },
    { event: 'DELETE', schema: 'public', table: 'jam_members', filter: `code=eq.${code}` },
  ] });
  jam.live.on('change', onJamChange);
  jam.live.on('status', (s) => { if (s === 'live') refreshJam(); });
  await refreshJam();
  jam.timers.push(setInterval(() => { if (jam.host) publishJamState(false); else checkJamQuality(); }, 4000));
  setPanelOpen(true);
  showPanel('together');
  paintJamButton();
}

/** Everything about the session, fetched (start, reconnect). */
async function refreshJam() {
  if (!jam.code) return;
  try {
    const [[row], members, events] = await Promise.all([
      db('GET', `jams?code=eq.${jam.code}&select=*`),
      db('GET', `jam_members?code=eq.${jam.code}&select=user_id,profiles(name,avatar)`),
      db('GET', `jam_events?code=eq.${jam.code}&select=*&order=id.desc&limit=100`),
    ]);
    if (!row || !row.active) { endedByHost(); return; }
    if (!members.some((m) => m.user_id === me())) { removedFromJam(); return; }
    for (const m of members) if (m.profiles) jam.names.set(m.user_id, m.profiles);
    jam.chat = [];
    jam.suggestions = [];
    jam.votes = new Map();
    for (const e of events.reverse()) addJamEvent(e, false);
    applyJamRow(row);
  } catch { /* offline: the channel reconnects and calls this again */ }
  renderTogether();
}

function onJamChange(d) {
  if (!d || !jam.code) return;
  if (d.table === 'jams') {
    if (d.type === 'DELETE' || d.record?.active === false) { endedByHost(); return; }
    applyJamRow(d.record);
  }
  if (d.table === 'jam_events' && d.type === 'INSERT') addJamEvent(d.record, true);
  if (d.table === 'jam_events' && d.type === 'UPDATE') updateSuggestion(d.record);
  if (d.table === 'jam_members' && d.type === 'DELETE' && d.old_record?.user_id === me()) removedFromJam();
  renderTogether();
}

function applyJamRow(row) {
  if (!row) return;
  const wasHost = jam.host;
  jam.row = row;
  jam.host = row.host === me();
  if (jam.host && !wasHost) { toast('You’re the host now: the session follows what you play', 'info', 3500); publishJamState(true); }
  if (!jam.host && wasHost) toast('You handed over hosting', 'info', 2000);
  if (!jam.host && row.state) {
    jam.remote = row.state;
    jam.remoteAt = row.state_at ? Date.parse(row.state_at) : serverNow();
    applyJamState();
  } else if (!jam.host) {
    jam.remote = null;
    jam.quality = 'waiting';
  }
  paintJamButton();
}

function nameOf(uid) {
  if (uid === me()) return 'You';
  const n = jam.names.get(uid);
  if (!n) fetchName(uid);
  return n?.name || 'Someone';
}
async function fetchName(uid) {
  if (!UUID_RE.test(uid) || jam.names.has(uid)) return;
  jam.names.set(uid, null);
  try { const [p] = await db('GET', `profiles?id=eq.${uid}&select=name,avatar`); if (p) { jam.names.set(uid, p); renderTogether(); } } catch { /* later */ }
}

function addJamEvent(e, live) {
  if (e.kind === 'chat') {
    jam.chat.push(e);
    if (jam.chat.length > 100) jam.chat.shift();
    if (live && e.user_id !== me() && state.panel !== 'together') toast(`${nameOf(e.user_id)}: ${e.payload.text}`, 'info', 3500);
  }
  if (e.kind === 'suggest') {
    jam.suggestions.push(e);
    if (live && jam.host && e.status === 'pending') {
      if (jam.row?.approve_adds === false) answerSuggestion(e, true);
      else toast(`${nameOf(e.user_id)} suggested “${e.payload?.track?.title || 'a song'}”`, 'info', 4000, { action: { label: 'Add', onClick: () => answerSuggestion(e, true) } });
    }
  }
  if (e.kind === 'vote_skip') {
    jam.votes.set(e.user_id, e.payload?.key);
    if (jam.host) checkSkipVotes();
  }
}

function updateSuggestion(rec) {
  const s = jam.suggestions.find((x) => x.id === rec.id);
  if (s) s.status = rec.status;
  if (rec.user_id === me() && !jam.host) toast(rec.status === 'approved' ? `“${rec.payload?.track?.title || 'Your song'}” was added to the session` : `The host passed on “${rec.payload?.track?.title || 'your song'}”`, rec.status === 'approved' ? 'ok' : 'info', 2500);
}

async function answerSuggestion(e, approve) {
  try {
    if (approve) await call('add_to_queue', { tracks: [fromCloudTrack(e.payload.track)], next: false });
    await db('PATCH', `jam_events?id=eq.${e.id}`, { status: approve ? 'approved' : 'rejected' });
    e.status = approve ? 'approved' : 'rejected';
    renderTogether();
  } catch (err) { toast(errMsg(err), 'error'); }
}

/** Guests other than the host who are here now. */
function jamGuests() { return jam.people.filter((p) => p.key !== jam.row?.host); }

function skipVotes() {
  const key = jam.host ? state.playback?.current?.track?.key : jam.remote?.track?.key;
  const n = [...jam.votes.entries()].filter(([uid, k]) => k === key && jamGuests().some((p) => p.key === uid)).length;
  const need = Math.max(1, Math.ceil(jamGuests().length / 2));
  return { key, n, need };
}
function checkSkipVotes() {
  const v = skipVotes();
  if (v.key && v.n >= v.need) {
    jam.votes.clear();
    call('next_track').catch(() => {});
    toast('Skipped by vote', 'info', 2000);
  }
}
async function voteSkip() {
  const key = jam.remote?.track?.key;
  if (!key) return;
  try { await db('POST', 'jam_events', { code: jam.code, kind: 'vote_skip', payload: { key } }); } catch (e) { toast(errMsg(e), 'error'); }
}

function jamStateNow() {
  const p = state.playback;
  const t = p?.current?.track;
  return {
    track: t ? slimTrack(t) : null,
    playing: p?.status === 'playing',
    position: p?.position || 0,
    upcoming: state.queue.items.slice((state.queue.current ?? -1) + 1, (state.queue.current ?? -1) + 6).map((i) => ({ title: i.track.title, artist: i.track.artist })),
  };
}

/** Host: write the state on every real change, and every 16 s while playing (drift). */
let jamLastWrite = 0;
async function publishJamState(force) {
  if (!jam.code || !jam.host || !state.playback) return;
  const st = jamStateNow();
  const sig = `${st.track?.key}|${st.playing}|${st.upcoming.map((u) => u.title).join(',')}`;
  const stale = st.playing && Date.now() - jamLastWrite > 16000;
  if (!force && sig === jam.lastSig && !stale) return;
  jam.lastSig = sig;
  jamLastWrite = Date.now();
  try { await db('PATCH', `jams?code=eq.${jam.code}`, { state: st }); } catch { jam.lastSig = ''; }
}
// Host: share every real change right away.
let jamLastPlay = '';
function onPlaybackForJam() {
  if (!jam.code || !jam.host) return;
  const p = state.playback;
  const sig = `${p?.current?.track?.key}|${p?.status}`;
  if (sig !== jamLastPlay) { jamLastPlay = sig; publishJamState(true); }
}

/** Where the host's song is now, by the server's clock. */
function jamTarget() {
  const st = jam.remote;
  if (!st) return null;
  return st.position + (st.playing ? Math.max(0, serverNow() - jam.remoteAt) / 1000 : 0);
}

async function applyJamState() {
  const st = jam.remote;
  renderTogether();
  if (jam.applying || !st?.track) return;
  const p = state.playback;
  const cur = p?.current?.track?.key;
  jam.applying = true;
  jam.quality = 'catching';
  try {
    if (cur !== st.track.key) {
      await call('play_track', { track: fromCloudTrack(st.track), platform: null });
      // Seek once it's loaded (a few hundred ms usually).
      for (let i = 0; i < 40; i++) {
        await new Promise((r) => setTimeout(r, 250));
        if (state.playback?.status === 'playing' && state.playback.current?.track?.key === st.track.key) break;
      }
      const now = jamTarget();
      if (now > 2) await call('seek', { position: now });
    } else if (st.playing && p.status !== 'playing') {
      await call('resume');
    } else if (!st.playing && p.status === 'playing') {
      await call('pause');
    }
    const target = jamTarget();
    if (cur === st.track.key && Math.abs((state.playback?.position || 0) - target) > 1.5) await call('seek', { position: target });
    if (!st.playing && cur === st.track.key) await call('pause');
  } catch { /* the next state fixes it */ }
  jam.applying = false;
  checkJamQuality();
}

/** Guest: how close to the host this player is. */
function checkJamQuality() {
  if (!jam.code || jam.host) return;
  const prev = jam.quality;
  const st = jam.remote;
  const p = state.playback;
  if (!st?.track) jam.quality = 'waiting';
  else if (jam.applying || p?.status === 'loading') jam.quality = 'catching';
  else if (p?.current?.track?.key !== st.track.key || (st.playing !== (p?.status === 'playing'))) jam.quality = 'off';
  else {
    const d = Math.abs((p?.position || 0) - jamTarget());
    jam.quality = d < 0.75 ? 'sync' : d < 2.5 ? 'close' : 'off';
  }
  // Drifted away (seeked, paused here): put it back.
  if (jam.quality === 'off' && !jam.applying && st?.track) applyJamState();
  if (prev !== jam.quality) renderTogether();
}

function jamAdd(track) {
  if (!jam.code) return;
  if (jam.host) { enqueue([track]); return; }
  db('POST', 'jam_events', { code: jam.code, kind: 'suggest', status: 'pending', payload: { track: slimTrack(track) } })
    .then(() => toast(jam.row?.approve_adds === false ? `Added “${track.title}” to the session` : `Suggested “${track.title}”: the host decides`, 'ok', 2200))
    .catch((e) => toast(errMsg(e), 'error'));
}

async function sendChat(text) {
  const t = text.trim().slice(0, 300);
  if (!t || !jam.code) return;
  try { await db('POST', 'jam_events', { code: jam.code, kind: 'chat', payload: { text: t } }); } catch (e) { toast(errMsg(e), 'error'); }
}

function sendReaction(emoji) {
  if (!REACTIONS.includes(emoji)) return;
  jam.channel?.broadcast('react', { emoji, from: cloud.session.name.slice(0, 40) });
  floatReaction(emoji, 'You');
}
function floatReaction(emoji, from) {
  const el = h('div', { class: 'reaction', 'aria-hidden': 'true' }, emoji, h('small', {}, from));
  el.style.left = `${55 + Math.random() * 30}%`;
  document.body.append(el);
  setTimeout(() => el.remove(), 2600);
}

async function handOff(uid) {
  try { await rpc('jam_handoff', { jam_code: jam.code, new_host: uid }); } catch (e) { toast(errMsg(e), 'error'); }
}
async function kick(uid) {
  if (!(await confirmModal({ title: `Remove ${nameOf(uid)}?`, message: 'They leave the session and can’t join it again with this link.', okLabel: 'Remove', danger: true }))) return;
  try { await rpc('jam_kick', { jam_code: jam.code, who: uid }); toast(`${nameOf(uid)} was removed`, 'info', 2000); } catch (e) { toast(errMsg(e), 'error'); }
}

function endedByHost() {
  if (!jam.code) return;
  leaveJam({ silent: true, server: false });
  toast('The host ended the session', 'info', 2500);
}
function removedFromJam() {
  if (!jam.code) return;
  leaveJam({ silent: true, server: false });
  toast('The host removed you from the session', 'warn', 3000);
}

/**
 * Leave (or, with `end`, end for everyone). The channels are taken from the
 * session before it's cleared, so they really close.
 */
async function leaveJam({ end = false, silent = false, server = true } = {}) {
  const { code, channel, live, timers, host } = jam;
  for (const t of timers) clearInterval(t);
  jam = freshJam();
  channel?.close();
  live?.close();
  if (code && server && signedIn()) {
    try {
      if (end && host) await rpc('end_jam', { jam_code: code });
      else await db('DELETE', `jam_members?code=eq.${code}&user_id=eq.${me()}`);
    } catch { /* gone already */ }
  }
  renderTogether();
  paintJamButton();
  if (!silent) toast(end ? 'Session ended' : 'You left the session', 'info', 1800);
  if (state.panel === 'together') showPanel('queue');
}

function hostLeaveMenu(anchor) {
  const guests = jamGuests();
  openMenu(anchor, [
    { label: 'End for everyone', icon: 'x', onClick: () => leaveJam({ end: true }) },
    ...(guests.length ? [{ label: 'Hand over and leave', heading: true }, ...guests.map((g) => ({ label: g.name || 'Someone', icon: 'user', onClick: async () => { await handOff(g.key); leaveJam(); } }))] : []),
  ]);
}

function paintJamButton() {
  const b = $('#btn-jam');
  if (b) { b.classList.toggle('on', !!jam.code); b.hidden = !cloudOn(); b.setAttribute('aria-pressed', String(!!jam.code)); }
}

const QUALITY = {
  waiting: ['Waiting for the host to play something…', ''],
  catching: ['Catching up…', 'warn'],
  sync: ['In sync', 'ok'],
  close: ['Almost in sync', 'ok'],
  off: ['Out of sync · fixing', 'bad'],
};

function renderTogether() {
  const box = $('#together-panel');
  if (!box || box.hidden) return;
  if (!jam.code) {
    box.replaceChildren(h('div', { class: 'empty' },
      h('p', {}, 'Listen together'),
      h('p', { class: 'small' }, 'Start a session and send the link: everyone hears the same song at the same moment, can chat, suggest songs and vote to skip.'),
      h('div', { class: 'actions center mt-12' },
        h('button', { class: 'btn primary', onclick: startJam }, 'Start a session'),
        h('button', { class: 'btn', onclick: async () => {
          const c = await promptModal({ title: 'Join a session', placeholder: 'Code or link', okLabel: 'Join' });
          if (!c) return;
          if (!handleShareLink(c)) joinJam(c.trim().toLowerCase());
        } }, 'Join'))));
    return;
  }
  const st = jam.remote;
  const hostId = jam.row?.host;
  // Until presence arrives, at least show yourself.
  const people = jam.people.some((p) => p.key === me()) ? jam.people : [{ key: me(), name: cloud.session.name, avatar: cloud.session.avatar }, ...jam.people];
  const [qText, qCls] = jam.host ? ['You control playback', 'ok'] : QUALITY[jam.quality] || QUALITY.waiting;
  const votes = skipVotes();

  const chatList = h('div', { class: 'jam-chat', role: 'log', 'aria-live': 'polite', 'aria-label': 'Session chat' },
    ...jam.chat.slice(-50).map((c) => h('div', { class: `chat-line${c.user_id === me() ? ' mine' : ''}` }, h('b', {}, nameOf(c.user_id)), ' ', c.payload?.text || '')));
  // One input per session: re-rendering the panel keeps what's being typed.
  if (!jam.input) {
    jam.input = h('input', { class: 'input', placeholder: 'Say something…', maxlength: 300, 'aria-label': 'Chat message' });
    jam.input.addEventListener('keydown', (e) => { if (e.key === 'Enter' && jam.input.value.trim()) { sendChat(jam.input.value); jam.input.value = ''; } });
  }
  const input = jam.input;
  const hadFocus = document.activeElement === input;
  const pending = jam.suggestions.filter((s) => s.status === 'pending');

  box.replaceChildren(...[
    h('div', { class: 'jam-head' },
      h('div', {}, h('b', {}, jam.host ? 'You’re hosting' : `Listening with ${nameOf(hostId)}`), h('div', { class: `jam-quality ${qCls}`, role: 'status' }, h('span', { class: 'sync-dot' }), qText)),
      h('span', { class: 'meta' }, `${people.length} ${people.length === 1 ? 'person' : 'people'}`)),
    h('div', { class: 'jam-people' }, ...people.map((p) => h('div', { class: 'jam-person' }, avatarEl(p.avatar, p.name), h('span', {}, p.key === me() ? `${p.name} (you)` : p.name), p.key === hostId ? h('em', {}, 'host') : null,
      jam.host && p.key !== me() ? h('button', { class: 'icon-btn small', title: `Options for ${p.name}`, html: icon('dots'), onclick: (e) => openMenu(e.currentTarget, [
        { label: 'Make host', icon: 'user', onClick: () => handOff(p.key) },
        { label: 'Remove from session', icon: 'x', onClick: () => kick(p.key) }]) }) : null))),
    h('div', { class: 'actions' },
      h('button', { class: 'btn small', html: `${icon('link')}Copy invite link`, onclick: () => { copyText(JAM_LINK(jam.code)); toast('Invite link copied', 'ok', 1500); } }),
      jam.host
        ? h('button', { class: 'btn small danger-ghost', onclick: (e) => hostLeaveMenu(e.currentTarget) }, 'End or hand over')
        : h('button', { class: 'btn small danger-ghost', onclick: () => leaveJam() }, 'Leave')),
    jam.host ? h('label', { class: 'setting compact' }, h('div', { class: 'label' }, 'Approve suggested songs'),
      (() => { const i = h('input', { type: 'checkbox', checked: jam.row?.approve_adds !== false }); i.addEventListener('change', () => db('PATCH', `jams?code=eq.${jam.code}`, { approve_adds: i.checked }).catch((e) => toast(errMsg(e), 'error'))); return h('span', { class: 'switch' }, i, h('span')); })()) : null,
    !jam.host && st?.track ? h('div', { class: 'npv-card' }, h('h4', {}, 'Now playing'), h('div', { class: 'npv-source' }, `${st.track.title} — ${st.track.artist}`),
      h('div', { class: 'actions mt-8' },
        h('button', { class: 'btn small', disabled: votes.n && jam.votes.get(me()) === votes.key, onclick: voteSkip }, jam.votes.get(me()) === votes.key ? 'You voted to skip' : 'Vote to skip'),
        votes.n ? h('span', { class: 'meta' }, `${votes.n} of ${votes.need} votes`) : null)) : null,
    jam.host && votes.n ? h('div', { class: 'meta' }, `${votes.n} of ${votes.need} votes to skip this song`) : null,
    !jam.host && st?.upcoming?.length ? h('div', { class: 'npv-card' }, h('h4', {}, 'Up next'), h('div', { class: 'npv-source' }, ...st.upcoming.map((u) => h('div', {}, `${u.title} — ${u.artist}`)))) : null,
    jam.host && pending.length ? h('div', { class: 'npv-card' }, h('h4', {}, `Suggestions (${pending.length})`),
      ...pending.map((s) => h('div', { class: 'suggestion' },
        h('div', { class: 'grow' }, h('b', {}, s.payload?.track?.title || 'Song'), h('div', { class: 'meta' }, `${s.payload?.track?.artist || ''} · from ${nameOf(s.user_id)}`)),
        h('button', { class: 'btn small primary', onclick: () => answerSuggestion(s, true) }, 'Add'),
        h('button', { class: 'btn small', onclick: () => answerSuggestion(s, false) }, 'Pass')))) : null,
    h('div', { class: 'jam-hint meta' }, jam.host ? 'Guests follow what you play. Songs they suggest show up here.' : 'Suggest songs with ⋯ → Suggest for the session. The host controls playback.'),
    h('div', { class: 'reactions-bar', role: 'group', 'aria-label': 'Reactions' }, ...REACTIONS.map((e) => h('button', { 'aria-label': `React ${e}`, onclick: () => sendReaction(e) }, e))),
    chatList,
    input].filter(Boolean));
  chatList.scrollTop = chatList.scrollHeight;
  if (hadFocus) input.focus();
}

// Friends & follow requests ----------------------------------------------------------------
const friends = { lastPublished: '', lastPos: null, lastPlaying: false, lastAt: 0, tab: 'friends' };

async function follow(uid) {
  if (!(await requireAccount({ why: 'Sign in to follow people' }))) return false;
  try { await db('POST', 'follows', { followee: uid }, { prefer: 'resolution=ignore-duplicates' }); return true; } catch (e) { toast(errMsg(e), 'error'); return false; }
}
async function unfollow(uid) {
  try { await db('DELETE', `follows?follower=eq.${me()}&followee=eq.${uid}`); return true; } catch (e) { toast(errMsg(e), 'error'); return false; }
}
async function answerRequest(uid, accept) {
  try {
    if (accept) await db('PATCH', `follows?follower=eq.${uid}&followee=eq.${me()}`, { status: 'accepted' });
    else await db('DELETE', `follows?follower=eq.${uid}&followee=eq.${me()}`);
    db('PATCH', `notifications?user_id=eq.${me()}&kind=eq.follow_request&actor=eq.${uid}`, { read: true }).catch(() => {});
    toast(accept ? 'Request accepted' : 'Request declined', 'ok', 1500);
    refreshNotifications();
    renderFriends();
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function renderFriends() {
  const box = $('#friends-panel');
  if (!box || box.hidden) return;
  clearInterval(friends.liveTimer);
  if (!cloudOn()) { box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'Not available in this build.'))); return; }
  if (!signedIn()) {
    box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'See what your friends are listening to'), h('p', { class: 'small' }, 'Follow friends (they approve it) and their current song shows up here.'),
      h('button', { class: 'btn primary mt-12', onclick: () => requireAccount({ why: 'Sign in to see friends' }) }, 'Sign in with Discord')));
    return;
  }
  const unread = notif.list.filter((n) => n.kind === 'song_shared' && !n.read).length;
  const tab = (id, label) => h('button', { role: 'tab', 'aria-selected': String(friends.tab === id), class: friends.tab === id ? 'active' : '', onclick: () => { friends.tab = id; renderFriends(); } }, label);
  const tabs = h('div', { class: 'friends-tabs', role: 'tablist' }, tab('friends', 'Friends'), tab('inbox', unread ? `Inbox · ${unread}` : 'Inbox'), tab('board', 'This week'));
  const content = h('div', { class: 'friends-content' });
  box.replaceChildren(tabs, content);
  if (friends.tab === 'inbox') {
    renderInbox(content);
    // Song notes count as seen once the inbox is open.
    const ids = notif.list.filter((n) => n.kind === 'song_shared' && !n.read);
    if (ids.length) { db('PATCH', `notifications?user_id=eq.${me()}&kind=eq.song_shared&read=eq.false`, { read: true }).catch(() => {}); ids.forEach((n) => { n.read = true; }); notif.unread = notif.list.filter((n) => !n.read).length; paintAccountButton(); }
    return;
  }
  if (friends.tab === 'board') { renderLeaderboard(content); return; }

  const search = h('input', { class: 'input', placeholder: 'Find people by name', spellcheck: 'false', 'aria-label': 'Find people by name' });
  const results = h('div', { class: 'people-results' });
  search.addEventListener('input', debounce(async () => {
    const q = search.value.trim();
    if (q.length < 2) { results.replaceChildren(); return; }
    try {
      const rows = (await rpc('search_profiles', { q })).slice(0, 8);
      results.replaceChildren(...rows.filter((r) => r.id !== me()).map((r) => personRow(r)));
    } catch { /* offline */ }
  }, 300));
  const requests = h('div', { class: 'friend-requests' });
  const list = h('div', { class: 'friends-list' }, ...skeletonFriends(3));
  content.replaceChildren(h('div', { class: 'friends-tools' }, search, h('button', { class: 'icon-btn', title: 'Copy link to your profile', 'aria-label': 'Copy link to your profile', html: icon('link'), onclick: () => { copyText(PROFILE_LINK(me())); toast('Profile link copied', 'ok', 1500); } })), results, requests, list);
  try {
    const [incoming, follows] = await Promise.all([
      db('GET', `follows?followee=eq.${me()}&status=eq.pending&select=follower,profiles!follows_follower_fkey(*)`),
      db('GET', `follows?follower=eq.${me()}&select=followee,status,profiles!follows_followee_fkey(*)`),
    ]);
    if (incoming.length) {
      requests.replaceChildren(h('div', { class: 'section-mini' }, `Follow requests (${incoming.length})`), ...incoming.map((r) => h('div', { class: 'friend' }, decoratedAvatar(r.profiles?.avatar, r.profiles?.name, r.profiles?.decoration),
        h('button', { class: 'friend-name', onclick: () => openProfile(r.follower) }, r.profiles?.name || 'Someone'),
        h('button', { class: 'btn small primary', onclick: () => answerRequest(r.follower, true) }, 'Accept'),
        h('button', { class: 'btn small', onclick: () => answerRequest(r.follower, false) }, 'Decline'))));
    }
    const accepted = follows.filter((f) => f.status === 'accepted');
    const asked = follows.filter((f) => f.status === 'pending');
    const ids = accepted.map((f) => f.followee);
    const playing = ids.length ? await db('GET', `now_playing?user_id=in.(${ids.join(',')})&select=*`) : [];
    const byUser = new Map(playing.map((p) => [p.user_id, p]));
    const order = { listening: 0, online: 1, offline: 2 };
    const rows = accepted.map((f) => ({ ...f.profiles, np: byUser.get(f.followee) }))
      .sort((a, b) => (order[presenceOf(a.np)] - order[presenceOf(b.np)]) || (b.np?.updated_at || '').localeCompare(a.np?.updated_at || ''));
    const out = [];
    if (!rows.length && !asked.length) out.push(h('p', { class: 'meta pad-12' }, 'You don’t follow anyone yet. Search above, or send friends your profile link.'));
    out.push(...rows.map((r) => friendRow(r)));
    if (asked.length) out.push(h('div', { class: 'section-mini' }, 'Waiting for approval'), ...asked.map((f) => h('div', { class: 'friend' }, decoratedAvatar(f.profiles?.avatar, f.profiles?.name, f.profiles?.decoration),
      h('button', { class: 'friend-name', onclick: () => openProfile(f.followee) }, f.profiles?.name || 'Someone'),
      h('button', { class: 'btn small', onclick: async () => { if (await unfollow(f.followee)) renderFriends(); } }, 'Cancel'))));
    list.replaceChildren(...out);
    // Live progress bars move on their own between updates.
    friends.liveTimer = setInterval(() => {
      if (box.hidden || !list.isConnected) { clearInterval(friends.liveTimer); return; }
      for (const bar of list.querySelectorAll('.friend-progress[data-uid]')) {
        const np = byUser.get(bar.dataset.uid);
        const pos = livePosition(np);
        if (pos != null && np.duration) bar.firstChild.style.transform = `scaleX(${Math.min(1, pos / np.duration)})`;
      }
    }, 1000);
  } catch (e) { list.replaceChildren(h('p', { class: 'meta' }, errMsg(e))); }
}

function skeletonFriends(n) {
  return Array.from({ length: n }, () => h('div', { class: 'friend skeleton' }, h('span', { class: 'avatar sk' }), h('div', { class: 'grow' }, h('div', { class: 'sk w-60' }), h('div', { class: 'sk w-40' }))));
}

function personRow(r) {
  const btn = h('button', { class: 'btn small' }, 'Follow');
  btn.addEventListener('click', async () => {
    if (await follow(r.id)) { btn.textContent = 'Requested'; btn.disabled = true; renderFriends(); }
  });
  return h('div', { class: 'friend' }, decoratedAvatar(r.avatar, r.name, r.decoration), h('button', { class: 'friend-name', onclick: () => openProfile(r.id) }, r.name), btn);
}

const PRESENCE_TEXT = { listening: 'Listening now', online: 'Online', offline: 'Offline' };
function friendRow(r) {
  const np = r.np;
  const pres = presenceOf(np);
  const t = np?.track;
  const live = pres === 'listening' && t;
  const pos = livePosition(np);
  const av = h('span', { class: `presence ${pres}`, title: PRESENCE_TEXT[pres] }, decoratedAvatar(r.avatar, r.name, r.decoration), h('i', { class: 'p-dot', 'aria-label': PRESENCE_TEXT[pres] }));
  const tools = h('div', { class: 'friend-tools' },
    live ? h('button', { class: 'icon-btn small', title: `Listen along with ${r.name}`, html: icon('headphones'), onclick: () => listenAlong(r.id, r.name) }) : null,
    t ? h('button', { class: 'icon-btn small', title: 'Play this song', html: icon('play'), onclick: () => playTrack(fromCloudTrack(t)) }) : null);
  return h('div', { class: `friend${along.uid === r.id ? ' along' : ''}` }, av,
    h('div', { class: 'friend-main' },
      h('button', { class: 'friend-name', onclick: () => openProfile(r.id) }, r.name),
      t ? h('div', { class: 'friend-track', title: `${t.title} · ${t.artist}` }, `${t.title} · ${t.artist}`) : h('span', { class: 'meta' }, pres === 'offline' ? 'Nothing played lately' : 'Online, not playing'),
      live && np.duration ? h('div', { class: 'friend-progress', dataset: { uid: r.id } }, h('i', { style: `transform:scaleX(${Math.min(1, (pos || 0) / np.duration)})` })) : null),
    np && !live ? h('span', { class: 'meta friend-when' }, pres === 'online' ? 'online' : timeAgo(Date.parse(np.updated_at))) : null,
    tools);
}

/** Tell approved followers what's playing, where in the song, and that the
 * app is open (only if sharing is on). Real changes go out right away,
 * seeks too; `force` (the minute heartbeat) refreshes position and presence. */
function publishActivity(force = false) {
  if (!signedIn() || !state.settings?.share_activity || !cloud.profile?.share_activity) return;
  const p = state.playback;
  const t = p?.current?.track;
  if (p?.status === 'loading') return;
  const playing = p?.status === 'playing';
  const pos = p?.position || 0;
  // Where followers think we are, from the last update.
  const expected = friends.lastPos == null ? null : friends.lastPos + (friends.lastPlaying ? (Date.now() - friends.lastAt) / 1000 : 0);
  const seeked = expected != null && Math.abs(pos - expected) > 4;
  const sig = `${t?.key}|${playing}`;
  if (!force && sig === friends.lastPublished && !seeked) return;
  friends.lastPublished = sig;
  Object.assign(friends, { lastPos: pos, lastPlaying: playing, lastAt: Date.now() });
  const now = new Date().toISOString();
  return db('POST', 'now_playing?on_conflict=user_id', {
    user_id: me(), track: t ? slimTrack(t) : null, playing, position: t ? Math.round(pos * 10) / 10 : null,
    duration: t ? p.duration || t.duration || null : null, updated_at: now, online_at: now,
  }, { prefer: 'resolution=merge-duplicates' })
    .catch(() => { friends.lastPublished = ''; }); // try again on the next update
}

// Notifications ---------------------------------------------------------------------------------
const notif = { list: [], unread: 0, channel: null, user: null };
const NOTIF_SELECT = 'select=*,actor_profile:profiles!notifications_actor_fkey(name,avatar)';

async function startNotifications() {
  if (!signedIn()) {
    notif.channel?.close();
    Object.assign(notif, { list: [], unread: 0, channel: null, user: null });
    paintAccountButton();
    return;
  }
  if (notif.user === me()) return;
  notif.channel?.close();
  notif.user = me();
  notif.channel = new Channel(`notes:${me()}`, { changes: [{ event: 'INSERT', schema: 'public', table: 'notifications', filter: `user_id=eq.${me()}` }] });
  notif.channel.on('change', async (d) => {
    if (!d?.record) return;
    const [n] = await db('GET', `notifications?id=eq.${d.record.id}&${NOTIF_SELECT}`).catch(() => [d.record]);
    if (!n) return;
    notif.list.unshift(n);
    notif.unread++;
    paintAccountButton();
    const a = notifAction(n);
    toast(notifText(n), 'info', a ? 8000 : 4000, a ? { action: a } : undefined);
  });
  // After a reconnect: whatever arrived while the connection was down.
  let first = true;
  notif.channel.on('status', (s) => { if (s === 'live' && !first) refreshNotifications(); if (s === 'live') first = false; });
  refreshNotifications();
}

async function refreshNotifications() {
  if (!signedIn()) return;
  try {
    notif.list = await db('GET', `notifications?${NOTIF_SELECT}&order=created_at.desc&limit=50`);
    notif.unread = notif.list.filter((n) => !n.read).length;
  } catch { /* offline */ }
  paintAccountButton();
}

function notifText(n) {
  const who = n.actor_profile?.name || 'Someone';
  switch (n.kind) {
    case 'follow_request': return `${who} wants to follow you`;
    case 'follow_accepted': return `${who} accepted your follow request`;
    case 'playlist_activity': return `${who} changed “${n.data?.title || 'a playlist'}”`;
    case 'jam_started': return `${who} started a listening session`;
    case 'song_shared': return `${who} sent you “${n.data?.title || 'a song'}”`;
    default: return 'Something happened';
  }
}
function notifAction(n) {
  if (n.kind === 'follow_request') return { label: 'Accept', onClick: () => answerRequest(n.actor, true) };
  if (n.kind === 'jam_started' && LINK_CODES.j.test(n.data?.code || '')) return { label: 'Join', onClick: () => confirmJoinJam(n.data.code) };
  if (n.kind === 'playlist_activity' && n.data?.playlist) return { label: 'Open', onClick: () => openSharedPlaylist(n.data.playlist) };
  if (n.kind === 'follow_accepted') return { label: 'View', onClick: () => openProfile(n.actor) };
  if (n.kind === 'song_shared') return { label: 'Open', onClick: () => { friends.tab = 'inbox'; setPanelOpen(true); showPanel('friends'); renderFriends(); } };
  return null;
}

function openNotifications() {
  const list = h('div', { class: 'notif-list' });
  const paint = () => {
    if (!notif.list.length) { list.replaceChildren(h('p', { class: 'meta pad-12' }, 'Nothing yet. Follow requests, friends’ sessions and changes to your shared playlists show up here.')); return; }
    list.replaceChildren(...notif.list.map((n) => {
      const a = notifAction(n);
      return h('div', { class: `notif${n.read ? '' : ' unread'}` }, avatarEl(n.actor_profile?.avatar, n.actor_profile?.name),
        h('div', { class: 'grow' }, h('div', {}, notifText(n)), h('div', { class: 'meta' }, timeAgo(Date.parse(n.created_at)))),
        n.kind === 'follow_request' ? h('button', { class: 'btn small', onclick: () => { answerRequest(n.actor, false); m.close(); } }, 'Decline') : null,
        a ? h('button', { class: `btn small${n.kind === 'follow_request' ? ' primary' : ''}`, onclick: () => { m.close(); a.onClick(); } }, a.label) : null);
    }));
  };
  paint();
  const m = openModal({
    title: 'Notifications',
    body: list,
    footer: [
      notif.list.length ? h('button', { class: 'btn', onclick: async () => { await db('DELETE', `notifications?user_id=eq.${me()}`).catch(() => {}); notif.list = []; notif.unread = 0; paint(); paintAccountButton(); } }, 'Clear all') : null,
      h('button', { class: 'btn primary', onclick: () => m.close() }, 'Done')].filter(Boolean),
  });
  if (notif.unread) {
    db('PATCH', `notifications?user_id=eq.${me()}&read=eq.false`, { read: true }).catch(() => {});
    notif.unread = 0;
    for (const n of notif.list) n.read = true;
    paintAccountButton();
  }
}

// Profiles ------------------------------------------------------------------------------------
// A Discord-style header (their banner, or their accent color), and
// tabs for public playlists, followers and following.
const DISCORD_BANNER = /^https:\/\/cdn\.discordapp\.com\/banners\/[0-9]+\/(a_)?[0-9a-f]+\.(png|gif|webp)(\?size=[0-9]+)?$/;
const hexColor = (n) => `#${Number(n).toString(16).padStart(6, '0')}`;

let profileSeq = 0;
const PUBLIC_PROFILE = 'id,name,avatar,banner,accent_color,decoration';
async function openProfile(userId, { push = true, tab = 'playlists' } = {}) {
  if (!cloudOn() || !UUID_RE.test(userId)) return;
  const seq = ++profileSeq;
  showView('profile', { push, reopen: () => openProfile(userId, { push: false, tab }) });
  const body = $('#profile-body');
  body.replaceChildren(loadingBox('Loading profile', h('div', { class: 'profile-head' }, h('div', { class: 'profile-banner sk' }), h('div', { class: 'profile-id' }, h('div', { class: 'profile-avatar sk' }), h('div', { class: 'grow' }, h('div', { class: 'sk sk-title w-40' }), h('div', { class: 'sk w-20' }))))));
  try {
    const signed = signedIn();
    const [[p], lists, followers, following, mine, stats, nps, badges, followsMe] = await Promise.all([
      // Signed out, only a profile's public fields are readable.
      db('GET', `profiles?id=eq.${userId}&select=${signed ? '*' : PUBLIC_PROFILE}`),
      db('GET', `playlists?owner=eq.${userId}&visibility=eq.public&deleted_at=is.null&select=id,title,cover,tracks->0->thumbnail&order=updated_at.desc`),
      db('GET', `follows?followee=eq.${userId}&select=follower,status,created_at,profiles!follows_follower_fkey(id,name,avatar,decoration)&order=created_at.desc`),
      db('GET', `follows?follower=eq.${userId}&status=eq.accepted&select=followee,created_at,profiles!follows_followee_fkey(id,name,avatar,decoration)&order=created_at.desc`),
      signed ? db('GET', `follows?follower=eq.${me()}&select=followee,status`) : Promise.resolve([]),
      signed ? db('GET', `profile_stats?user_id=eq.${userId}&select=*`).catch(() => []) : Promise.resolve([]),
      signed ? db('GET', `now_playing?user_id=eq.${userId}&select=*`).catch(() => []) : Promise.resolve([]),
      badgesOf(userId),
      signed && userId !== me() ? db('GET', `follows?follower=eq.${userId}&followee=eq.${me()}&status=eq.accepted&select=follower`).catch(() => []) : Promise.resolve([]),
    ]);
    if (seq !== profileSeq) return;
    if (!p) { body.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'This profile doesn’t exist.'))); return; }
    const isMe = signed && p.id === me();
    const accepted = followers.filter((f) => f.status === 'accepted');
    const myStatus = new Map(mine.map((f) => [f.followee, f.status]));
    const st = stats[0] || null;
    const np = nps[0] || null;
    // People you follow who also follow them.
    const mutuals = accepted.filter((f) => myStatus.get(f.follower) === 'accepted').map((f) => f.profiles).filter(Boolean);
    let match = null;
    const matchEl = h('span', { class: 'match-chip', hidden: true });
    if (!isMe && st?.top_artists?.length) tasteMatch(st.top_artists).then((m) => { if (m != null) { match = m; matchEl.textContent = `${m}% taste match`; matchEl.hidden = false; } });

    // Header: banner, decorated avatar, name, pronouns, badges, bio, counts.
    const banner = h('div', { class: 'profile-banner' });
    const accent = p.accent_color != null ? hexColor(p.accent_color) : null;
    // Their upload (a GIF moves) or their Discord banner.
    if (typeof p.banner === 'string' && (DISCORD_BANNER.test(p.banner) || MEDIA_RE.test(p.banner))) banner.style.backgroundImage = `url("${mediaUrl(p.banner)}")`;
    else banner.classList.add('no-image');
    if (accent) banner.style.setProperty('--accent-c', accent);
    const avatar = h('div', { class: 'profile-avatar' });
    if (mediaUrl(p.avatar)) avatar.append(h('img', { src: mediaUrl(p.avatar), 'data-px': 300, alt: '', referrerpolicy: 'no-referrer' }));
    else { avatar.textContent = (p.name || '?')[0].toUpperCase(); if (accent) avatar.style.background = accent; }
    const avatarBox = h('div', { class: 'profile-avatar-box' }, avatar);
    if (typeof p.decoration === 'string' && DECORATION_RE.test(p.decoration)) avatarBox.append(h('img', { class: 'profile-deco', src: p.decoration, alt: '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }));

    const followBtn = (uid, name) => {
      if (!signedIn() || uid === me()) return null;
      const stt = myStatus.get(uid);
      const b = h('button', { class: `btn small${stt ? '' : ' primary'}` }, !stt ? (uid === p.id && followsMe.length ? 'Follow back' : 'Follow') : stt === 'accepted' ? 'Following' : 'Requested');
      b.addEventListener('click', async () => {
        b.disabled = true;
        if (stt) { if (await unfollow(uid)) myStatus.delete(uid); }
        else if (await follow(uid)) { myStatus.set(uid, 'pending'); toast(`Request sent to ${name}`, 'ok', 2000); }
        render();
      });
      return b;
    };
    const tabBtn = (id, label) => h('button', { role: 'tab', 'aria-selected': String(tab === id), class: tab === id ? 'active' : '', onclick: () => { tab = id; render(); } }, label);
    const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;
    const personList = (rows, empty) => (rows.length
      ? h('div', { class: 'people-grid' }, ...rows.map((r) => h('div', { class: 'person-card' },
        h('button', { class: 'person-open', onclick: () => openProfile(r.id) }, decoratedAvatar(r.avatar, r.name, r.decoration, 'avatar lg'), h('b', {}, r.name || 'Someone'), r.id === me() ? h('span', { class: 'meta' }, 'You') : null),
        followBtn(r.id, r.name))))
      : h('p', { class: 'meta' }, empty));

    // Listening now (approved followers, when they share what they play).
    const nowCard = () => {
      if (!np?.track || presenceOf(np) !== 'listening') return null;
      const t = fromCloudTrack(np.track);
      const pos = livePosition(np);
      return h('div', { class: 'now-card' },
        thumbEl(t.thumbnail, 'thumb big'),
        h('div', { class: 'grow min-w-0' },
          h('div', { class: 'now-kicker' }, h('span', { class: 'sync-dot' }), 'Listening now'),
          h('b', { class: 'ellipsis' }, t.title), h('div', { class: 'meta ellipsis' }, t.artist || ''),
          np.duration ? h('div', { class: 'friend-progress' }, h('i', { style: `transform:scaleX(${Math.min(1, (pos || 0) / np.duration)})` })) : null),
        isMe ? null : h('button', { class: 'btn small primary', html: `${icon('headphones')}Listen along`, onclick: () => listenAlong(p.id, p.name) }));
    };

    const listening = () => {
      if (!st) return h('p', { class: 'meta' }, isMe ? 'Turn on “Share my listening” in Settings → Account to show your top artists and songs here.' : `${p.name} doesn’t share their listening${signedIn() ? ' with you' : ''}.`);
      const artistTiles = (st.top_artists || []).map((a, i) => h('button', { class: 'top-artist', style: `--i:${i}`, onclick: () => openArtist(a.name) },
        h('div', { class: 'ta-img' }, a.thumbnail ? h('img', { src: a.thumbnail, 'data-px': 200, alt: '', referrerpolicy: 'no-referrer' }) : h('span', { html: icon('user') })), h('b', {}, a.name)));
      const songList = (arr, label) => (arr?.length ? h('div', {}, h('div', { class: 'section-title' }, label),
        h('div', { class: 'track-list', role: 'listbox', 'aria-label': label }, ...arr.map((t, i) => trackRow(fromCloudTrack(t), i, { onPlay: () => playList(arr.map(fromCloudTrack), i) })))) : null);
      return h('div', {},
        h('div', { class: 'stat-strip' },
          h('div', { class: 'stat-pill' }, h('b', {}, (st.week_minutes || 0).toLocaleString()), ' min this week'),
          h('div', { class: 'stat-pill' }, h('b', {}, (st.month_minutes || 0).toLocaleString()), ' min in 4 weeks'),
          st.streak ? h('div', { class: 'stat-pill' }, h('b', {}, `🔥 ${st.streak}`), ' day streak') : null),
        artistTiles.length ? h('div', {}, h('div', { class: 'section-title' }, 'Top artists · last 4 weeks'), h('div', { class: 'top-artists' }, ...artistTiles)) : null,
        songList(st.top_tracks, 'Top songs · last 4 weeks'),
        songList(st.recent, 'Recently played'));
    };

    const render = () => {
      const facts = [
        followsMe.length ? h('span', { class: 'follows-you' }, 'Follows you') : null,
        matchEl,
      ];
      const head = h('div', { class: 'profile-head' }, banner,
        h('div', { class: 'profile-id' }, avatarBox,
          h('div', { class: 'grow min-w-0' },
            h('div', { class: 'hero-kicker' }, 'Profile', ...facts.filter(Boolean)),
            h('h1', { class: 'profile-name', title: p.name }, p.name, p.pronouns ? h('span', { class: 'pronouns' }, p.pronouns) : null),
            h('div', { class: 'profile-counts' },
              h('button', { onclick: () => { tab = 'followers'; render(); } }, h('b', {}, String(accepted.length)), accepted.length === 1 ? ' follower' : ' followers'),
              h('button', { onclick: () => { tab = 'following'; render(); } }, h('b', {}, String(following.length)), ' following'),
              h('span', {}, count(lists.length, 'public playlist', 'public playlists')))),
          h('div', { class: 'profile-actions' },
            isMe ? h('button', { class: 'btn small', onclick: () => editProfile(p, () => openProfile(userId, { push: false, tab })) }, 'Edit profile') : followBtn(p.id, p.name),
            !isMe && signedIn() ? h('button', { class: 'ab-icon', title: 'Send a song: use ⋯ → Send to a friend on any song', html: icon('send'), onclick: () => { const t = state.playback?.current?.track; if (t) sendSongToFriend(t); else toast('Play a song, then send it with ⋯ → Send to a friend', 'info', 3000); } }) : null,
            isMe ? h('button', { class: 'btn small', onclick: () => askCloudPrivacy(true) }, 'Privacy') : null,
            h('button', { class: 'ab-icon', title: 'Copy profile link', html: icon('link'), onclick: () => { copyText(PROFILE_LINK(p.id)); toast('Profile link copied', 'ok', 1500); } }))),
        (() => { const b = badgeRow(badges); if (b) b.classList.add('profile-badges'); return b; })(),
        p.bio ? h('p', { class: 'profile-bio' }, p.bio) : null,
        mutuals.length ? h('div', { class: 'mutuals' }, h('span', { class: 'avatar-stack' }, ...mutuals.slice(0, 3).map((m) => avatarEl(m.avatar, m.name, 'avatar xs'))),
          h('span', {}, `Followed by ${mutuals.slice(0, 2).map((m) => m.name).join(', ')}${mutuals.length > 2 ? ` and ${mutuals.length - 2} other${mutuals.length - 2 === 1 ? '' : 's'} you follow` : ''}`)) : null,
        nowCard());
      const tabs = h('div', { class: 'profile-tabs chips', role: 'tablist' },
        tabBtn('playlists', `Playlists · ${lists.length}`), tabBtn('listening', 'Listening'), tabBtn('followers', `Followers · ${accepted.length}`), tabBtn('following', `Following · ${following.length}`));

      let content;
      if (tab === 'playlists') {
        content = lists.length
          ? h('div', { class: 'card-row' }, ...lists.map((l, i) => cardTile({ kind: 'playlist', title: l.title, subtitle: `Playlist · ${p.name}`, thumbnail: l.cover || l.thumbnail, browse_id: null, track: null, _cloud: l.id }, i)))
          : h('p', { class: 'meta' }, isMe ? 'Make a playlist public (⋯ → Share link → Public) to show it here.' : 'No public playlists yet.');
      } else if (tab === 'listening') {
        content = listening();
      } else if (tab === 'followers') {
        const rows = accepted.map((f) => ({ ...f.profiles, id: f.follower }));
        content = h('div', {}, personList(rows, isMe ? 'Nobody follows you yet. Send friends your profile link.' : 'No followers yet.'),
          isMe && rows.length ? h('details', { class: 'manage-followers' }, h('summary', {}, 'Remove followers'),
            ...rows.map((r) => h('div', { class: 'friend' }, avatarEl(r.avatar, r.name), h('span', { class: 'grow' }, r.name || 'Someone'),
              h('button', { class: 'btn small', onclick: async () => {
                if (!(await confirmModal({ title: `Remove ${r.name || 'this follower'}?`, message: 'They stop seeing what you play and your Wrapped. They aren’t told.', okLabel: 'Remove', danger: true }))) return;
                try { await db('DELETE', `follows?follower=eq.${r.id}&followee=eq.${me()}`); openProfile(userId, { push: false, tab: 'followers' }); } catch (e) { toast(errMsg(e), 'error'); }
              } }, 'Remove')))) : null);
      } else {
        content = personList(following.map((f) => ({ ...f.profiles, id: f.followee })), isMe ? 'You don’t follow anyone yet. Find people in the Friends panel.' : 'Not following anyone yet.');
      }
      body.replaceChildren(head, tabs, h('div', { class: 'profile-tab' }, content));
      if (tab === 'playlists') {
        $$('#profile-body .tile').forEach((tile, i) => { tile.onclick = (e) => { e.stopPropagation(); openSharedPlaylist(lists[i].id); }; const pb = tile.querySelector('.tile-play'); if (pb) pb.onclick = async (e) => { e.stopPropagation(); const [row] = await db('GET', `playlists?id=eq.${lists[i].id}&select=tracks`); playList(row.tracks.map(fromCloudTrack), 0); }; });
      }
      markPlaying();
    };
    render();
    heroColor(body, mediaUrl(p.avatar));
  } catch (e) { if (seq === profileSeq) body.replaceChildren(h('div', { class: 'empty' }, h('p', {}, errMsg(e)))); }
}

// Wrapped: compare with approved friends ------------------------------------------------------------
async function compareWrapped(s, year) {
  if (!(await requireAccount({ why: 'Sign in to compare with friends' }))) return;
  if (cloud.profile?.share_wrapped === false) {
    if (!(await confirmModal({ title: 'Share your Wrapped with friends?', message: 'Comparing shares your top artists, songs and minutes with approved followers and people you follow. You can turn it off again in Settings → Account.', okLabel: 'Share and compare' }))) return;
    await saveCloudPrivacy({ share_wrapped: true });
  }
  const mine = { artists: s.top_artists.slice(0, 30).map((a) => a.name), songs: s.top_tracks.slice(0, 30).map((t) => `${t.track.artist} — ${t.track.title}`), minutes: Math.round(s.minutes) };
  try {
    await db('POST', 'wrapped', { user_id: me(), year, data: mine, updated_at: new Date().toISOString() }, { prefer: 'resolution=merge-duplicates' });
    const follows = await db('GET', `follows?follower=eq.${me()}&status=eq.accepted&select=followee,profiles!follows_followee_fkey(id,name,avatar)`);
    const theirs = follows.length ? await db('GET', `wrapped?year=eq.${year}&user_id=in.(${follows.map((f) => f.followee).join(',')})`) : [];
    const m = openModal({
      title: `Compare your ${year}`,
      wide: true,
      body: theirs.length ? h('div', { class: 'import-list' }, ...theirs.map((w) => {
        const who = follows.find((f) => f.followee === w.user_id)?.profiles;
        const common = mine.artists.filter((a) => w.data.artists?.includes(a));
        const score = Math.round((common.length / Math.max(1, Math.min(mine.artists.length, w.data.artists?.length || 0))) * 100);
        return h('div', { class: 'import-row align-start' }, avatarEl(who?.avatar, who?.name),
          h('div', { class: 'grow' }, h('b', {}, `${who?.name || 'Friend'} · ${score}% match`),
            h('div', { class: 'meta' }, common.length ? `You both love ${common.slice(0, 5).join(', ')}${common.length > 5 ? ` and ${common.length - 5} more` : ''}` : 'Totally different taste this year'),
            h('div', { class: 'meta' }, `${w.data.minutes?.toLocaleString() || 0} minutes vs your ${mine.minutes.toLocaleString()}`)));
      })) : h('p', {}, 'None of the people you follow have shared their Wrapped yet. Yours is shared now, so they can compare with you.'),
      footer: [h('button', { class: 'btn primary', onclick: () => m.close() }, 'Done')],
    });
  } catch (e) { toast(errMsg(e), 'error'); }
}
