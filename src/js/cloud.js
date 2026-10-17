'use strict';

// ===========================================================================
// Online features (Supabase): accounts, database calls and live channels.
// The backend handles sign-in (Discord only) and keeps the refresh token in
// the keychain; this file asks it for a fresh access token when needed.
// ===========================================================================

const cloud = { cfg: null, session: null, ready: false, listeners: new Set(), clockOffset: 0 };

async function cloudInit() {
  try { cloud.cfg = await call('cloud_config'); } catch { cloud.cfg = null; }
  if (cloud.cfg) {
    try { cloud.session = await call('cloud_session'); } catch { cloud.session = null; }
    await loadProfileName();
    syncServerClock();
  }
  cloud.ready = true;
  cloudChanged();
}

/** The name/picture friends see (the profile row, which the user can edit). */
async function loadProfileName() {
  if (!cloud.session) return;
  try {
    const [p] = await db('GET', `profiles?id=eq.${cloud.session.user_id}&select=name,avatar,share_activity,share_wrapped`);
    if (p) { cloud.session.name = p.name; cloud.session.avatar = p.avatar || cloud.session.avatar; cloud.profile = p; }
  } catch { /* offline */ }
}

function cloudOn() { return !!cloud.cfg; }
function signedIn() { return !!cloud.session; }
function cloudChanged() { for (const f of cloud.listeners) { try { f(); } catch { /* keep going */ } } }
function onCloudChange(f) { cloud.listeners.add(f); }

// One refresh at a time: refresh tokens are single-use, so parallel callers
// share the same request instead of each spending the token.
let tokenInflight = null;
async function cloudToken() {
  if (!cloud.cfg) throw { kind: 'unavailable', message: 'Online features aren’t available in this build' };
  if (cloud.session && cloud.session.expires_at > Date.now() + 90_000) return cloud.session.access_token;
  if (!tokenInflight) {
    tokenInflight = call('cloud_session')
      .then((s) => {
        const was = !!cloud.session;
        cloud.session = s;
        if (was !== !!s) cloudChanged();
        return s?.access_token || null;
      })
      .finally(() => { tokenInflight = null; });
  }
  return tokenInflight;
}

/** Server time (ms) as seen from here: Listen Together positions use it. */
function serverNow() { return Date.now() + cloud.clockOffset; }
async function syncServerClock() {
  try {
    const t0 = Date.now();
    const s = await db('POST', 'rpc/server_now', {}, { anon: true });
    const t1 = Date.now();
    if (typeof s === 'number') cloud.clockOffset = s - (t0 + t1) / 2;
  } catch { /* keep the last offset */ }
}

/** Sign in with Discord (the only way in). Resolves true when signed in. */
async function requireAccount({ why = 'Sign in to use this' } = {}) {
  if (!cloudOn()) { toast('Online features aren’t available in this build', 'warn'); return false; }
  if (signedIn()) return true;
  return new Promise((resolve) => {
    let done = false;
    const status = h('p', { class: 'muted small min-h-18', role: 'status' });
    const btn = h('button', { class: 'import-option discord-signin' },
      h('span', { class: 'src-ico discord', html: plogo('discord') || icon('friends') }),
      h('div', {}, h('b', {}, 'Continue with Discord'), h('span', {}, 'Opens your browser. Tunewire gets your Discord name and picture, nothing else.')),
      h('span', { html: icon('chevRight') }));
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      status.textContent = 'Continue in your browser…';
      try {
        cloud.session = await call('cloud_sign_in', { provider: 'discord' });
        await loadProfileName();
        cloudChanged();
        done = true;
        m.close();
        resolve(true);
        askCloudPrivacy();
      } catch (e) {
        status.textContent = e?.kind === 'cancelled' ? 'Sign-in was cancelled.' : errMsg(e);
        btn.disabled = false;
      }
    });
    const m = openModal({
      title: why,
      subtitle: 'Your playlists and listening stay on this computer. An account is only used for sharing, collaborative playlists, Listen Together and friends.',
      body: h('div', { class: 'import-box' }, btn, status),
      onClose: () => { if (!done) resolve(false); },
    });
  });
}

