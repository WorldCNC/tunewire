'use strict';

// ===========================================================================
// Friends, part 2: badges, avatar decorations, bio & pronouns, shared
// listening stats, presence (online dots, live progress), listen along,
// sending songs to friends (inbox) and the weekly leaderboard.
// ===========================================================================

// Badges -------------------------------------------------------------------------
// Worked out by the server (profile_badges); this is only how they look.
const BADGES = {
  developer: { icon: '🛠️', label: 'Tunewire developer', desc: 'Builds Tunewire' },
  early: { icon: '🌱', label: 'Early adopter', desc: 'Joined in Tunewire’s first months' },
  social: { icon: '🤝', label: 'Social', desc: '10 or more followers' },
  curator: { icon: '📀', label: 'Curator', desc: 'Their playlists have 10 or more followers' },
  host: { icon: '🎧', label: 'Host', desc: 'Hosted 10 listening sessions' },
  connector: { icon: '💌', label: 'Connector', desc: 'Sent 25 songs to friends' },
  streak7: { icon: '🔥', label: '7-day streak', desc: 'Listened every day for a week' },
  streak30: { icon: '☄️', label: '30-day streak', desc: 'Listened every day for a month' },
  night_owl: { icon: '🦉', label: 'Night owl', desc: 'Mostly listens late at night' },
  early_bird: { icon: '🐦', label: 'Early bird', desc: 'Mostly listens in the morning' },
  explorer: { icon: '🧭', label: 'Explorer', desc: 'Listened to 100+ artists in a month' },
};
const badgeCache = new Map();
async function badgesOf(uid) {
  const hit = badgeCache.get(uid);
  if (hit && Date.now() - hit.at < 5 * 60 * 1000) return hit.list;
  const list = await rpc('profile_badges', { uid }).catch(() => []) || [];
  badgeCache.set(uid, { at: Date.now(), list });
  return list;
}
function badgeRow(ids, { max = 20, small = false } = {}) {
  const shown = (ids || []).filter((b) => BADGES[b]).slice(0, max);
  if (!shown.length) return null;
  return h('div', { class: `badges${small ? ' small' : ''}`, role: 'list', 'aria-label': 'Badges' },
    ...shown.map((b) => h('span', { class: 'badge', role: 'listitem', title: `${BADGES[b].label}: ${BADGES[b].desc}` }, h('span', { class: 'b-ico', 'aria-hidden': 'true' }, BADGES[b].icon), small ? null : h('span', {}, BADGES[b].label))));
}

// Avatars with Discord decorations ----------------------------------------------------
const DECORATION_RE = /^https:\/\/cdn\.discordapp\.com\/avatar-decoration-presets\/(a_)?[0-9a-f]+\.png(\?[a-z0-9=&]+)?$/;
/** An avatar, with the person's Discord avatar decoration around it. */
function decoratedAvatar(src, name, decoration, cls = 'avatar') {
  const av = avatarEl(src, name, cls);
  if (typeof decoration !== 'string' || !DECORATION_RE.test(decoration)) return av;
  return h('span', { class: `av-wrap ${cls.split(' ').filter((c) => c !== 'avatar').join(' ')}` }, av, h('img', { class: 'av-deco', src: decoration, alt: '', 'aria-hidden': 'true', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }));
}

// Presence -------------------------------------------------------------------------------
const ONLINE_MS = 3 * 60 * 1000;
/** online | listening | away | offline, from a now_playing row. */
function presenceOf(np) {
  if (!np) return 'offline';
  const seen = Math.max(Date.parse(np.online_at || 0) || 0, Date.parse(np.updated_at || 0) || 0);
  if (Date.now() - seen > ONLINE_MS) return 'offline';
  return np.playing ? 'listening' : 'online';
}
/** Where their song is now (their last position, moved on by the time since). */
function livePosition(np) {
  if (!np || np.position == null) return null;
  const pos = np.position + (np.playing ? (Date.now() - Date.parse(np.updated_at)) / 1000 : 0);
  return np.duration ? Math.min(pos, np.duration) : pos;
}

// While the app is open (and sharing is on), say so once a minute.
setInterval(() => { if (signedIn() && state.settings?.share_activity && cloud.profile?.share_activity) publishActivity(true); }, 60 * 1000);

