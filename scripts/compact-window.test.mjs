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
const { CompactPanel, updateIndicator, fillColor, pairReset } = await import(
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

test("Codex paired windows use explicit slots and preserve unknown and additional rows", () => {
  const row = { ...panel.providers[0].rows[0], countdown: "1h35" };
  const week = { ...row, pairSlot: 1, label: "7 jours", remaining: 85, value: "85 %" };
  const render = (rows, compact = true) => html({panel: {...panel, providers:[{...panel.providers[0],compact,rows}]}});
  const paired = render([{...row,pairSlot:0},week,{...row,label:"Autre modèle",pairSlot:null}]);
  assert.equal((paired.match(/role="meter"/g)||[]).length,3);
  assert.match(paired,/compact-pair__track--short/);assert.match(paired,/compact-pair__track--week/);
  assert.match(paired,/1h35 · 85 %/);assert.match(paired,/Autre modèle/);
  const only = render([week]);
  assert.match(only,/grid-row:2/); // no fictitious meter, but retain the native weekly y slot
  assert.doesNotMatch(only,/compact-pair__track--short/);assert.match(only,/compact-pair__track--week/);
  assert.equal((only.match(/role="meter"/g)||[]).length,1);
  assert.doesNotMatch(render([week],false),/class="compact-pair"/);
  const missing = render([{...week,remaining:null,value:"Non communiquée"}]);
  assert.doesNotMatch(missing,/aria-valuenow/);assert.match(missing,/Non communiquée/);
  assert.doesNotMatch(render([]),/role="meter"/);
});

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

// --- Claude multi-account cards (same model as the native menu card) -------
const multiRow = (label, remaining, countdown, extra = {}) => ({
  model: false, label, remaining, value: remaining === null ? "—" : `${remaining} %`,
  reset: "Réinitialisation à 15h40", countdown, low: remaining !== null && remaining <= 20,
  fill: fill("green", "green", 0, (remaining ?? 0) / 100), ...extra,
});
const multiPanel = {
  ...panel,
  providers: [
    { name: "Claude · Personnel", section: "CLAUDE · PERSONNEL", compact: true, badge: "Pro", note: null, rows: [
      multiRow("Limite sur 3h30", 73, "3h30"),
      multiRow("Limite sur 3j 8h", 86, "3j 8h"),
      multiRow("Limite Fable", 60, "4j 4h", { model: true }),
    ] },
    { name: "Claude · Travail", section: "CLAUDE · TRAVAIL", compact: true, badge: "Max",
      note: { text: "Usage supplémentaire : plafond atteint", tone: "warning" }, rows: [
      multiRow("Limite sur 42 min", 0, "42 min", { value: "Épuisé", low: true, fill: fill("red", "red", 0, 0) }),
      multiRow("Limite sur 2j 1h", 20, "2j 1h", { reset: "Réinitialisation le 02/11 à 12h00" }),
      multiRow("Limite Fable", 86, "3j 8h", { model: true, reset: "Réinitialisation le 03/11 à 19h00" }),
    ] },
  ],
};

test("multi-account cards pair 5 h and 7 j with countdowns above and resets below, Fable stays its own row", () => {
  const markup = html({ panel: multiPanel });
  assert.equal((markup.match(/class="compact-pair"/g) ?? []).length, 2);
  assert.match(markup, /CLAUDE · PERSONNEL/);
  assert.match(markup, /CLAUDE · TRAVAIL/);
  // Countdown + provider value; the window name stays in the accessible label.
  assert.match(markup, />3h30 · 73 %</);
  assert.match(markup, />3j 8h · 86 %</);
  assert.match(markup, />42 min · Épuisé</);
  assert.match(markup, /aria-label="Limite 5 heures : Limite sur 3h30 · 73 % restant"/);
  // The pair is always 5 h + 7 j: Fable never takes the weekly slot, whatever it reports.
  assert.match(markup, />2j 1h · 20 %</);
  assert.doesNotMatch(markup, />Fable · /);
  // Reset of each window under its bar, independent of Fable (Personnel has one, Travail another).
  assert.equal((markup.match(/class="compact-pair__resets"/g) ?? []).length, 2);
  assert.match(markup, /compact-pair__resets"><span title="[^"]*">Réinit\. à 15h40<\/span><span title="[^"]*">Réinit\. à 15h40<\/span>/);
  assert.match(markup, />Réinit\. le 02\/11 à 12h00</);
  assert.equal(pairReset({ reset: "Réinitialisation non communiquée" }), "Réinit. non communiquée");
  // 5 h track = the Unique track class (same 5 px), 7 j finer.
  assert.equal((markup.match(/compact-row__track compact-pair__track--short/g) ?? []).length, 2);
  assert.equal((markup.match(/compact-row__track compact-pair__track--week/g) ?? []).length, 2);
  // A separate Fable row stays visible as a normal row (nothing dropped).
  assert.equal((markup.match(/Limite <span style="color:var\(--compact-teal\)">Fable<\/span>/g) ?? []).length, 2);
  assert.equal((markup.match(/class="compact-row"/g) ?? []).length, 2, "one Fable row per account");
  assert.match(markup, /Usage supplémentaire : plafond atteint/);
});

test("single-account and Codex cards keep the v3.1.0 rows (no pairing)", () => {
  const markup = html();
  assert.doesNotMatch(markup, /compact-pair/);
  assert.equal((markup.match(/class="compact-row"/g) ?? []).length, 3);
});

test("compact window thickness: 5 h inherits the 5 px Unique track, 7 j is 3 px with a 2 px gap", async () => {
  const sass = await import("sass");
  const css = sass.compile(new URL("../src/features/compact/compact.scss", import.meta.url).pathname).css;
  assert.match(css, /\.compact-row__track\s*\{[^}]*height: 5px/);
  assert.doesNotMatch(css, /\.compact-pair__track--short\s*\{[^}]*height:/);
  assert.match(css, /\.compact-pair__track--week\s*\{[^}]*height: 3px[^}]*border-radius: 1\.5px/);
  assert.match(css, /\.compact-pair__bars\s*\{[^}]*gap: 2px/);
  assert.match(css, /\.compact-pair\s*\{[^}]*height: 54px/);
  assert.match(css, /\.compact-pair__resets\s*\{[^}]*font-size: 10px/);
});

// --- Provider header ↗, account state, + (same logic as the Dashboard) ---------
const account = (kind, id, name, badge, state, tone = "good", rows = [multiRow("Limite sur 3h30", 73, "3h30"), multiRow("Limite sur 3j 8h", 86, "3j 8h")]) => ({
  kind, accountName: name, state: { label: state, tone },
  name: `${kind === "codex" ? "Codex" : "Claude"} · ${name}`, section: `${kind.toUpperCase()} · ${name.toUpperCase()}`, compact: true, badge, note: null, rows,
});
const groupedPanel = {
  ...panel,
  addable: ["codex", "claude"],
  providers: [
    account("codex", "primary", "Principal", "Plus", "Connecté"),
    account("codex", "profile:travail", "Travail", "Pro", "Non connecté", "warning", []),
    account("claude", "primary", "Personnel", "Pro", "Connecté"),
  ],
};
const calls = [];
const handlers = { onUsage: (kind) => calls.push(["usage", kind]), onAdd: (kind) => calls.push(["add", kind]) };
const groupedProps = (extra = {}) => props({ panel: groupedPanel, usageIcon: createElement("svg", { className: "external-icon" }), ...handlers, ...extra });

test("one header « CODEX ↗ » per provider before its account cards, then its +", () => {
  const markup = renderToStaticMarkup(createElement(CompactPanel, groupedProps()));
  const groups = markup.split('<section class="compact-group"').slice(1);
  assert.equal(groups.length, 2);
  assert.match(groups[0], /^[^>]*aria-label="Codex"[\s\S]*<h2 class="compact-group__head"><button[^>]*aria-label="Codex : usage détaillé"[^>]*>CODEX<svg class="external-icon"/);
  assert.match(groups[1], /<button[^>]*aria-label="Claude : usage détaillé"[^>]*>CLAUDE<svg class="external-icon"/);
  assert.equal((markup.match(/compact-group__usage/g) ?? []).length, 2, "once per provider, not per card");
  assert.equal((groups[0].match(/<section class="compact-provider"/g) ?? []).length, 2);
  assert.ok(groups[0].indexOf("compact-group__usage") < groups[0].indexOf('class="compact-provider"'));
  assert.ok(groups[0].lastIndexOf('class="compact-provider"') < groups[0].indexOf("Ajouter un compte Codex"));
  assert.match(groups[1], /aria-label="Ajouter un compte Claude"/);
  assert.doesNotMatch(groups[0], /Ajouter un compte Claude/);
});

test("each card: account name, plan beside it, then its state; no chevron, no detail", () => {
  const markup = renderToStaticMarkup(createElement(CompactPanel, groupedProps()));
  const cards = markup.split('<section class="compact-provider"').slice(1);
  assert.match(cards[0], /<span class="compact-provider__name">Principal<\/span><span class="compact-provider__badge">Plus<\/span><span class="compact-provider__state compact-tone--good">Connecté<\/span><\/div>/);
  assert.match(cards[1], /compact-tone--warning">Non connecté<\/span><\/div>/);
  assert.doesNotMatch(markup, /Détail|compact-provider__chevron|<path d="M6 3\.5 10\.5 8 6 12\.5"/);
});

test("header ↗ and + route to their own provider; no account detail handler exists", () => {
  calls.length = 0;
  const tree = CompactPanel(groupedProps());
  const buttons = find(tree, (n) => n.type === "button");
  const click = (label) => buttons.find((b) => b.props["aria-label"] === label).props.onClick();
  click("Codex : usage détaillé"); click("Claude : usage détaillé");
  click("Ajouter un compte Claude"); click("Ajouter un compte Codex");
  assert.deepEqual(calls, [["usage", "codex"], ["usage", "claude"], ["add", "claude"], ["add", "codex"]]);
  assert.equal(buttons.some((b) => /Détail/.test(b.props["aria-label"] ?? "")), false);
});

test("an enabled provider without a visible card still offers its header and its +; a disabled one has none", () => {
  const onlyClaude = { ...groupedPanel, addable: ["codex", "claude"], providers: groupedPanel.providers.filter((p) => p.kind === "claude") };
  const markup = renderToStaticMarkup(createElement(CompactPanel, groupedProps({ panel: onlyClaude })));
  assert.match(markup, /aria-label="Codex : usage détaillé"/);
  assert.match(markup, /aria-label="Ajouter un compte Codex"/);
  const off = { ...onlyClaude, addable: ["claude"] };
  const without = renderToStaticMarkup(createElement(CompactPanel, groupedProps({ panel: off })));
  assert.doesNotMatch(without, /Codex/);
});

test("models without provider information (older fixtures) render as before, without header or +", () => {
  const markup = html();
  assert.doesNotMatch(markup, /compact-group|compact-provider__state|Ajouter un compte|usage détaillé/);
});
