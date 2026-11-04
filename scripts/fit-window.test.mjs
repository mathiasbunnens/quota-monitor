import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import ts from "typescript";

// Pure scheduler: transpiled alone, driven by a controlled clock (no DOM, no IPC).
const source = fs.readFileSync(new URL("../src/features/compact/fitWindow.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { createFitScheduler, FIT_DEBOUNCE_MS } = await import("data:text/javascript;base64," + Buffer.from(compiled).toString("base64"));

function harness() {
  let now = 0, next = 1;
  const timers = new Map();
  const requests = [];
  const scheduler = createFitScheduler({
    request: (value) => requests.push(value),
    setTimer: (callback, ms) => { const id = next++; timers.set(id, { at: now + ms, callback }); return id; },
    clearTimer: (id) => timers.delete(id),
  });
  const advance = (ms) => {
    now += ms;
    for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.callback(); }
  };
  return { scheduler, requests, advance };
}

test("a content change asks for one resize after the debounce", () => {
  const { scheduler, requests, advance } = harness();
  scheduler.measure(300.2, 520);
  advance(FIT_DEBOUNCE_MS - 1);
  assert.equal(requests.length, 0);
  advance(1);
  assert.deepEqual(requests, [{ height: 301, viewportHeight: 520 }]);
});

test("a burst of measurements collapses to the last one", () => {
  const { scheduler, requests, advance } = harness();
  for (const h of [280, 340, 410]) { scheduler.measure(h, 520); advance(10); }
  advance(FIT_DEBOUNCE_MS);
  assert.deepEqual(requests.map((r) => r.height), [410]);
});

test("our own window resize (same content, new viewport) never loops", () => {
  const { scheduler, requests, advance } = harness();
  scheduler.measure(410, 520); advance(FIT_DEBOUNCE_MS);
  scheduler.measure(410, 410); advance(1000);
  scheduler.measure(410, 410); advance(1000);
  assert.equal(requests.length, 1);
});

test("Compacte → Classique → Compacte requests each height once", () => {
  const { scheduler, requests, advance } = harness();
  for (const h of [260, 390, 260]) { scheduler.measure(h, 500); advance(FIT_DEBOUNCE_MS); }
  assert.deepEqual(requests.map((r) => r.height), [260, 390, 260]);
});

test("invalid measures are ignored and dispose cancels the pending request", () => {
  const { scheduler, requests, advance } = harness();
  for (const bad of [0, -5, NaN, Infinity]) scheduler.measure(bad, 500);
  advance(1000);
  assert.equal(requests.length, 0);
  scheduler.measure(200, 500); scheduler.dispose(); advance(1000);
  assert.equal(requests.length, 0);
});