// Listen along ------------------------------------------------------------------------------
// Follow a friend's player without a session: play what they play, where
// they are in it, and keep up as they go.
const along = { uid: null, name: '', channel: null, key: null, applying: false };

async function listenAlong(uid, name) {
  if (along.uid === uid) return;
  stopListenAlong(true);
  if (jam.code) { toast('Leave the listening session first', 'warn'); return; }
  let row;
  try { [row] = await db('GET', `now_playing?user_id=eq.${uid}&select=*`); } catch (e) { toast(errMsg(e), 'error'); return; }
  if (!row?.track || presenceOf(row) === 'offline') { toast(`${name} isn’t playing anything right now`, 'info', 2500); return; }
  Object.assign(along, { uid, name, key: null });
  along.channel = new Channel(`along:${uid}`, { changes: [{ event: 'UPDATE', schema: 'public', table: 'now_playing', filter: `user_id=eq.${uid}` }] });
  along.channel.on('change', (d) => { if (d?.record) followAlong(d.record); });
  await followAlong(row);
  paintAlong();
  toast(`Listening along with ${name}`, 'ok', 2500);
}

async function followAlong(row) {
  if (!along.uid || along.applying) return;
  if (!row.track || presenceOf(row) === 'offline') { const n = along.name; stopListenAlong(); toast(`${n} stopped playing`, 'info', 2500); return; }
  along.applying = true;
  try {
    const t = fromCloudTrack(row.track);
    const p = state.playback;
    if (p?.current?.track?.key !== t.key) {
      along.key = t.key;
      await call('play_track', { track: t, platform: null });
      for (let i = 0; i < 40 && !(state.playback?.status === 'playing' && state.playback.current?.track?.key === t.key); i++) await new Promise((r) => setTimeout(r, 250));
    }
    const target = livePosition(row);
    if (target != null && Math.abs((state.playback?.position || 0) - target) > 3) await call('seek', { position: target });
    if (!row.playing && state.playback?.status === 'playing') await call('pause');
    if (row.playing && state.playback?.status === 'paused') await call('resume');
  } catch { /* the next update fixes it */ }
  along.applying = false;
}

function stopListenAlong(quiet = false) {
  if (!along.uid) return;
  const n = along.name;
  along.channel?.close();
  Object.assign(along, { uid: null, name: '', channel: null, key: null });
  paintAlong();
  if (!quiet) toast(`Stopped listening along with ${n}`, 'info', 1800);
}

// Playing something else yourself ends it.
function checkAlongTakeover() {
  if (!along.uid || along.applying || !along.key) return;
  const cur = state.playback?.current?.track?.key;
  if (cur && cur !== along.key && state.playback?.status !== 'loading') stopListenAlong();
}

function paintAlong() {
  let pill = $('#along-pill');
  if (!along.uid) { pill?.remove(); return; }
  if (!pill) {
    pill = h('div', { id: 'along-pill', class: 'along-pill', role: 'status' });
    $('#player-bar .now')?.after(pill);
  }
  pill.replaceChildren(h('span', { class: 'sync-dot' }), h('span', {}, `Listening along with ${along.name}`), h('button', { class: 'link-btn', onclick: () => stopListenAlong() }, 'Stop'));
}

// Sending songs to friends -----------------------------------------------------------------
/** People you can send to: approved follows either way. */
async function friendList() {
  const me = cloud.session.user_id;
  const [out, inc] = await Promise.all([
    db('GET', `follows?follower=eq.${me}&status=eq.accepted&select=followee,profiles!follows_followee_fkey(id,name,avatar,decoration)`),
    db('GET', `follows?followee=eq.${me}&status=eq.accepted&select=follower,profiles!follows_follower_fkey(id,name,avatar,decoration)`),
  ]);
  const map = new Map();
  for (const f of out) if (f.profiles) map.set(f.profiles.id, f.profiles);
  for (const f of inc) if (f.profiles) map.set(f.profiles.id, f.profiles);
  return [...map.values()].sort((a, b) => a.name.localeCompare(b.name));
}

