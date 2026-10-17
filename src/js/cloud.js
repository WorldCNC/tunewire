'use strict';

// ===========================================================================
// Online features (Supabase): accounts, database calls and live channels.
// The backend handles sign-in and keeps the refresh token in the keychain;
// this file asks it for a fresh access token for every call.
// ===========================================================================

const cloud = { cfg: null, session: null, ready: false, listeners: new Set() };

async function cloudInit() {
  try { cloud.cfg = await call('cloud_config'); } catch { cloud.cfg = null; }
  if (cloud.cfg) {
    try { cloud.session = await call('cloud_session'); } catch { cloud.session = null; }
  }
  cloud.ready = true;
  cloudChanged();
}

function cloudOn() { return !!cloud.cfg; }
function signedIn() { return !!cloud.session; }
function cloudChanged() { for (const f of cloud.listeners) { try { f(); } catch { /* keep going */ } } }
function onCloudChange(f) { cloud.listeners.add(f); }

async function cloudToken() {
  if (!cloud.cfg) throw { kind: 'unavailable', message: 'Online features aren’t available in this build' };
  if (!cloud.session || cloud.session.expires_at < Date.now() + 90_000) {
    cloud.session = await call('cloud_session');
    cloudChanged();
  }
  return cloud.session?.access_token || null;
}

/** Signed in, or offer to sign in (guest allowed when `guestOk`). */
async function requireAccount({ guestOk = false, why = 'Sign in to use this' } = {}) {
  if (!cloudOn()) { toast('Online features aren’t available in this build', 'warn'); return false; }
  if (signedIn()) return true;
  return new Promise((resolve) => {
    let done = false;
    const go = async (fn) => {
      try {
        cloud.session = await fn();
        cloudChanged();
        done = true;
        m.close();
        resolve(true);
      } catch (e) { status.textContent = errMsg(e); }
    };
    const status = h('p', { class: 'muted small', style: 'min-height:18px' });
    const m = openModal({
      title: why,
      subtitle: 'Your playlists and listening stay on this computer; an account is only used for sharing and listening together.',
      body: h('div', { class: 'import-box' },
        h('button', { class: 'import-option', onclick: () => { status.textContent = 'Continue in your browser…'; go(() => call('cloud_sign_in', { provider: 'discord' })); } }, h('span', { class: 'src-ico', html: icon('discord') || icon('user') }), h('div', {}, h('b', {}, 'Continue with Discord')), h('span', { html: icon('chevRight') })),
        h('button', { class: 'import-option', onclick: () => { status.textContent = 'Continue in your browser…'; go(() => call('cloud_sign_in', { provider: 'google' })); } }, h('span', { class: 'src-ico', html: icon('user') }), h('div', {}, h('b', {}, 'Continue with Google')), h('span', { html: icon('chevRight') })),
        guestOk ? h('button', { class: 'import-option', onclick: () => go(() => call('cloud_sign_in_guest')) }, h('span', { class: 'src-ico', html: icon('user') }), h('div', {}, h('b', {}, 'Continue as a guest'), h('span', {}, 'No account. Good for joining a friend’s session.')), h('span', { html: icon('chevRight') })) : null,
        status),
      onClose: () => { if (!done) resolve(false); },
    });
  });
}

