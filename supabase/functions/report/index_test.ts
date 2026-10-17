import { assert, assertEquals } from "jsr:@std/assert@1.0.13";
import { defuse, discordPayload, handle, parseReport } from "./index.ts";

const env = { webhook: "https://discord.test/hook", supabaseUrl: "https://db.test", serviceKey: "service" };

function fakeFetch(allowed: boolean[]) {
  const calls: { url: string; body: string }[] = [];
  let i = 0;
  const f = (async (url: string, init?: RequestInit) => {
    calls.push({ url, body: String(init?.body ?? "") });
    if (url.includes("report_allow")) return new Response(JSON.stringify(allowed[i++] ?? true));
    return new Response(null, { status: 204 });
  }) as typeof fetch;
  return { f, calls };
}

const req = (body: unknown, ip = "1.2.3.4") =>
  new Request("https://x/report", { method: "POST", body: JSON.stringify(body), headers: { "x-forwarded-for": ip } });
const good = { kind: "error", title: "boom", details: "at x", version: "1.0.5", os: "linux x86_64" };

Deno.test("valid reports are delivered with mentions disabled", async () => {
  const { f, calls } = fakeFetch([true, true]);
  const r = await handle(req(good), env, f);
  assertEquals(r.status, 202);
  const sent = JSON.parse(calls.at(-1)!.body);
  assertEquals(calls.at(-1)!.url, env.webhook);
  assertEquals(sent.allowed_mentions, { parse: [] });
  // The address is never sent anywhere in clear.
  assert(!calls.some((c) => c.body.includes("1.2.3.4")));
});

Deno.test("rate limits stop delivery", async () => {
  const { f, calls } = fakeFetch([false]);
  assertEquals((await handle(req(good), env, f)).status, 429);
  assert(!calls.some((c) => c.url === env.webhook));
});

Deno.test("junk is refused", async () => {
  const { f } = fakeFetch([]);
  assertEquals((await handle(req({ ...good, kind: "spam" }), env, f)).status, 400);
  assertEquals((await handle(req({ ...good, version: "<b>" }), env, f)).status, 400);
  assertEquals((await handle(new Request("https://x", { method: "GET" }), env, f)).status, 405);
  assertEquals((await handle(new Request("https://x", { method: "POST", body: "x".repeat(20000) }), env, f)).status, 413);
  assertEquals(typeof parseReport({ ...good, title: "" }), "string");
});

Deno.test("mentions, links and code fences are defused", () => {
  const s = defuse("@everyone see https://evil.test ``` discord.gg/x");
  assert(!s.includes("@everyone"));
  assert(!s.includes("https://evil"));
  assert(!s.includes("```"));
  assert(!s.includes("discord.gg"));
  const p = discordPayload({ ...good, title: "@here" } as never);
  assert(!p.embeds[0].title.includes("@here"));
});