async function sendSongToFriend(track) {
  if (!(await requireAccount({ why: 'Sign in to send songs to friends' }))) return;
  let people;
  try { people = await friendList(); } catch (e) { toast(errMsg(e), 'error'); return; }
  if (!people.length) { toast('Follow some friends first (they approve it), then you can send them songs', 'info', 4000); return; }
  const chosen = new Set();
  const note = h('input', { class: 'input', placeholder: 'Add a note (optional)', maxlength: 200, 'aria-label': 'Note' });
  const search = h('input', { class: 'input', placeholder: 'Search friends', 'aria-label': 'Search friends' });
  const list = h('div', { class: 'send-list', role: 'listbox', 'aria-multiselectable': 'true' });
  const send = h('button', { class: 'btn primary', disabled: true }, 'Send');
  const paint = () => {
    const q = search.value.trim().toLowerCase();
    list.replaceChildren(...people.filter((p) => !q || p.name.toLowerCase().includes(q)).map((p) => {
      const row = h('button', { class: `send-row${chosen.has(p.id) ? ' on' : ''}`, role: 'option', 'aria-selected': String(chosen.has(p.id)) },
        decoratedAvatar(p.avatar, p.name, p.decoration), h('span', { class: 'grow' }, p.name), h('span', { class: 'send-check', html: icon('check') }));
      row.addEventListener('click', () => { if (chosen.has(p.id)) chosen.delete(p.id); else chosen.add(p.id); paint(); });
      return row;
    }));
    send.disabled = !chosen.size;
    send.textContent = chosen.size > 1 ? `Send to ${chosen.size}` : 'Send';
  };
  search.addEventListener('input', paint);
  paint();
  send.addEventListener('click', async () => {
    send.disabled = true;
    try {
      const slim = slimTrack(track);
      await db('POST', 'song_shares', [...chosen].map((to) => ({ to_user: to, track: slim, note: note.value.trim().slice(0, 200) })));
      m.close();
      toast(`Sent “${track.title}”`, 'ok', 2000);
    } catch (e) { send.disabled = false; toast(errMsg(e), 'error'); }
  });
  const m = openModal({
    title: 'Send to a friend',
    body: h('div', { class: 'send-box' },
      h('div', { class: 'send-song' }, thumbEl(track.thumbnail), h('div', { class: 'min-w-0' }, h('b', { class: 'ellipsis' }, track.title), h('div', { class: 'meta ellipsis' }, track.artist || ''))),
      search, list, note),
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), send],
  });
}

// Inbox --------------------------------------------------------------------------------------------
const INBOX_REACTIONS = ['🔥', '❤️', '😂', '🕺', '👏', '😮'];
async function renderInbox(box) {
  box.replaceChildren(...skeletonFriends(3));
  let rows;
  try { rows = await db('GET', `song_shares?to_user=eq.${cloud.session.user_id}&select=*,sender:profiles!song_shares_from_user_fkey(id,name,avatar,decoration)&order=created_at.desc&limit=60`); } catch (e) { box.replaceChildren(h('p', { class: 'meta' }, errMsg(e))); return; }
  if (!rows.length) { box.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'No songs yet'), h('p', { class: 'small' }, 'When friends send you a song, it shows up here. Send one with ⋯ → Send to a friend.'))); return; }
  box.replaceChildren(...rows.map((r) => {
    const t = fromCloudTrack(r.track);
    const react = h('div', { class: 'inbox-react' }, ...INBOX_REACTIONS.map((e) => h('button', { class: r.reaction === e ? 'on' : '', 'aria-label': `React ${e}`, onclick: async (ev) => {
      try { await db('PATCH', `song_shares?id=eq.${r.id}`, { reaction: r.reaction === e ? null : e }); r.reaction = r.reaction === e ? null : e; ev.currentTarget.parentElement.querySelectorAll('button').forEach((b) => b.classList.toggle('on', b.textContent === r.reaction)); } catch (er) { toast(errMsg(er), 'error'); }
    } }, e)));
    return h('div', { class: `inbox-item${r.read ? '' : ' unread'}` },
      h('div', { class: 'inbox-from' }, decoratedAvatar(r.sender?.avatar, r.sender?.name, r.sender?.decoration, 'avatar'), h('button', { class: 'friend-name', onclick: () => openProfile(r.from_user) }, r.sender?.name || 'Someone'), h('span', { class: 'meta' }, timeAgo(Date.parse(r.created_at)))),
      r.note ? h('p', { class: 'inbox-note' }, r.note) : null,
      h('div', { class: 'inbox-song' }, thumbEl(t.thumbnail),
        h('div', { class: 'grow min-w-0' }, h('b', { class: 'ellipsis' }, t.title), h('div', { class: 'meta ellipsis' }, t.artist || '')),
        h('button', { class: 'icon-btn', title: 'Play', html: icon('play'), disabled: !t.versions.length, onclick: () => playTrack(t) }),
        h('button', { class: 'icon-btn', title: 'Add to queue', html: icon('plus'), disabled: !t.versions.length, onclick: () => enqueue([t]) })),
      react);
  }));
  // Seen now.
  if (rows.some((r) => !r.read)) db('PATCH', `song_shares?to_user=eq.${cloud.session.user_id}&read=eq.false`, { read: true }).catch(() => {});
}

