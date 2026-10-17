import { assert, assertEquals, assertStringIncludes } from "jsr:@std/assert@1.0.13";
import { handle, route } from "./index.ts";

const env = { supabaseUrl: "https://db.test", anonKey: "anon", publicUrl: "https://share.test" };
const PID = "11111111-1111-1111-1111-111111111111";

const fake = (rows: Record<string, unknown>) =>
  (async (url: string) => {
    for (const [k, v] of Object.entries(rows)) if (url.includes(k)) return new Response(JSON.stringify(v));
    return new Response("[]");
  }) as typeof fetch;

const get = (path: string, f: typeof fetch) => handle(new Request(`https://share.test${path}`), env, f);

Deno.test("routes work with and without the functions prefix", () => {
  assertEquals(route("/functions/v1/share/p/x"), ["p", "x"]);
  assertEquals(route("/share/s/abc"), ["s", "abc"]);
  assertEquals(route("/p/x"), ["p", "x"]);
});

Deno.test("playlist links get a real preview", async () => {
  const f = fake({ playlist_preview: [{ title: "Road trip", description: "summer", owner_name: "Alex", songs: 42, thumbnail: "https://i.ytimg.com/vi/x/hqdefault.jpg", has_cover: true }] });
  const html = await (await get(`/share/p/${PID}`, f)).text();
  assertStringIncludes(html, '<meta property="og:title" content="Road trip">');
  assertStringIncludes(html, "42 songs · by Alex · summer");
  assertStringIncludes(html, `https://share.test/img/p/${PID}`);
  assertStringIncludes(html, `#p=${PID}`);
});

Deno.test("everything shown is escaped", async () => {
  const f = fake({ playlist_preview: [{ title: '"><script>alert(1)</script>', description: "", owner_name: "<b>", songs: 1, thumbnail: 'javascript:alert(1)', has_cover: false }] });
  const html = await (await get(`/share/p/${PID}`, f)).text();
  assert(!html.includes("<script>alert"));
  assert(!html.includes("<b>"));
  assert(!html.includes("javascript:"), "only https images");
});

Deno.test("song links decode their payload", async () => {
  const code = btoa(JSON.stringify({ t: "Song", a: "Artist", i: "https://i.scdn.co/image/x" })).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const html = await (await get(`/share/s/${code}`, fake({}))).text();
  assertStringIncludes(html, '<meta property="og:title" content="Song">');
  assertStringIncludes(html, "Artist");
  assertStringIncludes(html, "https://i.scdn.co/image/x");
});

Deno.test("private or bad ids fall back safely", async () => {
  const html = await (await get(`/share/p/${PID}`, fake({}))).text();
  assertStringIncludes(html, "private or no longer shared");
  const r = await get(`/share/p/not-a-uuid&select=*`, fake({}));
  assertEquals(r.status, 200);
  assertStringIncludes(await r.text(), "<title>Tunewire</title>");
  assertEquals((await get(`/share/x/y`, fake({}))).status, 302);
});

Deno.test("data: covers are served as images", async () => {
  const png = "iVBORw0KGgo=";
  const r = await get(`/share/img/p/${PID}`, fake({ playlist_cover: `data:image/png;base64,${png}` }));
  assertEquals(r.headers.get("content-type"), "image/png");
  assertEquals(new Uint8Array(await r.arrayBuffer())[1], 0x50);
  const r2 = await get(`/share/img/p/${PID}`, fake({ playlist_cover: "data:text/html;base64,PHNjcmlwdD4=" }));
  assertEquals(r2.status, 302, "only images");
});
