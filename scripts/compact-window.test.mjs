import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import ts from "typescript";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

// Dependency-free: transpile the pure compact view; no DOM, IPC or network.
const source = fs.readFileSync(new URL("../src/features/compact/CompactPanel.tsx", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext },
}).outputText;
const moduleSource = compiled.replace(/from "([^"]+)"/g,
  (_, specifier) => `from ${JSON.stringify(import.meta.resolve(specifier))}`);
const { CompactPanel, updateIndicator, fillColor } = await import(
  "data:text/javascript;base64," + Buffer.from(moduleSource).toString("base64"));

const fill = (from, to, fraction, ratio) => ({ from, to, fraction, visible: ratio * 276 >= 5, ratio });
const panel = {
  header: { title: "Quota Monitor", subtitle: "Actualisé à 14h05", status: "À jour", tone: "good" },
  providers: [
    { name: "Codex", section: "CODEX", badge: "Plus", note: null, rows: [
      { model: false, label: "Limite sur 1h35", remaining: 63, value: "63 %", reset: "Réinitialisation à 15h40", low: false, fill: fill("green", "green", 0, 0.63) },
      { model: false, label: "Réserve", remaining: 0, value: "Épuisé", reset: "Réinitialisation le 29/10 à 09h12", low: true, fill: fill("red", "red", 0, 0) },
    ] },
    { name: "Claude", section: "CLAUDE", badge: null, note: { text: "Données anciennes, nouvelle lecture en cours.", tone: "warning" }, rows: [
      { model: true, label: "Limite Fable", remaining: 50, value: "50 %", reset: "Réinitialisation à 15h40", low: false, fill: fill("orange", "green", 0.5, 0.5) },
    ] },
  ],
  connection: null,
  version: "test",
  update: null,
};
const props = (overrides = {}) => ({
  panel, error: null, pending: null,
  onSettings: () => {}, onUpdate: () => {}, onConnect: () => {},
  ...overrides,
});
const html = (overrides) => renderToStaticMarkup(createElement(CompactPanel, props(overrides)));

/** Walks the element tree returned by the component function (no DOM). */
function find(node, predicate, out = []) {
  if (Array.isArray(node)) { node.forEach((child) => find(child, predicate, out)); return out; }
  if (!node || typeof node !== "object") return out;
  if (predicate(node)) out.push(node);
  find(node.props?.children, predicate, out);
  return out;
}

test("header carries exactly two icon buttons, settings then update, after the status", () => {
  const markup = html();
  const header = markup.slice(markup.indexOf("<header"), markup.indexOf("</header>"));
  const buttons = [...header.matchAll(/<button[^>]*aria-label="([^"]+)"[^>]*>(.*?)<\/button>/g)];
  assert.deepEqual(buttons.map((b) => b[1]), ["Réglages", "Vérifier les mises à jour"]);
  for (const [, , inner] of buttons) {
    assert.match(inner, /^<svg[^>]*aria-hidden="true"/);
    assert.equal(inner.replace(/<[^>]+>/g, "").trim(), "");
  }
  assert.ok(header.indexOf("compact__capsule") < header.indexOf("compact__actions"));
  assert.match(header, /title="Réglages… \(⌘,\)"/);
  assert.match(header, /aria-keyshortcuts="Meta\+Comma"/);
  assert.match(header, /title="Vérifier les mises à jour…"/);
  assert.doesNotMatch(markup.replace(/<[^>]+>/g, " "), /Réglages…|Vérifier les mises à jour/);
});

test("rows keep the menu panel order, labels, values, resets and gauges", () => {
  const markup = html();
  const text = markup.replace(/<[^>]+>/g, "|");
  let cursor = 0;
  for (const provider of panel.providers) {
    for (const piece of [provider.section, ...provider.rows.flatMap((r) => [r.label === "Limite Fable" ? "Fable" : r.label, r.value, r.reset])]) {
      const at = text.indexOf(piece, cursor);
      assert.ok(at >= cursor, `${piece} in order`);
      cursor = at;
    }
  }
  assert.match(markup, /class="compact-row__value is-low">Épuisé/);
  assert.match(markup, /role="meter"[^>]*aria-valuenow="63"[^>]*aria-valuetext="63 % restant"/);
  assert.equal((markup.match(/compact-row__fill/g) ?? []).length, 2, "0 % shows the track only");
  assert.match(markup, /width:63%/);
  assert.match(markup, /compact-card__note compact-tone--warning has-rows/);
  assert.match(markup, /Plus/);
  assert.match(markup, /Quota Monitor v<!-- -->test|Quota Monitor vtest/);
});