// Weekly leaderboard ---------------------------------------------------------------------------------
async function renderLeaderboard(box) {
  box.replaceChildren(...skeletonFriends(4));
  let rows;
  try { rows = await rpc('friend_leaderboard', {}); } catch (e) { box.replaceChildren(h('p', { class: 'meta' }, errMsg(e))); return; }
  const mineShared = !!cloud.profile?.share_stats;
  const intro = h('p', { class: 'meta lb-intro' }, 'Minutes listened since Monday, among you and the friends who share their listening.');
  if (!rows.length) {
    box.replaceChildren(intro, h('div', { class: 'empty' }, h('p', {}, 'Nobody to rank yet'),
      h('p', { class: 'small' }, mineShared ? 'When friends share their listening, they show up here.' : 'Turn on “Share my listening” to join the board.'),
      mineShared ? null : h('button', { class: 'btn small primary mt-12', onclick: () => saveCloudPrivacy({ share_stats: true }).then(() => { publishStats(true); renderLeaderboard(box); }) }, 'Share my listening')));
    return;
  }
  const top = rows[0]?.week_minutes || 1;
  box.replaceChildren(intro, ...rows.map((r, i) => h('div', { class: `lb-row${r.user_id === cloud.session.user_id ? ' me' : ''}` },
    h('span', { class: `lb-rank r${i + 1}` }, i < 3 ? ['🥇', '🥈', '🥉'][i] : String(i + 1)),
    decoratedAvatar(r.avatar, r.name, r.decoration),
    h('div', { class: 'grow min-w-0' },
      h('button', { class: 'friend-name', onclick: () => openProfile(r.user_id) }, r.user_id === cloud.session.user_id ? `${r.name} (you)` : r.name),
      h('div', { class: 'lb-bar' }, h('i', { style: `width:${Math.max(3, (r.week_minutes / top) * 100)}%` }))),
    h('span', { class: 'lb-min' }, `${r.week_minutes.toLocaleString()} min`),
    r.streak >= 3 ? h('span', { class: 'lb-streak', title: `${r.streak}-day streak` }, `🔥${r.streak}`) : null)));
}

