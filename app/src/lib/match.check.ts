// Self-check for match.ts: node --experimental-strip-types app/src/lib/match.check.ts
import assert from "node:assert/strict";
import type { Segment } from "../bindings/Segment";
import { rank, samples } from "./match.ts";

const tags = ["project:setr", "project:atlas", "team:design", "setup", "hiring"];
assert.equal(rank("setr", tags)[0], "project:setr"); // prefix after ':'
assert.deepEqual(rank("set", tags).slice(0, 2), ["setup", "project:setr"]);
assert.deepEqual(rank("pset", tags), ["project:setr"]); // letters in order
assert.deepEqual(rank("SIGN", tags), ["team:design"]); // any case, substring
assert.deepEqual(rank("zzz", tags), []);
assert.equal(rank("", tags, 2).length, 2);

const seg = (id: string, startMs: number, endMs: number, sensitive = false): Segment => ({
  id,
  speakerId: "s1",
  startMs,
  endMs,
  text: id,
  redacted: sensitive,
  tags: [],
  answers: [],
  owners: [],
  due: null,
  byHand: false,
});
const segs = [
  seg("a", 0, 6_000),
  seg("b", 10_000, 15_000), // within 60 s of "a"
  seg("c", 100_000, 104_000),
  seg("d", 200_000, 209_000, true), // longest, but sensitive
  seg("e", 300_000, 303_000),
  seg("f", 400_000, 400_500), // too short
];
const picked = samples(segs, "s1", false).map((s) => s.id);
assert.deepEqual(picked, ["a", "c", "e"]); // ≤ 3, spread out, by start
assert.ok(samples(segs, "s1", true).some((s) => s.id === "d"));
assert.deepEqual(samples(segs, "other", false), []);

console.log("match ok");
