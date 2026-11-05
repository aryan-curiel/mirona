// Runnable check for the per-line note times (NGR-144): `node scripts/notes-check.mjs` (Node 23.6+).
import { retime } from "../src/lib/notes.ts";
import assert from "node:assert";
assert.deepEqual(retime([""], [], ["a"], 5), [5]);
assert.deepEqual(retime(["a"], [5], ["ab"], 9), [5], "edited line keeps its time");
assert.deepEqual(retime(["a"], [5], ["a", ""], 9), [5, null]);
assert.deepEqual(retime(["a", ""], [5, null], ["a", "b"], 9), [5, 9]);
assert.deepEqual(retime(["a", "c"], [5, 7], ["a", "b", "c"], 9), [5, 9, 7], "insert in the middle");
assert.deepEqual(retime(["a", "b", "c"], [5, 6, 7], ["a", "c"], 9), [5, 7], "delete a line");
assert.deepEqual(retime(["a"], [null], ["ab"], null), [null], "outside a recording");
console.log("retime ok");