// Shared listening stats --------------------------------------------------------------------------
// Top artists and songs of the last 4 weeks, recently played, minutes this
// week and the streak: sent only when "Share my listening" is on.
let statsSentAt = 0;
async function publishStats(force = false) {
  if (!signedIn() || !cloud.profile?.share_stats) return;
  if (!force && Date.now() - statsSentAt < 20 * 60 * 1000) return;
  statsSentAt = Date.now();
  try {
    const [month, week, recent] = await Promise.all([
      call('listening_stats', { range: '4w' }),
      call('listening_stats', { range: 'wk' }),
      call('list_history', { limit: 12 }),
    ]);
    const hours = month.hours || [];
    const night = [22, 23, 0, 1, 2, 3, 4].reduce((n, h2) => n + (hours[h2] || 0), 0);
    const morning = [5, 6, 7, 8, 9, 10].reduce((n, h2) => n + (hours[h2] || 0), 0);
    const all = hours.reduce((a, b) => a + b, 0) || 1;
    const tod = night / all > 0.4 ? 'night' : morning / all > 0.35 ? 'morning' : month.minutes > 0 ? 'day' : '';
    const monday = new Date();
    monday.setHours(0, 0, 0, 0);
    monday.setDate(monday.getDate() - ((monday.getDay() + 6) % 7));
    const iso = `${monday.getFullYear()}-${String(monday.getMonth() + 1).padStart(2, '0')}-${String(monday.getDate()).padStart(2, '0')}`;
    await db('POST', 'profile_stats?on_conflict=user_id', {
      user_id: cloud.session.user_id,
      top_artists: month.top_artists.slice(0, 6).map((a) => ({ name: a.name, thumbnail: a.thumbnail || null, minutes: Math.round(a.minutes) })),
      top_tracks: month.top_tracks.slice(0, 6).map((t) => ({ ...slimTrack(t.track), plays: t.plays })),
      recent: recent.filter((e, i, arr) => arr.findIndex((x) => x.track.key === e.track.key) === i).slice(0, 8).map((e) => ({ ...slimTrack(e.track), played_at: e.played_at })),
      week_start: iso,
      week_minutes: Math.min(10080, Math.round(week.minutes)),
      month_minutes: Math.min(43200, Math.round(month.minutes)),
      artists_count: month.artists,
      streak: month.streak,
      time_of_day: tod,
      updated_at: new Date().toISOString(),
    }, { prefer: 'resolution=merge-duplicates' });
    badgeCache.delete(cloud.session.user_id);
  } catch { statsSentAt = 0; }
}
setInterval(() => publishStats(), 5 * 60 * 1000);
onCloudChange(() => setTimeout(() => publishStats(), 4000));

// Taste match ------------------------------------------------------------------------------------
/** 0-100: how much your top artists overlap with theirs (null if unknown). */
async function tasteMatch(theirArtists) {
  if (!theirArtists?.length) return null;
  const mine = await call('listening_stats', { range: '6m' }).catch(() => null);
  if (!mine?.top_artists?.length) return null;
  const a = new Set(mine.top_artists.slice(0, 30).map((x) => x.name.toLowerCase()));
  const b = theirArtists.map((x) => x.name.toLowerCase());
  const common = b.filter((x) => a.has(x)).length;
  return Math.round(Math.min(100, (common / Math.max(1, Math.min(b.length, 6))) * 100));
}

// Editing your profile ----------------------------------------------------------------------------
async function editProfile(p, done) {
  const name = h('input', { class: 'input', value: p.name || '', maxlength: 40, 'aria-label': 'Name' });
  const pronouns = h('input', { class: 'input', value: p.pronouns || '', maxlength: 40, placeholder: 'e.g. she/her, they/them', 'aria-label': 'Pronouns' });
  const bio = h('textarea', { class: 'input font-inherit', rows: 4, maxlength: 300, placeholder: 'A line about you and your music', 'aria-label': 'Bio' }, p.bio || '');
  const count = h('span', { class: 'meta' }, `${(p.bio || '').length}/300`);
  bio.addEventListener('input', () => { count.textContent = `${bio.value.length}/300`; });
  const m = openModal({
    title: 'Edit profile',
    body: h('div', { class: 'col gap-10' },
      h('label', { class: 'field' }, h('span', {}, 'Name'), name),
      h('label', { class: 'field' }, h('span', {}, 'Pronouns'), pronouns),
      h('label', { class: 'field' }, h('span', {}, 'Bio'), bio, count),
      h('p', { class: 'meta' }, 'Your picture, banner and avatar decoration come from Discord and update when you sign in.')),
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), h('button', { class: 'btn primary', onclick: async () => {
      const patch = { name: name.value.trim().slice(0, 40) || p.name, pronouns: pronouns.value.trim().slice(0, 40), bio: bio.value.trim().slice(0, 300) };
      try {
        await db('PATCH', `profiles?id=eq.${cloud.session.user_id}`, patch);
        cloud.session.name = patch.name;
        cloud.profile = { ...(cloud.profile || {}), ...patch };
        cloudChanged();
        m.close();
        done?.();
      } catch (e) { toast(errMsg(e), 'error'); }
    } }, 'Save')],
  });
}
