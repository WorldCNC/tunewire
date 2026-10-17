// Link previews for shared Tunewire links.
//
// Chat apps (Discord, WhatsApp, iMessage, Slack…) read Open Graph tags from
// the page a link points to, and the GitHub Pages landing page can't vary
// them per playlist (its routes are #fragments). Links made through this
// function get a real title, description and cover in the preview; people
// who click are sent on to the landing page right away.
//
//   <base>/p/<playlist id>   <base>/s/<song code>   <base>/u/<user id>
//   <base>/j/<code>          <base>/c/<code>        <base>/img/p/<id>
//
// Supabase rewrites text/html from *.supabase.co to text/plain, so this must
// be served from a custom domain (or any Deno host; it has no Supabase-only
// code). Set SHARE_PUBLIC_URL to the address it's served at, and build the
// app with TUNEWIRE_SHARE_URL set to the same.

const LANDING = "https://worldcnc.github.io/tunewire/";
const DEFAULT_IMAGE = `${LANDING}og.png`;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const CODE = /^[A-Za-z0-9_-]{1,4000}$/;

export interface Env {
  supabaseUrl?: string;
  anonKey?: string;
  /** Where this function is reachable, e.g. https://share.tunewire.app */
  publicUrl?: string;
}

export interface Preview {
  kind: string;
  title: string;
  description: string;
  image: string;
  /** Where people who click end up. */
  target: string;
  large: boolean;
}

