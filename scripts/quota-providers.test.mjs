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

test("Fable alone is smaller and bar-coloured, without a dot or dimming", () => {
  const input = snapshot([metric("claude:five_hour", 20), metric("claude:weekly_fable", 50)]);
  const render = (enabled, compact = false, data = input) => renderToStaticMarkup(createElement(ProviderCard, {
    name: "Claude", rows: claudeRows(data, enabled), compact,
  }));
  const html = render(true);
  assert.doesNotMatch(html, /opacity/);
  const rows = html.split('<div class="provider-row').slice(1);
  assert.equal(rows.length, 2);
  assert.match(rows[0], /^ provider-row--healthy"><div class="dashboard__quota-heading"><span>Limite 5 heures<\/span>/);
  assert.doesNotMatch(rows[0], /model/);
  assert.match(rows[1], /^ provider-row--model provider-row--healthy"><div class="dashboard__quota-heading"><span>Limite hebdomadaire <span class="provider-row__model-label">Fable<\/span><\/span><strong>/);
  // Accessible name and state classes are unchanged: the tint only overrides the healthy fill in CSS.
  assert.match(rows[1], /class="dashboard__progress dashboard__progress--healthy" role="progressbar" aria-label="Limite hebdomadaire Fable"/);
  assert.match(rows[1], /provider-row__reset/);
  assert.equal((html.match(/provider-row__model-label/g) || []).length, 1);
  assert.doesNotMatch(html, /model-dot|<i|•/);
  const low = render(true, false, snapshot([metric("claude:weekly_fable", 90)]));
  assert.match(low, /provider-row--model.*dashboard__progress--low/);
  assert.doesNotMatch(render(false), /Fable|model/);
  assert.doesNotMatch(render(true, true), /Fable|model/); // Secondary rows remain in details.
  const fableOnly = render(true, true, snapshot([metric("claude:weekly_fable", 30)]));
  assert.match(fableOnly, /<div class="provider-row provider-row--model provider-row--healthy">/);
  for (const [used, state] of [[0, "healthy"], [70, "warning"], [90, "low"], [100, "low"], [null, "low"]]) {
    const output = render(true, false, snapshot([metric("claude:weekly_fable", used)]));
    assert.match(output, new RegExp(`provider-row--model provider-row--${state}`));
    assert.match(output, new RegExp(`dashboard__progress--${state}`));
    assert.match(output, /<span class="provider-row__model-label">Fable<\/span>/);
  }
  assert.doesNotMatch(html, /30h/);
});

test("model label and bar share the state token in both themes", () => {
  const css = fs.readFileSync(new URL("../src/pages/Dashboard/dashboard.scss", import.meta.url), "utf8");
  assert.match(css, /\.provider-row--model\.provider-row--healthy \{ --quota-fill: var\(--model-accent\); \}/);
  assert.match(css, /background: var\(--quota-fill, #30a46c\)/);
  assert.match(css, /\.provider-row__model-label \{ font-size: 11px; font-weight: 400; color: var\(--quota-fill\); \}/);
  for (const [state, colour] of [["warning", "#d68b12"], ["low", "#e5484d"]]) {
    assert.ok(css.includes(`.provider-row--${state} { --quota-fill: ${colour}; }`));
    assert.ok(css.includes(`&__progress--${state} { --quota-fill: ${colour}; }`));
  }
  assert.doesNotMatch(css, /model-dot/);
  assert.equal((css.match(/--model-accent: #[0-9a-f]{6};/g) || []).length, 2);
  assert.doesNotMatch(css, /provider-row[^{]*\{[^}]*opacity/);
});

test("Settings keeps the Fable switch label but no help text", () => {
  const settings = fs.readFileSync(new URL("../src/features/settings/Settings.tsx", import.meta.url), "utf8");
  assert.match(settings, /<Switch label="Afficher le quota Fable et ses alertes"/);
  assert.match(settings, /<h3>Quota Fable<\/h3>/);
  assert.doesNotMatch(settings, /si Claude Code le communique|Ne modifie pas votre forfait/);
});
