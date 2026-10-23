import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import ts from "typescript";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { claudeRows, formatResetDate } from "../src/features/quota/providers.ts";

// Synthetic snapshot fixtures matching the backend's normal metric contract.
const metric = (id, usedPercent, resetAt = null) => ({
  id, label: id === "claude:weekly_fable" ? "Limite hebdomadaire Fable" : "Limite 5 heures",
  usedPercent, resetAt, availability: "available",
  periodDurationSeconds: id === "claude:weekly_fable" ? 604800 : 18000,
  exhausted: usedPercent >= 100,
});
const snapshot = (metrics) => ({ metrics });

test("optional Fable uses normal remaining-percentage and reset rendering", () => {
  const reset = "2026-10-30T14:00:00Z";
  const rows = claudeRows(snapshot([
    metric("claude:five_hour", 23.5), metric("claude:weekly_fable", 37.5, reset),
  ]));
  assert.equal(rows.length, 2);
  assert.equal(rows[0].remaining, 77);
  assert.deepEqual(rows[1], {
    id: "claude:weekly_fable", label: "Limite hebdomadaire Fable", modelLabel: "Fable",
    remaining: 63, exhausted: false, missing: "Non communiquée",
    resetText: formatResetDate(reset),
  });
});

test("old snapshots do not fabricate Fable; expired windows remain unavailable", () => {
  assert.equal(claudeRows(snapshot([metric("claude:five_hour", 0)])).length, 1);
  const expired = { ...metric("claude:weekly_fable", null), availability: "unavailable" };
  const [row] = claudeRows(snapshot([expired]));
  assert.equal(row.remaining, null);
  assert.equal(row.exhausted, false);
  assert.equal(row.missing, "Réinitialisée · en attente");
  assert.equal(row.resetText, "non communiquée");
});

test("rounded zero remaining is not actual Fable exhaustion", () => {
  const [rounded, exhausted] = claudeRows(snapshot([
    metric("claude:weekly_fable", 99.6), metric("claude:weekly_fable", 100),
  ]));
  assert.equal(rounded.remaining, 0);
  assert.equal(rounded.exhausted, false);
  assert.equal(exhausted.remaining, 0);
  assert.equal(exhausted.exhausted, true);
});

test("Fable preference hides only Fable, can restore it, and never fabricates missing data", () => {
  const input = snapshot([metric("claude:five_hour", 20), metric("claude:weekly_fable", 30)]);
  assert.deepEqual(claudeRows(input, false), claudeRows(snapshot([input.metrics[0]])));
  assert.deepEqual(claudeRows(input, true), claudeRows(input));
  for (const enabled of [true, false]) {
    assert.deepEqual(claudeRows(snapshot([]), enabled), []);
    assert.equal(claudeRows(snapshot([metric("claude:five_hour", 10)]), enabled).length, 1);
  }
  const [unknown] = claudeRows(snapshot([metric("claude:weekly_fable", null)]));
  assert.equal(unknown.remaining, null);
});

// Exercise the actual React renderer without adding a test runtime dependency.
const source = fs.readFileSync(new URL("../src/features/quota/ProviderCard.tsx", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext } }).outputText;
const moduleSource = compiled.replaceAll('"react/jsx-runtime"', JSON.stringify(import.meta.resolve("react/jsx-runtime")))
  .replaceAll('"react"', JSON.stringify(import.meta.resolve("react")));
const { default: ProviderCard } = await import("data:text/javascript;base64," + Buffer.from(moduleSource).toString("base64"));

test("only the Fable word receives 0.8 opacity in real row markup", () => {
  const input = snapshot([metric("claude:five_hour", 20), metric("claude:weekly_fable", 30)]);
  const render = (enabled, compact = false) => renderToStaticMarkup(createElement(ProviderCard, {
    name: "Claude", rows: claudeRows(input, enabled), compact,
  }));
  const html = render(true);
  assert.match(html, /Limite hebdomadaire <span class="provider-row__model" style="opacity:0.8">Fable<\/span>/);
  assert.equal((html.match(/opacity:/g) || []).length, 1);
  assert.match(html, /aria-label="Limite hebdomadaire Fable"/);
  assert.doesNotMatch(render(false), /Fable|opacity:/);
  assert.doesNotMatch(render(true, true), /provider-row__model/); // Secondary rows remain in details.
  assert.doesNotMatch(html, /30h/);
});
