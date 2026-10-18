// Library sync, end to end: two "devices" (each its own headless browser on
// the mock backend, with its own library and sync state) signed in to the
// same account on a local Supabase. Run with scripts/test-sync.sh.
import { launch, type Page } from "jsr:@astral/astral";

const API = "http://127.0.0.1:54321";
const ANON = Deno.env.get("ANON")!;
const SERVICE = Deno.env.get("SERVICE")!;

async function user(email: string) {
  await fetch(`${API}/auth/v1/admin/users`, {
    method: "POST",
    headers: { apikey: SERVICE, Authorization: `Bearer ${SERVICE}`, "Content-Type": "application/json" },
    body: JSON.stringify({ email, password: "pw-123456", email_confirm: true, user_metadata: { full_name: email.split("@")[0] } }),
  }).then((r) => r.text());
  const r = await fetch(`${API}/auth/v1/token?grant_type=password`, {
    method: "POST",
    headers: { apikey: ANON, "Content-Type": "application/json" },
    body: JSON.stringify({ email, password: "pw-123456" }),
  }).then((r) => r.json());
  if (!r.access_token) throw new Error(JSON.stringify(r));
  return { access_token: r.access_token, user_id: r.user.id, name: "Tester", avatar: null, expires_at: Date.now() + r.expires_in * 1000 };
}

const browser = await launch();
const browsers = [browser];
let d1: Page | undefined, d2: Page | undefined;
const errors: string[] = [];

async function device(label: string, session: unknown, prep: string) {
  // Each device in its own browser: a background tab would be throttled.
  const b = label === "dev1" ? browser : await launch();
  if (b !== browser) browsers.push(b);
  const page = await b.newPage("http://127.0.0.1:1420/");
  page.addEventListener("console", (e) => {
    const t = e.detail.text;
    if (/error|sync|warn/i.test(e.detail.type) || /^sync/.test(t)) {
      if (!/mock\] unhandled|favicon|DevTools/.test(t)) console.log(`  [${label} ${e.detail.type}] ${t.slice(0, 300)}`);
      if (e.detail.type === "error") errors.push(`${label}: ${t}`);
    }
  });
  await page.waitForFunction("typeof state !== 'undefined' && !!state.settings && typeof syncTask === 'function'");
  await page.evaluate(`(async () => {
    ${prep}
    await updateSettings({ privacy_asked: true, cloud_privacy_asked: true, sync_asked: true, sync_library: true });
    window.__SB = { url: ${JSON.stringify(API)}, anon: ${JSON.stringify(ANON)} };
    window.__SBsession = ${JSON.stringify(session)};
    await cloudInit();
  })()`);
  return page;
}

const keysOf = (page: Page, name: string) =>
  page.evaluate(`call('list_playlists').then((ps) => (ps.find((p) => p.name === ${JSON.stringify(name)}) || { tracks: null }).tracks?.map((t) => t.key) ?? null)`) as Promise<string[] | null>;
const names = (page: Page) => page.evaluate(`call('list_playlists').then((ps) => ps.map((p) => p.name).sort())`) as Promise<string[]>;
const likes = (page: Page) => page.evaluate(`call('list_favorites').then((f) => f.map((t) => t.key))`) as Promise<string[]>;
const run = (page: Page, js: string) => page.evaluate(`(async () => { ${js} })()`);

async function until(what: string, f: () => Promise<boolean>, ms = 20000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    if (await f()) { console.log(`ok   ${what} (${Date.now() - t0} ms)`); return; }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`timed out: ${what}`);
}
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

const me = await user(`sync${Date.now()}@test.local`);
// Device 1: the mock's library (Late night drive, Gym, 2 liked songs).
d1 = await device("dev1", me, "");
await until("device 1 uploaded its library", async () =>
  (await run(d1, `return state.playlists.filter((p) => p.cloud_id).length`)) === 2 && !!(await run(d1, `return bases().liked`)));

