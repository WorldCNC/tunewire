// Error reports from the Tunewire app → the developer's Discord channel.
//
// The Discord webhook URL is a function secret (DISCORD_WEBHOOK), so it never
// ships in the app. Every field is checked and trimmed, mentions and links
// are defused, and reports are rate-limited per address (hashed) and in
// total through the report_allow() database function.
//
// Secrets: DISCORD_WEBHOOK. Provided by Supabase: SUPABASE_URL,
// SUPABASE_SERVICE_ROLE_KEY.

const KINDS = new Set(["error", "frontend", "crash"]);
const COLORS: Record<string, number> = { crash: 0xe53935, frontend: 0xfb8c00, error: 0xfdd835 };
const PER_ADDRESS_PER_HOUR = 20;
const TOTAL_PER_HOUR = 400;

export interface Env {
  webhook?: string;
  supabaseUrl?: string;
  serviceKey?: string;
}

export interface Report {
  kind: string;
  title: string;
  details: string;
  version: string;
  os: string;
}

/** The report, cleaned, or why it was refused. */
export function parseReport(body: unknown): Report | string {
  if (!body || typeof body !== "object") return "not a report";
  const b = body as Record<string, unknown>;
  const str = (v: unknown, max: number) => (typeof v === "string" ? v.slice(0, max) : "");
  const kind = str(b.kind, 20);
  if (!KINDS.has(kind)) return "unknown kind";
  const title = str(b.title, 240).trim();
  if (!title) return "empty report";
  const version = str(b.version, 20);
  if (!/^\d+\.\d+\.\d+([-.][0-9A-Za-z.]+)?$/.test(version)) return "bad version";
  return { kind, title, details: str(b.details, 3500), version, os: str(b.os, 40).replace(/[^\w .-]/g, "") };
}

/** No pings, no clickable links, no breaking out of the code block. */
export function defuse(s: string): string {
  return s
    .replace(/@/g, "@​")
    .replace(/```/g, "`​``")
    .replace(/https?:\/\//gi, (m) => m.replace("//", "/​/"))
    .replace(/discord(?:app)?\.(?:gg|com\/invite)/gi, (m) => m.replace(".", ".​"));
}

export function discordPayload(r: Report) {
  return {
    username: "Tunewire error reports",
    allowed_mentions: { parse: [] },
    embeds: [{
      title: defuse(`${r.kind}: ${r.title}`).slice(0, 256),
      description: ("```\n" + defuse(r.details) + "\n```").slice(0, 4000),
      color: COLORS[r.kind] ?? COLORS.error,
      fields: [
        { name: "Version", value: r.version, inline: true },
        { name: "OS", value: r.os || "?", inline: true },
      ],
    }],
  };
}

async function sha256(s: string): Promise<string> {
  const d = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s));
  return [...new Uint8Array(d)].map((x) => x.toString(16).padStart(2, "0")).join("");
}

async function allow(env: Env, bucket: string, max: number, f: typeof fetch): Promise<boolean> {
  const r = await f(`${env.supabaseUrl}/rest/v1/rpc/report_allow`, {
    method: "POST",
    headers: { apikey: env.serviceKey!, Authorization: `Bearer ${env.serviceKey}`, "Content-Type": "application/json" },
    body: JSON.stringify({ b: bucket, max_count: max, window_secs: 3600 }),
  });
  return r.ok && (await r.json()) === true;
}

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

export async function handle(req: Request, env: Env, f: typeof fetch = fetch): Promise<Response> {
  if (req.method !== "POST") return json(405, { error: "POST only" });
  if (!env.webhook || !env.supabaseUrl || !env.serviceKey) return json(503, { error: "reports are not set up" });
  const text = await req.text();
  if (text.length > 16_000) return json(413, { error: "too large" });
  let body: unknown;
  try { body = JSON.parse(text); } catch { return json(400, { error: "not JSON" }); }
  const r = parseReport(body);
  if (typeof r === "string") return json(400, { error: r });

  const ip = (req.headers.get("x-forwarded-for") || "").split(",")[0].trim() || "unknown";
  const who = `ip:${(await sha256(`tunewire-report:${ip}`)).slice(0, 32)}`;
  if (!(await allow(env, who, PER_ADDRESS_PER_HOUR, f)) || !(await allow(env, "all", TOTAL_PER_HOUR, f))) {
    return json(429, { error: "slow down" });
  }
  const d = await f(env.webhook, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(discordPayload(r)) });
  return d.ok ? json(202, { ok: true }) : json(502, { error: "could not deliver" });
}

if (import.meta.main) {
  Deno.serve((req) => handle(req, {
    webhook: Deno.env.get("DISCORD_WEBHOOK"),
    supabaseUrl: Deno.env.get("SUPABASE_URL"),
    serviceKey: Deno.env.get("SUPABASE_SERVICE_ROLE_KEY"),
  }));
}
