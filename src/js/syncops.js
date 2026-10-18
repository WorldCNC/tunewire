'use strict';

// ===========================================================================
// Playlist sync arithmetic (no I/O, so it can be tested on its own):
// turning "the songs at the last sync" + "the songs now" into the server's
// operations, and applying the server's operations to a local list. The
// operations behave exactly like playlist_op() in the database.
// ===========================================================================

/** Songs with each key once (the first one wins), like the server keeps them. */
function uniqTracks(tracks) {
  const seen = new Set();
  return (tracks || []).filter((t) => t && t.key && !seen.has(t.key) && seen.add(t.key));
}
const uniqKeys = (tracks) => uniqTracks(tracks).map((t) => t.key);
function sameKeys(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

/** Cheap fingerprint of a cover (data: URL) to notice it changed. */
function coverSig(c) {
  if (!c) return '';
  let h = 0x811c9dc5;
  for (let i = 0; i < c.length; i++) h = Math.imul(h ^ c.charCodeAt(i), 0x01000193) >>> 0;
  return `${c.length}:${h.toString(36)}`;
}

/** The server's "move": `keys` (in their current order) taken out and put back at `to`. */
function moveKeys(list, keys, to) {
  const set = new Set(keys);
  const moving = list.filter((k) => set.has(k));
  const rest = list.filter((k) => !set.has(k));
  rest.splice(Math.max(0, Math.min(to, rest.length)), 0, ...moving);
  return rest;
}

/** One "move" op that turns `from` into `to` (same keys), or null if it takes more. */
function singleMove(from, to) {
  // Songs that stayed in order = the longest increasing run of old positions;
  // everything else moved.
  const pos = new Map(from.map((k, i) => [k, i]));
  const seq = to.map((k) => pos.get(k));
  const tails = [], tailIdx = [], prev = new Array(seq.length).fill(-1);
  seq.forEach((v, i) => {
    let lo = 0, hi = tails.length;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (tails[mid] < v) lo = mid + 1; else hi = mid; }
    tails[lo] = v; tailIdx[lo] = i;
    prev[i] = lo > 0 ? tailIdx[lo - 1] : -1;
  });
  const keep = new Set();
  for (let i = tailIdx[tails.length - 1] ?? -1; i >= 0; i = prev[i]) keep.add(i);
  const moved = to.map((_, i) => i).filter((i) => !keep.has(i));
  if (!moved.length) return null;
  const at = moved[0];
  if (moved[moved.length - 1] - at !== moved.length - 1) return null; // not one block
  const keys = moved.map((i) => to[i]);
  return sameKeys(moveKeys(from, keys, at), to) ? { move: keys, to: at } : null;
}

/**
 * Operations that turn the list from the last sync (`baseKeys`) into the
 * local songs: removals, additions (each run of new songs in one op, at its
 * place), then one move or the whole order if songs were rearranged.
 * `slim` shapes a song for the server. `keepOrder`: don't send the order
 * (first sync of a playlist: the server's order wins).
 */
function planOps(baseKeys, tracks, slim, { keepOrder = false } = {}) {
  const local = uniqTracks(tracks);
  const keys = local.map((t) => t.key);
  const here = new Set(keys), before = new Set(baseKeys);
  const ops = [];
  const removed = baseKeys.filter((k) => !here.has(k));
  if (removed.length) ops.push({ remove: removed });
  const cur = baseKeys.filter((k) => here.has(k));
  for (let i = 0; i < local.length;) {
    if (before.has(keys[i])) { i++; continue; }
    let j = i;
    while (j < local.length && !before.has(keys[j])) j++;
    const at = i === 0 ? 0 : cur.indexOf(keys[i - 1]) + 1;
    ops.push({ add: local.slice(i, j).map(slim), at });
    cur.splice(at, 0, ...keys.slice(i, j));
    i = j;
  }
  if (!keepOrder && !sameKeys(cur, keys)) ops.push(singleMove(cur, keys) || { order: keys });
  return ops;
}

/**
 * A change from the server applied to local songs, the way the server
 * applied it. `fromCloud` turns a server song into a local one.
 */
function applyOp(tracks, op, fromCloud) {
  const list = tracks.slice();
  if (op.add) {
    const have = new Set(list.map((t) => t.key));
    const fresh = uniqTracks(op.add.filter((t) => t && !have.has(t.key))).map(fromCloud);
    list.splice(Math.max(0, Math.min(op.at ?? list.length, list.length)), 0, ...fresh);
    return list;
  }
  if (op.remove) {
    const gone = new Set(op.remove);
    return list.filter((t) => !gone.has(t.key));
  }
  const byKey = new Map(list.map((t) => [t.key, t]));
  if (op.move) return moveKeys(list.map((t) => t.key), op.move, op.to ?? 0).map((k) => byKey.get(k));
  if (op.order) {
    const rank = new Map(op.order.map((k, i) => [k, i]));
    return list.map((t, i) => [t, rank.has(t.key) ? rank.get(t.key) : op.order.length + i]).sort((a, b) => a[1] - b[1]).map((x) => x[0]);
  }
  return null; // replace / unknown: fetch the whole playlist
}