// Device 2: its own playlist instead, and one more liked song.
d2 = await device("dev2", me, `
  for (const p of await call('list_playlists')) await call('delete_playlist', { id: p.id });
  const all = (await call('search_tracks', { query: '', source: 'all', limit: 20 })).tracks;
  await call('create_playlist', { name: 'From device 2', tracks: all.slice(5, 7) });
  await call('toggle_favorite', { track: all[6] });
`);
const all = await run(d1, `return (await call('search_tracks', { query: '', source: 'all', limit: 20 })).tracks.map((t) => t.key)`) as string[];

await until("both devices have all three playlists", async () => {
  const [a, b] = [await names(d1), await names(d2)];
  return same(a, ["From device 2", "Gym", "Late night drive"]) && same(a, b);
});
for (const n of ["Gym", "Late night drive", "From device 2"]) {
  await until(`"${n}" has the same songs on both`, async () => same(await keysOf(d1, n), await keysOf(d2, n)));
}
await until("Liked Songs merged on both", async () => {
  const [a, b] = [await likes(d1), await likes(d2)];
  return a.length === 3 && same([...a].sort(), [...b].sort());
});

// Live updates need both devices connected (slow right after the stack starts).
await until("both devices are listening live", async () =>
  !!(await run(d1!, `return syncs.owner?.live`)) && !!(await run(d2!, `return syncs.owner?.live`)), 60000);

// Live edits from your other device (the old "it was me, skip it" bug).
await run(d1, `const gym = state.playlists.find((p) => p.name === 'Gym'); const t = (await call('search_tracks', { query: '', source: 'all', limit: 20 })).tracks[6]; await call('add_to_playlist', { id: gym.id, tracks: [t] });`);
await until("an add on device 1 shows up on device 2", async () => (await keysOf(d2, "Gym"))?.includes(all[6]) ?? false);

await run(d2, `const gym = (await call('list_playlists')).find((p) => p.name === 'Gym');
  await call('remove_many_from_playlist', { id: gym.id, indices: [0], keys: [gym.tracks[0].key] });
  await call('update_playlist_meta', { id: gym.id, patch: { name: 'Gym 2', folder: 'Sport', pinned: true } });`);
await until("a removal + rename + folder on device 2 shows up on device 1", async () => {
  const k1 = await keysOf(d1, "Gym 2");
  const p = await run(d1, `const p = (await call('list_playlists')).find((x) => x.name === 'Gym 2'); return p && [p.folder, p.pinned]`);
  return !!k1 && same(k1, await keysOf(d2, "Gym 2")) && same(p, ["Sport", true]);
});

// A drag on device 2: one block moved.
await run(d2, `const p = (await call('list_playlists')).find((x) => x.name === 'Late night drive');
  await call('move_many_in_playlist', { id: p.id, indices: [0, 1], to: p.tracks.length - 2, keys: [p.tracks[0].key, p.tracks[1].key] });`);
await until("a reorder on device 2 shows up on device 1", async () => {
  const [a, b] = [await keysOf(d1, "Late night drive"), await keysOf(d2, "Late night drive")];
  return same(a, b);
});

// Both at the same moment, different songs: nobody's edit is lost.
await Promise.all([
  run(d1, `const p = state.playlists.find((x) => x.name === 'From device 2'); await call('add_to_playlist', { id: p.id, tracks: [(await call('search_tracks', { query: '', source: 'all', limit: 20 })).tracks[0]] });`),
  run(d2, `const p = (await call('list_playlists')).find((x) => x.name === 'From device 2'); await call('add_to_playlist', { id: p.id, tracks: [(await call('search_tracks', { query: '', source: 'all', limit: 20 })).tracks[1]] });`),
]);
await until("concurrent adds: both songs on both devices", async () => {
  const [a, b] = [await keysOf(d1, "From device 2"), await keysOf(d2, "From device 2")];
  return !!a && a.includes(all[0]) && a.includes(all[1]) && same(a, b);
});