/** PostgREST call. path like "playlists?id=eq.x&select=*". */
async function db(method, path, body, { prefer, anon = false } = {}) {
  const token = anon ? null : await cloudToken().catch(() => null);
  const headers = { apikey: cloud.cfg.anon_key, Authorization: `Bearer ${token || cloud.cfg.anon_key}` };
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  if (prefer) headers.Prefer = prefer;
  let r;
  try {
    r = await fetch(`${cloud.cfg.url}/rest/v1/${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  } catch (e) {
    throw { kind: 'network', message: 'Can’t reach the Tunewire server' };
  }
  const text = await r.text();
  const data = text ? JSON.parse(text) : null;
  if (!r.ok) throw { kind: r.status === 401 || r.status === 403 ? 'unavailable' : 'storage', message: data?.message || `Server error ${r.status}` };
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
function fromCloudTrack(t) {
  const { _by, _at, ...rest } = t;
  return { score: 0, ...rest, versions: (rest.versions || []).map((v) => ({ rank: 0, partial: false, ...v })) };
}

// Share links --------------------------------------------------------------------
const SHARE_BASE = 'https://worldcnc.github.io/tunewire/';
function b64url(s) { return btoa(unescape(encodeURIComponent(s))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''); }
function unb64url(s) { return decodeURIComponent(escape(atob(s.replace(/-/g, '+').replace(/_/g, '/')))); }

/** A song link works without the server: the song info rides in the link. */
function songLink(track) {
  const t = slimTrack(track);
  const v = t.versions.slice(0, 3).map((x) => [x.platform, x.url]);
  return `${SHARE_BASE}#s=${b64url(JSON.stringify({ t: t.title, a: t.artist, d: t.duration, i: t.thumbnail, v }))}`;
}
function trackFromSongLink(code) {
  const o = JSON.parse(unb64url(code));
  const versions = (o.v || []).map(([platform, url], i) => ({ platform, url, id: '', title: o.t, artist: o.a || '', duration: o.d ?? null, thumbnail: o.i || null, preview: false, partial: false, rank: i }));
  return { key: `${(o.a || '').toLowerCase()}::${(o.t || '').toLowerCase()}`, title: o.t, artist: o.a || '', duration: o.d ?? null, thumbnail: o.i || null, versions, score: 0 };
}

// Realtime (Phoenix protocol over WebSocket) -----------------------------------------
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
    this.connect();
  }
  on(event, fn) { if (!this.handlers.has(event)) this.handlers.set(event, []); this.handlers.get(event).push(fn); return this; }
  emit(event, payload) { for (const f of this.handlers.get(event) || []) { try { f(payload); } catch (e) { console.error(e); } } }
  async connect() {
    if (this.closed) return;
    const token = await cloudToken().catch(() => null);
    const url = `${cloud.cfg.url.replace(/^http/, 'ws')}/realtime/v1/websocket?apikey=${encodeURIComponent(cloud.cfg.anon_key)}&vsn=1.0.0`;
    const ws = new WebSocket(url);
    this.ws = ws;
    ws.onopen = () => {
      this.send('phx_join', { config: { broadcast: { self: false }, presence: { key: this.config.presenceKey || '' }, postgres_changes: this.config.changes || [] }, access_token: token });
      this.beat = setInterval(() => this.raw({ topic: 'phoenix', event: 'heartbeat', payload: {}, ref: String(++this.ref) }), 25000);
    };
    ws.onmessage = (m) => this.receive(JSON.parse(m.data));
    ws.onclose = () => {
      clearInterval(this.beat);
      this.joined = false;
      this.emit('status', 'closed');
      if (!this.closed) setTimeout(() => this.connect(), 2000 + Math.random() * 2000);
    };
  }
  raw(msg) { if (this.ws?.readyState === 1) this.ws.send(JSON.stringify(msg)); }
  send(event, payload) { this.raw({ topic: this.topic, event, payload, ref: String(++this.ref) }); }
  receive(msg) {
    if (msg.topic !== this.topic) return;
    if (msg.event === 'phx_reply' && msg.payload?.status === 'ok' && !this.joined) {
      this.joined = true;
      this.emit('status', 'joined');
      for (const [ev, p] of this.queue.splice(0)) this.broadcast(ev, p);
      if (this.myPresence) this.track(this.myPresence);
      return;
    }
    if (msg.event === 'phx_reply' && msg.payload?.status === 'error') this.emit('status', 'error');
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
    if (!this.joined) { this.queue.push([event, payload]); return; }
    this.send('broadcast', { type: 'broadcast', event, payload });
  }
  track(meta) { this.myPresence = meta; if (this.joined) this.send('presence', { type: 'presence', event: 'track', payload: meta }); }
  close() { this.closed = true; clearInterval(this.beat); try { this.send('phx_leave', {}); this.ws?.close(); } catch { /* gone */ } }
}

/** People in a presence map → [{key, name, avatar, ...}]. */
function presenceList(p) {
  return Object.entries(p).map(([key, metas]) => ({ key, ...(metas[0] || {}) }));
}
