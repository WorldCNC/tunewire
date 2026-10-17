// Parses every frontend script (they're classic scripts, no build step), so a
// syntax error can't reach a release. Run: deno run --allow-read scripts/check-frontend.ts
const files: string[] = [];
const walk = async (dir: string) => {
  for await (const e of Deno.readDir(dir)) {
    const p = `${dir}/${e.name}`;
    if (e.isDirectory) await walk(p);
    else if (p.endsWith(".js")) files.push(p);
  }
};
const root = new URL("..", import.meta.url).pathname;
await walk(`${root}src`);
await walk(`${root}dev`);
files.push(`${root}docs/config.js`);
let bad = 0;
for (const f of files) {
  try {
    new Function(await Deno.readTextFile(f));
  } catch (e) {
    bad++;
    console.error(`${f.slice(root.length)}: ${(e as Error).message}`);
  }
}
console.log(bad ? `${bad} file(s) with syntax errors` : `${files.length} scripts parse`);
if (bad) Deno.exit(1);
