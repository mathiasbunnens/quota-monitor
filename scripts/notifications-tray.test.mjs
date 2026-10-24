import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { inflateSync } from "node:zlib";
import ts from "typescript";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const asset = new URL("../src-tauri/icons/mascot-menu.png", import.meta.url);
test("tray generator is deterministic and does not touch the app artwork", () => {
  const before = readFileSync(asset);
  const app = readFileSync(new URL("../assets/app-icon.png", import.meta.url));
  execFileSync(process.execPath, ["scripts/render-tray-icon.mjs"], { cwd: new URL("..", import.meta.url) });
  assert.deepEqual(readFileSync(asset), before);
  assert.deepEqual(readFileSync(new URL("../assets/app-icon.png", import.meta.url)), app);
});

test("runtime template has transparent corners, antenna, simple eye and separated feet", () => {
  const png = readFileSync(asset);
  assert.equal(png.readUInt32BE(16), 36);
  assert.equal(png.readUInt32BE(20), 36);
  assert.equal(png[25], 6); // RGBA, no opaque mint rectangle.
  const idat = [];
  for (let offset = 8; offset < png.length;) {
    const length = png.readUInt32BE(offset);
    if (png.toString("ascii", offset + 4, offset + 8) === "IDAT") idat.push(png.subarray(offset + 8, offset + 8 + length));
    offset += length + 12;
  }
  const rows = inflateSync(Buffer.concat(idat));
  const rgba = [];
  for (let y = 0; y < 36; y++) {
    assert.equal(rows[y * 145], 0);
    for (let x = 0; x < 36; x++) rgba.push([...rows.subarray(y * 145 + 1 + x * 4, y * 145 + 5 + x * 4)]);
  }
  assert(rgba.every(p => p[0] === 0 && p[1] === 0 && p[2] === 0));
  assert(rgba.some(p => p[3] === 255));
  const alpha = (x, y) => rgba[Math.floor(y * 36 / 44) * 36 + Math.floor(x * 36 / 44)][3];
  assert.equal(alpha(0, 0), 0);
  assert(alpha(22, 4) > 240, "antenna");
  assert(alpha(12, 30) > 240, "body");
  assert(alpha(23.5, 24.5) > 240, "lower-right pupil quadrant");
  assert.equal(alpha(20.5, 21.5), 0, "transparent upper-left quarter, matching app gauge");
  assert(alpha(23.5, 21.5) > 240, "upper-right pupil quadrant");
  assert(alpha(20.5, 24.5) > 240, "lower-left pupil quadrant");
  assert.equal(alpha(17.7, 19.8), 0, "eye counterform");
  assert(alpha(24, 23) > 240, "broad three-quarter pupil");
  for (const [x, y] of [[22, 17.5], [27.5, 23], [22, 28.5], [16.5, 23]]) {
    assert.equal(alpha(x, y), 0, "uninterrupted counterform, no gauge arc");
  }
  const svg = readFileSync(new URL("../src-tauri/icons/mascot-menu.svg", import.meta.url), "utf8");
  assert.match(svg, /<path d="M22 23V19.5A3.5 3.5 0 1 1 18.5 23Z"\/>/);
  assert.doesNotMatch(svg, /stroke-width|stroke-linecap/);
  assert.equal(alpha(22, 33.7), 0, "mouth");
  assert.equal(alpha(22, 41), 0, "foot gap");
  assert(alpha(16, 41) > 240, "left foot");
});

test("both effective macOS menu paths use this template; desktop tray stays numeric", () => {
  const rust = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(rust, /include_bytes!\("\.\.\/icons\/mascot-menu.png"\)/);
  assert.match(rust, /icon.setSize\(NSSize::new\(18.0, 18.0\)\);\s*icon.setTemplate\(true\)/);
  assert.match(rust, /button.setImage\(mascot_image\(mtm\).as_deref\(\)\)/);
  assert.match(rust, /let icon = mascot_image\(mtm\)/);
  assert.match(rust, /\.icon\(tray_percentage::icon\(None\)\)/);
});

const source = readFileSync(new URL("../src/features/settings/NtfySettings.tsx", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext } }).outputText;
const moduleSource = compiled.replace(/from "([^"]+)"/g, (_, specifier) => `from ${JSON.stringify(import.meta.resolve(specifier))}`);
const { default: NtfySettings } = await import("data:text/javascript;base64," + Buffer.from(moduleSource).toString("base64"));
test("compact notifications preserve labelled controls, visible safety and discoverable help", () => {
  const html = renderToStaticMarkup(createElement(NtfySettings, {
    config: { enabled: false, serverUrl: "https://fixture.invalid", topic: "" }, onSave: async () => true,
  }));
  assert.match(html, /0 % restant/);
  assert.match(html, /doit rester ouverte et connectée/);
  assert.match(html, /aria-describedby="ntfy-privacy"/);
  assert.match(html, /Topic public/);
  assert.match(html, /<details class="prefs-disclosure"><summary>/);
  assert.doesNotMatch(html, /<details[^>]* open/);
  assert.match(html, /réglages du téléphone/);
  assert.match(html, /sans les enregistrer/);
  assert.match(html, /aucun nouvel essai automatique/);
  assert.match(html, /<label>Serveur ntfy<input type="url"/);
  assert.match(html, /<label>Topic<input type="text"/);
  assert.match(html, /maxLength="128"/);
  assert.match(html, />Enregistrer<\/button>/);
  assert.match(html, /disabled="">Envoyer un test/);
});