// Liked Songs.
await run(d1, `await call('toggle_favorite', { track: (await call('search_tracks', { query: '', source: 'all', limit: 20 })).tracks[3] });`);
await until("a like on device 1 shows up on device 2", async () => (await likes(d2)).includes(all[3]));

// Offline on device 1: edits wait, then go out.
await run(d1, `window.__SBsession = null; cloud.session = null; cloudChanged();
  const p = state.playlists.find((x) => x.name === 'Gym 2'); await call('remove_many_from_playlist', { id: p.id, indices: [0], keys: [p.tracks[0].key] });`);
await new Promise((r) => setTimeout(r, 2500));
const offlineEdit = await keysOf(d1, "Gym 2");
if (same(offlineEdit, await keysOf(d2, "Gym 2"))) throw new Error("edit went out while signed out?");
await run(d1, `window.__SBsession = ${JSON.stringify(me)}; cloud.session = window.__SBsession; cloudChanged();`);
await until("an edit made while signed out goes out after signing in", async () => same(await keysOf(d2, "Gym 2"), offlineEdit));

// Delete on device 2.
await run(d2, `const p = (await call('list_playlists')).find((x) => x.name === 'From device 2'); await call('delete_playlist', { id: p.id });`);
await until("a delete on device 2 removes it on device 1", async () => !(await names(d1)).includes("From device 2"));

// Sharing a synced playlist opens it up; stopping makes it private again.
const id = await run(d1, `const p = state.playlists.find((x) => x.name === 'Gym 2'); return await publishPlaylist(p);`);
const vis = await run(d1, `return (await db('GET', 'playlists?id=eq.' + ${JSON.stringify(id)} + '&select=visibility'))[0].visibility`);
if (vis !== "unlisted") throw new Error(`shared visibility: ${vis}`);
console.log("ok   sharing a synced playlist makes its link work");

// Steady state: how long a live edit takes.
await run(d1, `const p = state.playlists.find((x) => x.name === 'Late night drive'); await call('add_to_playlist', { id: p.id, tracks: [(await call('search_tracks', { query: '', source: 'all', limit: 20 })).tracks[1]] });`);
await until("steady-state live edit", async () => (await keysOf(d2, "Late night drive"))?.includes(all[1]) ?? false);

// Another account on device 1: account A's playlists are left alone…
const other = await user(`other${Date.now()}@test.local`);
const before = await run(d2, `return (await db('GET', 'playlists?select=id,deleted_at,version&order=id')).map((r) => [r.id, r.deleted_at, r.version])`);
await run(d1, `window.__SBsession = ${JSON.stringify(other)}; cloud.session = window.__SBsession; cloudChanged();`);
await new Promise((r) => setTimeout(r, 6000));
const linkedA = await run(d1, `return (await call('list_playlists')).filter((p) => p.cloud_id).length`);
// …and back on A: nothing was deleted or changed online.
await run(d1, `window.__SBsession = ${JSON.stringify(me)}; cloud.session = window.__SBsession; cloudChanged();`);
await new Promise((r) => setTimeout(r, 6000));
const after = await run(d2, `return (await db('GET', 'playlists?select=id,deleted_at,version&order=id')).map((r) => [r.id, r.deleted_at, r.version])`);
if (!same(before, after)) throw new Error(`account switch changed A's rows:\n${JSON.stringify(before)}\n${JSON.stringify(after)}`);
console.log(`ok   switching accounts on a device leaves the other account's playlists alone (${linkedA} still linked)`);
const otherRows = await run(d1, `return (await db('GET', 'playlists?select=title,kind')).length`);
console.log(`     (account B, back on A: rows visible ${otherRows})`);

for (const b of browsers) await b.close();
if (errors.length) { console.log("console errors:\n" + errors.join("\n")); Deno.exit(1); }
console.log("all sync checks passed");