/** PostgREST call. path like "playlists?id=eq.x&select=*". */
async function db(method, path, body, { prefer, anon = false } = {}) {
  if (!cloud.cfg) throw { kind: 'unavailable', message: 'Online features aren’t available in this build' };
  const token = anon ? null : await cloudToken().catch(() => null);
  const headers = { apikey: cloud.cfg.anon_key, Authorization: `Bearer ${token || cloud.cfg.anon_key}` };
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  if (prefer) headers.Prefer = prefer;
  let r;
  try {
    r = await fetch(`${cloud.cfg.url}/rest/v1/${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  } catch {
    throw { kind: 'network', message: 'Can’t reach the Tunewire server' };
  }
  const text = await r.text();
  // Gateways answer errors in HTML; never let that become a SyntaxError.
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { data = null; }
  if (!r.ok) {
    const kind = r.status === 401 || r.status === 403 ? 'unavailable' : r.status >= 500 || r.status === 429 ? 'network' : 'storage';
    const msg = data?.message || (r.status === 429 ? 'Too many requests, try again in a moment' : r.status >= 500 ? 'The Tunewire server is having trouble' : `Server error ${r.status}`);
    throw { kind, status: r.status, message: msg };
  }
  if (text && data === null) throw { kind: 'network', message: 'The Tunewire server sent something unexpected' };
  return data;
}
const rpc = (fn, args) => db('POST', `rpc/${fn}`, args);

// Song info for the server (no local paths, smaller).
function slimTrack(t) {
  return {
    key: t.key, title: t.title, artist: t.artist, duration: t.duration ?? null, thumbnail: t.thumbnail || null,
    versions: t.versions.filter((v) => v.platform !== 'local').slice(0, 4).map((v) => ({
      platform: v.platform, url: v.url, id: v.id || '', title: v.title, artist: v.artist || '', duration: v.duration ?? null,
      thumbnail: v.thumbnail || null, album: v.album || null, matched_for: v.matched_for || null, isrc_match: !!v.isrc_match, preview: !!v.preview,
    })),
  };
}

// Songs from other people only play from the platforms' own sites.
const PLATFORM_HOSTS = {
  ytmusic: /^https:\/\/music\.youtube\.com\//,
  youtube: /^https:\/\/(www\.|m\.)?youtube\.com\/|^https:\/\/youtu\.be\//,
  soundcloud: /^https:\/\/(www\.|m\.)?soundcloud\.com\//,
  spotify: /^https:\/\/open\.spotify\.com\/|^spotify:track:/,
};
function safeVersion(v) {
  return v && typeof v.url === 'string' && PLATFORM_HOSTS[v.platform]?.test(v.url);
}

function fromCloudTrack(t) {
  const { _by, _at, ...rest } = t || {};
  const str = (x, n) => (typeof x === 'string' ? x.slice(0, n) : '');
  return {
    score: 0,
    ...rest,
    key: str(rest.key, 400),
    title: str(rest.title, 300),
    artist: str(rest.artist, 300),
    versions: (Array.isArray(rest.versions) ? rest.versions : []).filter(safeVersion).map((v) => ({ rank: 0, partial: false, ...v })),
  };
}

// Share links --------------------------------------------------------------------
// With a link-preview server (TUNEWIRE_SHARE_URL) links show the playlist or
// song in chat apps; otherwise they go to the GitHub Pages landing page.
const LANDING_BASE = 'https://worldcnc.github.io/tunewire/';
function shareLink(kind, code) {
  const base = cloud.cfg?.share_url;
  return base ? `${base}/${kind}/${code}` : `${LANDING_BASE}#${kind}=${code}`;
}
function b64url(s) { return btoa(unescape(encodeURIComponent(s))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''); }
function unb64url(s) { return decodeURIComponent(escape(atob(s.replace(/-/g, '+').replace(/_/g, '/')))); }

/** A song link works without the server: the song info rides in the link. */
function songLink(track) {
  const t = slimTrack(track);
  const v = t.versions.filter(safeVersion).slice(0, 3).map((x) => [x.platform, x.url]);
  return shareLink('s', b64url(JSON.stringify({ t: t.title, a: t.artist, d: t.duration, i: t.thumbnail, v })));
}
function trackFromSongLink(code) {
  const o = JSON.parse(unb64url(code));
  if (!o || typeof o.t !== 'string') throw new Error('bad link');
  const versions = (Array.isArray(o.v) ? o.v : []).slice(0, 4)
    .map(([platform, url], i) => ({ platform, url, id: '', title: o.t, artist: o.a || '', duration: typeof o.d === 'number' ? o.d : null, thumbnail: typeof o.i === 'string' && o.i.startsWith('https://') ? o.i : null, preview: false, partial: false, rank: i }))
    .filter(safeVersion);
  if (!versions.length) throw new Error('no playable source');
  const title = o.t.slice(0, 300), artist = typeof o.a === 'string' ? o.a.slice(0, 300) : '';
  return { key: `${artist.toLowerCase()}::${title.toLowerCase()}`, title, artist, duration: versions[0].duration, thumbnail: versions[0].thumbnail, versions, score: 0 };
}

// Realtime (Phoenix protocol over WebSocket) -----------------------------------------
// Reconnects with growing delays (2 s … 30 s), keeps the access token fresh
// on the open socket (Realtime drops an expired one), and holds at most 50
// unsent messages while offline.
const QUEUE_MAX = 50;
class Channel {
  constructor(topic, config = {}) {
    this.topic = `realtime:${topic}`;
    this.config = config;
    this.handlers = new Map();
    this.presence = {};
    this.ref = 0;
    this.joined = false;
    this.closed = false;
    this.queue = [];
    this.attempt = 0;
    this.connect();
  }
  on(event, fn) { if (!this.handlers.has(event)) this.handlers.set(event, []); this.handlers.get(event).push(fn); return this; }
  emit(event, payload) { for (const f of this.handlers.get(event) || []) { try { f(payload); } catch (e) { console.error(e); } } }
  async connect() {
    if (this.closed) return;
    const token = await cloudToken().catch(() => null);
    if (this.closed) return;
    const url = `${cloud.cfg.url.replace(/^http/, 'ws')}/realtime/v1/websocket?apikey=${encodeURIComponent(cloud.cfg.anon_key)}&vsn=1.0.0`;
    const ws = new WebSocket(url);
    this.ws = ws;
    this.token = token;
    ws.onopen = () => {
      this.send('phx_join', {
        config: { broadcast: { self: false }, presence: { key: this.config.presenceKey || '' }, postgres_changes: this.config.changes || [], private: !!this.config.private },
        access_token: token,
      });
      this.beat = setInterval(() => {
        this.raw({ topic: 'phoenix', event: 'heartbeat', payload: {}, ref: String(++this.ref) });
        this.refreshToken();
      }, 25000);
    };
    ws.onmessage = (m) => { try { this.receive(JSON.parse(m.data)); } catch { /* ignore junk */ } };
    ws.onclose = () => {
      clearInterval(this.beat);
      const wasJoined = this.joined;
      this.joined = false;
      this.emit('status', 'closed');
      if (this.closed) return;
      if (wasJoined) this.attempt = 0;
      const delay = Math.min(30000, 2000 * 2 ** this.attempt) * (0.75 + Math.random() * 0.5);
      this.attempt++;
      this.retry = setTimeout(() => this.connect(), delay);
    };
  }
  /** Hand Realtime the new token before the old one expires. */
  async refreshToken() {
    if (!this.joined) return;
    const t = await cloudToken().catch(() => null);
    if (t && t !== this.token) { this.token = t; this.send('access_token', { access_token: t }); }
  }
  raw(msg) { if (this.ws?.readyState === 1) this.ws.send(JSON.stringify(msg)); }
  send(event, payload) { this.raw({ topic: this.topic, event, payload, ref: String(++this.ref) }); }
  receive(msg) {
    if (msg.topic !== this.topic) return;
    if (msg.event === 'phx_reply' && msg.payload?.status === 'ok' && !this.joined) {
      this.joined = true;
      this.attempt = 0;
      this.emit('status', 'joined');
      for (const [ev, p] of this.queue.splice(0)) this.broadcast(ev, p);
      if (this.myPresence) this.track(this.myPresence);
      return;
    }
    if (msg.event === 'phx_reply' && msg.payload?.status === 'error' && !this.joined) {
      // Refused (warming up, or not allowed): reconnect later.
      this.emit('status', 'error');
      setTimeout(() => this.ws?.close(), 1000);
    }
    if (msg.event === 'phx_close' || msg.event === 'system' && msg.payload?.status === 'error') this.ws?.close();
    if (msg.event === 'broadcast') this.emit(`b:${msg.payload.event}`, msg.payload.payload);
    if (msg.event === 'postgres_changes') this.emit('change', msg.payload.data);
    if (msg.event === 'presence_state') { this.presence = msg.payload; this.emit('presence', this.presence); }
    if (msg.event === 'presence_diff') {
      for (const [k, v] of Object.entries(msg.payload.joins || {})) this.presence[k] = v;
      for (const k of Object.keys(msg.payload.leaves || {})) delete this.presence[k];
      this.emit('presence', this.presence);
    }
  }
  broadcast(event, payload) {
    if (!this.joined) {
      this.queue.push([event, payload]);
      if (this.queue.length > QUEUE_MAX) this.queue.shift();
      return;
    }
    this.send('broadcast', { type: 'broadcast', event, payload });
  }
  track(meta) { this.myPresence = meta; if (this.joined) this.send('presence', { type: 'presence', event: 'track', payload: meta }); }
  close() {
    this.closed = true;
    clearInterval(this.beat);
    clearTimeout(this.retry);
    try { this.send('phx_leave', {}); this.ws?.close(); } catch { /* gone */ }
  }
}

/** People in a presence map → [{key, name, avatar, ...}]. */
function presenceList(p) {
  return Object.entries(p).map(([key, v]) => ({ key, ...((v?.metas || v || [])[0] || {}) }));
}