test("gauge colours reuse the backend blend with teal for model rows", () => {
  assert.equal(fillColor(fill("green", "green", 0, 1), false), "var(--compact-green)");
  assert.equal(fillColor(fill("green", "green", 0, 1), true), "var(--compact-teal)");
  assert.equal(fillColor(fill("red", "orange", 0.25, 0.25), false),
    "color-mix(in srgb, var(--compact-red) 75%, var(--compact-orange) 25%)");
  assert.equal(fillColor(fill("orange", "green", 0.5, 0.5), true),
    "color-mix(in srgb, var(--compact-orange) 50%, var(--compact-teal) 50%)");
  // "Fable" alone takes the fill colour, like style_model_label.
  assert.match(html(), /Limite <span style="color:color-mix\(in srgb, var\(--compact-orange\) 50%, var\(--compact-teal\) 50%\)">Fable<\/span>/);
});

test("update states are exposed as busy, available, ready and error", () => {
  const status = (phase, extra = {}) => ({ phase, message: "", progress: null, currentVersion: "1", nextVersion: "2", ...extra });
  assert.equal(updateIndicator(null).state, "idle");
  assert.equal(updateIndicator(status("up-to-date")).state, "idle");
  assert.equal(updateIndicator(status("checking")).state, "busy");
  assert.equal(updateIndicator(status("downloading", { progress: 142 })).description,
    "Téléchargement de Quota Monitor v2 en cours (100 %)…");
  assert.equal(updateIndicator(status("downloading", { progress: NaN })).description,
    "Téléchargement de Quota Monitor v2 en cours…");
  assert.equal(updateIndicator(status("available")).state, "available");
  assert.equal(updateIndicator(status("ready")).state, "ready");
  assert.equal(updateIndicator(status("error")).state, "error");

  const busy = html({ panel: { ...panel, update: status("checking") } });
  assert.match(busy, /class="compact-icon compact-icon--update is-busy"[^>]*aria-busy="true"/);
  assert.match(busy, /aria-live="polite">Recherche de mises à jour en cours…</);
  const available = html({ panel: { ...panel, update: status("available") } });
  assert.match(available, /is-available/);
  assert.match(available, /compact-icon__badge/);
  assert.match(available, /title="Vérifier les mises à jour… — Mise à jour disponible : Quota Monitor v2\."/);
  const failed = html({ panel: { ...panel, update: status("error") } });
  assert.match(failed, /is-error/);
  assert.doesNotMatch(html(), /compact-icon__badge/);
});

test("the two icons call their own handler; pending actions are marked", () => {
  const calls = [];
  const tree = CompactPanel(props({
    onSettings: () => calls.push("settings"),
    onUpdate: () => calls.push("update"),
    onConnect: () => calls.push("connect"),
    panel: { ...panel, connection: { label: "Se connecter à Codex…", loggingIn: false } },
  }));
  const buttons = find(tree, (node) => node.type === "button");
  const byName = Object.fromEntries(buttons.map((b) => [b.props["aria-label"] ?? b.props.children, b]));
  byName["Réglages"].props.onClick();
  byName["Vérifier les mises à jour"].props.onClick();
  byName["Se connecter à Codex…"].props.onClick();
  assert.deepEqual(calls, ["settings", "update", "connect"]);

  const pending = html({ pending: "update" });
  assert.match(pending, /aria-label="Vérifier les mises à jour"[^>]*aria-busy="true"[^>]*aria-disabled="true"/);
  const logging = html({ panel: { ...panel, connection: { label: "Connexion en cours…", loggingIn: true } } });
  assert.match(logging, /<button type="button" disabled="">Connexion en cours…<\/button>/);
});

test("errors are announced and the actions stay reachable without data", () => {
  const markup = html({ panel: null, error: "Offline fixture failure" });
  assert.match(markup, /role="alert">Offline fixture failure</);
  assert.match(markup, /aria-label="Réglages"/);
  assert.match(markup, /aria-label="Vérifier les mises à jour"/);
});
