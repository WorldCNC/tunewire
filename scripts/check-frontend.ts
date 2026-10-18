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

// The app's scripts are classic scripts sharing one global scope: two files
// declaring the same top-level name either stop the second file from loading
// (const/let/class: "already declared") or silently replace a function.
const declared = new Map<string, { at: string; fn: boolean }>();
let clashes = 0;
const appScripts = files.filter((f) => f.startsWith(`${root}src/`));
for (const f of appScripts) {
  const at = f.slice(root.length);
  const text = await Deno.readTextFile(f);
  for (const line of text.split("\n")) {
    const fn = line.match(/^(?:async\s+)?function\*?\s+([A-Za-z_$][\w$]*)/);
    const m = fn || line.match(/^(?:const|let|var|class)\s+([A-Za-z_$][\w$]*)/);
    const names = m ? [m[1]] : (line.match(/^(?:const|let|var)\s+\{([^}]*)\}\s*=/)?.[1].split(",").map((x) => x.split(":").pop()!.trim()).filter(Boolean) ?? []);
    for (const name of names) {
      const prev = declared.get(name);
      // Allowed: a stub function that loads src/js/lazy/<x>.js, which then
      // replaces it with the real function of the same name.
      const lazyStub = prev && prev.fn && !!fn && (prev.at.includes("/lazy/") !== at.includes("/lazy/"));
      if (prev && prev.at !== at && !lazyStub) {
        clashes++;
        console.error(`${at}: "${name}" is already declared in ${prev.at}`);
      } else if (!prev) declared.set(name, { at, fn: !!fn });
    }
  }
}
console.log(clashes ? `${clashes} global name clash(es)` : `${declared.size} global names, no clashes`);
if (clashes) Deno.exit(1);
