import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import { createHash } from "node:crypto";

const root = new URL("../", import.meta.url);
const read = (path) => fs.readFileSync(new URL(path, root));
const dimensions = (bytes) => {
  assert.equal(bytes.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
  return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];
};

test("the approved bitmap is unchanged and all configured bundle icons exist", () => {
  const source = read("assets/app-icon.png");
  assert.equal(
    createHash("sha256").update(source).digest("hex"),
    "8ab76e2f3e93ff38b14b0ebdc9906b7fb11382cf2b418e2389487776f3d74309",
  );
  assert.deepEqual(dimensions(source), [1024, 1024]);
  const config = JSON.parse(read("src-tauri/tauri.conf.json"));
  for (const icon of config.bundle.icon)
    assert.ok(read("src-tauri/" + icon).length > 0, icon);
  for (const [file, size] of [
    ["32x32.png", 32],
    ["64x64.png", 64],
    ["128x128.png", 128],
    ["128x128@2x.png", 256],
    ["icon.png", 512],
  ]) {
    assert.deepEqual(
      dimensions(read("src-tauri/icons/" + file)),
      [size, size],
      file,
    );
  }
});

test("ICNS and ICO containers have valid sizes and bounded image entries", () => {
  const icns = read("src-tauri/icons/icon.icns");
  assert.equal(icns.toString("ascii", 0, 4), "icns");
  assert.equal(icns.readUInt32BE(4), icns.length);
  let offset = 8;
  let entries = 0;
  while (offset < icns.length) {
    const length = icns.readUInt32BE(offset + 4);
    assert.ok(length > 8 && offset + length <= icns.length);
    offset += length;
    entries++;
  }
  assert.equal(offset, icns.length);
  assert.ok(entries >= 5);
  const ico = read("src-tauri/icons/icon.ico");
  assert.equal(ico.readUInt16LE(0), 0);
  assert.equal(ico.readUInt16LE(2), 1);
  const count = ico.readUInt16LE(4);
  assert.ok(count >= 3);
  for (let index = 0; index < count; index++) {
    const entry = 6 + index * 16;
    const length = ico.readUInt32LE(entry + 8);
    const start = ico.readUInt32LE(entry + 12);
    assert.ok(
      length > 0 && start >= 6 + count * 16 && start + length <= ico.length,
    );
  }
});
