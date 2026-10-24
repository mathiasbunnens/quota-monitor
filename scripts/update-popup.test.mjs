import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import ts from "typescript";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const source = fs.readFileSync(
  new URL("../src/features/update/UpdatePopup.tsx", import.meta.url),
  "utf8",
);
const compiled = ts.transpileModule(
  source.replace('import "./update.scss";', ""),
  {
    compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext },
  },
).outputText;
const moduleSource = compiled.replace(
  /from "([^"]+)"/g,
  (_, specifier) => `from ${JSON.stringify(import.meta.resolve(specifier))}`,
);
const { UpdateContent, displayProgress } = await import(
  "data:text/javascript;base64," + Buffer.from(moduleSource).toString("base64")
);
const render = (phase, progress, props = {}) =>
  renderToStaticMarkup(
    createElement(UpdateContent, {
      status: {
        phase,
        progress,
        currentVersion: "test-current",
        nextVersion: "test-next",
        message: "Offline fixture",
      },
      busy: false,
      error: null,
      onCommand: async () => {},
      ...props,
    }),
  );

test("display values are bounded; unknown and invalid values remain indeterminate", () => {
  for (const value of [null, undefined, NaN, Infinity])
    assert.equal(displayProgress(value), undefined);
  for (const [value, expected] of [
    [-4, 0],
    [0, 0],
    [42.9, 42],
    [100, 100],
    [120, 100],
  ])
    assert.equal(displayProgress(value), expected);
});

test("0, partial and completed progress have a separate aligned heading and named native progress", () => {
  for (const value of [0, 42, 100]) {
    const html = render(value === 100 ? "ready" : "downloading", value);
    assert.match(html, new RegExp(`>${value} %</span>`));
    assert.match(
      html,
      new RegExp(
        `<progress max="100" value="${value}" aria-labelledby="update-progress-label"`,
      ),
    );
    assert.ok(
      html.indexOf("update-popup__percent") < html.indexOf("<progress"),
    );
  }
});

test("unknown total and installation never masquerade as zero or complete", () => {
  for (const [phase, value] of [
    ["downloading", null],
    ["installing", 100],
  ]) {
    const html = render(phase, value);
    assert.match(html, /En cours…/);
    assert.match(html, /<progress max="100" aria-labelledby=/);
    assert.doesNotMatch(html, /value="|100 %|0 %/);
  }
});

test("actions respect actual phases and IPC failures are exposed", () => {
  for (const phase of [
    "checking",
    "available",
    "downloading",
    "installing",
    "ready",
    "error",
    "up-to-date",
  ]) {
    const html = render(phase, null);
    assert.match(html, />Fermer</);
    assert.equal(
      html.includes("Télécharger et installer"),
      phase === "available",
    );
    assert.equal(html.includes("Quitter et relancer"), phase === "ready");
    assert.equal(html.includes("Réessayer"), phase === "error");
    assert.equal(
      html.includes("<progress"),
      ["downloading", "installing", "ready"].includes(phase),
    );
  }
  assert.match(render("available", null, { busy: true }), /disabled=""/);
  assert.match(
    render("error", null, { error: "IPC unavailable" }),
    /role="alert">IPC unavailable/,
  );
});