export const esc = (s: unknown) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]!));

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** Only web images can be preview images. */
const httpsImage = (u: unknown) => (typeof u === "string" && /^https:\/\/[^\s"'<>]+$/.test(u) ? u : "");

function b64urlJson(code: string): Record<string, unknown> | null {
  try {
    const bin = atob(code.replace(/-/g, "+").replace(/_/g, "/"));
    const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
    const o = JSON.parse(new TextDecoder().decode(bytes));
    return o && typeof o === "object" ? o : null;
  } catch {
    return null;
  }
}

/** /share/p/<id> or /functions/v1/share/p/<id> → ["p", "<id>"] */
export function route(pathname: string): string[] {
  const parts = pathname.split("/").filter(Boolean);
  const i = parts.indexOf("share");
  return i >= 0 ? parts.slice(i + 1) : parts;
}

async function rest(env: Env, path: string, f: typeof fetch, body?: unknown) {
  const r = await f(`${env.supabaseUrl}/rest/v1/${path}`, {
    method: body ? "POST" : "GET",
    headers: { apikey: env.anonKey!, Authorization: `Bearer ${env.anonKey}`, "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

export async function preview(kind: string, code: string, env: Env, f: typeof fetch = fetch): Promise<Preview> {
  const base = (env.publicUrl || "").replace(/\/$/, "");
  const target = `${LANDING}#${kind}=${encodeURIComponent(code)}`;
  const generic: Preview = { kind, title: "Tunewire", description: "A free music player for YouTube Music, SoundCloud, YouTube and Spotify.", image: DEFAULT_IMAGE, target, large: true };

  if (kind === "p" && UUID.test(code)) {
    const [p] = await rest(env, "rpc/playlist_preview", f, { pid: code }).catch(() => []);
    if (!p) return { ...generic, title: "Playlist on Tunewire", description: "This playlist is private or no longer shared." };
    const by = p.owner_name ? ` · by ${p.owner_name}` : "";
    const desc = p.description ? ` · ${p.description}` : "";
    const image = p.has_cover && base ? `${base}/img/p/${code}` : httpsImage(p.thumbnail) || DEFAULT_IMAGE;
    return { kind, title: clip(p.title, 100), description: clip(`Playlist · ${p.songs} song${p.songs === 1 ? "" : "s"}${by}${desc}`, 200), image, target, large: false };
  }
  if (kind === "s" && CODE.test(code)) {
    const o = b64urlJson(code);
    if (!o || typeof o.t !== "string") return generic;
    return { kind, title: clip(o.t, 100), description: clip(`Song${typeof o.a === "string" && o.a ? ` · ${o.a}` : ""} · Listen on Tunewire`, 200), image: httpsImage(o.i) || DEFAULT_IMAGE, target, large: false };
  }
  if (kind === "u" && UUID.test(code)) {
    const [p] = await rest(env, `profiles?id=eq.${code}&select=*`, f).catch(() => []);
    if (!p) return generic;
    const banner = httpsImage(p.banner);
    return { kind, title: clip(`${p.name} on Tunewire`, 100), description: "See their public playlists, followers and who they follow.", image: banner || httpsImage(p.avatar) || DEFAULT_IMAGE, target, large: !!banner };
  }
  if (kind === "j") return { ...generic, title: "Listen together on Tunewire", description: "You're invited to a listening session: everyone hears the same song at the same moment." };
  if (kind === "c") return { ...generic, title: "Join a collaborative playlist", description: "You're invited to add songs to a playlist together on Tunewire." };
  return generic;
}

export function page(p: Preview, selfUrl: string): string {
  const t = esc(p.title), d = esc(p.description), img = esc(p.image), target = esc(p.target);
  return `<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${t}</title>
<meta name="description" content="${d}">
<meta property="og:site_name" content="Tunewire">
<meta property="og:type" content="${p.kind === "s" ? "music.song" : p.kind === "p" ? "music.playlist" : "website"}">
<meta property="og:title" content="${t}">
<meta property="og:description" content="${d}">
<meta property="og:image" content="${img}">
<meta property="og:url" content="${esc(selfUrl)}">
<meta name="twitter:card" content="${p.large ? "summary_large_image" : "summary"}">
<meta name="twitter:title" content="${t}">
<meta name="twitter:description" content="${d}">
<meta name="twitter:image" content="${img}">
<meta name="theme-color" content="#4d8dff">
<link rel="canonical" href="${target}">
<meta http-equiv="refresh" content="0; url=${target}">
</head><body style="font:16px system-ui;background:#0d0d0f;color:#eee;display:grid;place-items:center;height:90vh">
<p><a style="color:#9cc0ff" href="${target}">Open ${t} in Tunewire</a></p>
</body></html>`;
}

/** The playlist's cover picture (data: URLs are served as images). */
export async function cover(code: string, env: Env, f: typeof fetch = fetch): Promise<Response> {
  if (!UUID.test(code)) return new Response("not found", { status: 404 });
  const c = await rest(env, "rpc/playlist_cover", f, { pid: code }).catch(() => null);
  if (typeof c === "string") {
    const m = c.match(/^data:(image\/(?:png|jpeg|webp|gif));base64,([A-Za-z0-9+/=]+)$/);
    if (m) {
      const bytes = Uint8Array.from(atob(m[2]), (ch) => ch.charCodeAt(0));
      return new Response(bytes, { headers: { "Content-Type": m[1], "Cache-Control": "public, max-age=3600" } });
    }
    if (httpsImage(c)) return Response.redirect(c, 302);
  }
  return Response.redirect(DEFAULT_IMAGE, 302);
}

export async function handle(req: Request, env: Env, f: typeof fetch = fetch): Promise<Response> {
  if (req.method !== "GET" && req.method !== "HEAD") return new Response("GET only", { status: 405 });
  const url = new URL(req.url);
  const [kind, a, b] = route(url.pathname);
  if (kind === "img" && a === "p" && b) return cover(b, env, f);
  if (!kind || !"pscju".includes(kind) || kind.length !== 1 || !a) return Response.redirect(LANDING, 302);
  const p = await preview(kind, decodeURIComponent(a), env, f);
  return new Response(page(p, url.toString()), {
    headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "public, max-age=300", "X-Content-Type-Options": "nosniff" },
  });
}

if (import.meta.main) {
  Deno.serve((req) => handle(req, {
    supabaseUrl: Deno.env.get("SUPABASE_URL"),
    anonKey: Deno.env.get("SUPABASE_ANON_KEY"),
    publicUrl: Deno.env.get("SHARE_PUBLIC_URL"),
  }));
}
