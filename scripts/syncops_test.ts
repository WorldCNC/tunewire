// Tests for src/js/syncops.js (the playlist sync arithmetic):
//   deno test --allow-read scripts/syncops_test.ts
// A tiny copy of playlist_op() replays every planned op the way the server
// does; the result must be the local list.
const src = await Deno.readTextFile(new URL("../src/js/syncops.js", import.meta.url));
const ops = new Function(`${src}; return { uniqKeys, planOps, applyOp, singleMove, coverSig };`)();
const { planOps, applyOp, singleMove, uniqKeys, coverSig } = ops;

const T = (k: string) => ({ key: k, title: k });
const slim = (t: { key: string }) => ({ key: t.key, title: t.key });
const fromCloud = (t: { key: string }) => T(t.key);
const keys = (ts: { key: string }[]) => ts.map((t) => t.key);

// The server's playlist_op, on key lists.
function server(list: string[], op: Record<string, unknown>): string[] {
  return keys(applyOp(list.map(T), op, fromCloud));
}
function assertEq(a: unknown, b: unknown, msg = "") {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${msg}\n  got:  ${JSON.stringify(a)}\n  want: ${JSON.stringify(b)}`);
}
function roundTrip(base: string[], local: string[], label: string) {
  let s = base.slice();
  const planned = planOps(base, local.map(T), slim);
  for (const op of planned) s = server(s, op);
  assertEq(s, [...new Set(local)], label);
  return planned;
}

Deno.test("no change, no ops", () => {
  assertEq(planOps(["a", "b"], ["a", "b"].map(T), slim), []);
});

Deno.test("adds land where they are locally, in one op per run", () => {
  const p = roundTrip(["a", "b", "c"], ["x", "a", "y", "z", "b", "c", "w"], "adds");
  assertEq(p.length, 3, "three runs of new songs");
});

Deno.test("removals and duplicates", () => {
  roundTrip(["a", "b", "c", "d"], ["b", "d"], "remove");
  roundTrip(["a"], ["a", "b", "b", "a"], "duplicates collapse like the server");
});

Deno.test("a dragged block is one move, not the whole order", () => {
  const base = ["a", "b", "c", "d", "e", "f"];
  const p = roundTrip(base, ["a", "d", "e", "b", "c", "f"], "block move");
  assertEq(p.length, 1);
  assertEq("move" in p[0], true, "move op");
  assertEq(singleMove(["a", "b", "c"], ["c", "a", "b"]), { move: ["c"], to: 0 });
});

Deno.test("shuffled: the whole order", () => {
  const p = roundTrip(["a", "b", "c", "d", "e"], ["e", "c", "a", "d", "b"], "shuffle");
  assertEq("order" in p[p.length - 1], true);
});

Deno.test("everything at once", () => {
  roundTrip(["a", "b", "c", "d", "e"], ["n1", "e", "c", "n2", "a"], "mixed");
  roundTrip([], ["a", "b"], "first upload");
  roundTrip(["a", "b"], [], "emptied");
});

Deno.test("random edits always round-trip", () => {
  let seed = 7;
  const rnd = (n: number) => ((seed = (seed * 1103515245 + 12345) % 2147483648) % n);
  for (let round = 0; round < 500; round++) {
    const base = Array.from({ length: rnd(12) }, (_, i) => `k${i}`);
    let local = base.filter(() => rnd(4) > 0);
    for (let i = rnd(4); i > 0; i--) local.splice(rnd(local.length + 1), 0, `n${round}_${i}`);
    if (rnd(2)) local = local.sort(() => rnd(3) - 1);
    roundTrip(base, local, `round ${round}: ${base} -> ${local}`);
  }
});

Deno.test("first sync keeps the server's order", () => {
  const p = planOps(["a", "b"], ["b", "a", "c"].map(T), slim, { keepOrder: true });
  assertEq(p.some((o: Record<string, unknown>) => "order" in o || "move" in o), false);
});

Deno.test("remote ops on local songs", () => {
  const local = ["a", "b", "c"].map(T);
  assertEq(keys(applyOp(local, { add: [{ key: "b" }, { key: "x" }], at: 1 }, fromCloud)), ["a", "x", "b", "c"]);
  assertEq(keys(applyOp(local, { order: ["c", "a"] }, fromCloud)), ["c", "a", "b"]);
  assertEq(applyOp(local, { replace: true }, fromCloud), null);
});

Deno.test("cover fingerprint", () => {
  assertEq(coverSig(null), "");
  assertEq(coverSig("data:image/jpeg;base64,AAAA") === coverSig("data:image/jpeg;base64,AAAB"), false);
  assertEq(uniqKeys([T("a"), T("a")]), ["a"]);
});
