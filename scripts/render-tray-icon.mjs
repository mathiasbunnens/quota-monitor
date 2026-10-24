import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

// 18pt at 2x, matching mascot_image() in lib.rs. The app/Dock PNG is
// deliberately separate: this is an optical, transparent template glyph.
const size = 36;
const samplesPerAxis = 4;

function circle(x, y, centerX, centerY, radius) {
  return (x - centerX) ** 2 + (y - centerY) ** 2 <= radius ** 2;
}

function roundedRect(x, y, left, top, width, height, radius) {
  const nearestX = Math.max(left + radius, Math.min(x, left + width - radius));
  const nearestY = Math.max(top + radius, Math.min(y, top + height - radius));
  return (x - nearestX) ** 2 + (y - nearestY) ** 2 <= radius ** 2;
}

function mascotContains(x, y) {
  // Reconstructed from assets/app-icon.png, not the retired two-eyed robot.
  // Work in a 44-unit space so the SVG and sampled bitmap share geometry.
  const antenna = circle(x, y, 22, 4, 2.5) || roundedRect(x, y, 21, 5, 2, 9, 1);
  const dome = circle(x, y, 22, 23, 12) && y <= 23;
  const body = roundedRect(x, y, 10, 21, 24, 18, 1.5);
  const feet = roundedRect(x, y, 13, 36, 7, 6, 1.5) || roundedRect(x, y, 24, 36, 7, 6, 1.5);
  const eye = circle(x, y, 22, 23, 7.5);
  // A solid three-quarter pupil, not a thin gauge arc. The transparent
  // upper-left quarter follows the app mascot and adapts to either theme.
  const pupil = circle(x, y, 22, 23, 3.5) && !(x < 22 && y < 23);
  const mouth = roundedRect(x, y, 16, 33, 12, 1.5, 0.75)
    || roundedRect(x, y, 17.5, 36, 9, 1.5, 0.75);
  return antenna || ((dome || body || feet) && !mouth && (!eye || pupil));
}

const pixels = Buffer.alloc(size * size * 4);
for (let pixelY = 0; pixelY < size; pixelY += 1) {
  for (let pixelX = 0; pixelX < size; pixelX += 1) {
    let coveredSamples = 0;
    for (let sampleY = 0; sampleY < samplesPerAxis; sampleY += 1) {
      for (let sampleX = 0; sampleX < samplesPerAxis; sampleX += 1) {
        const x = (pixelX + (sampleX + 0.5) / samplesPerAxis) * 44 / size;
        const y = (pixelY + (sampleY + 0.5) / samplesPerAxis) * 44 / size;
        if (mascotContains(x, y)) coveredSamples += 1;
      }
    }

    const offset = (pixelY * size + pixelX) * 4;
    pixels[offset] = 0;
    pixels[offset + 1] = 0;
    pixels[offset + 2] = 0;
    pixels[offset + 3] = Math.round((coveredSamples / samplesPerAxis ** 2) * 255);
  }
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const typeBuffer = Buffer.from(type);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])));
  return Buffer.concat([length, typeBuffer, data, checksum]);
}

const header = Buffer.alloc(13);
header.writeUInt32BE(size, 0);
header.writeUInt32BE(size, 4);
header[8] = 8;
header[9] = 6;

const scanlines = Buffer.alloc((size * 4 + 1) * size);
for (let row = 0; row < size; row += 1) {
  const rowOffset = row * (size * 4 + 1);
  scanlines[rowOffset] = 0;
  pixels.copy(scanlines, rowOffset + 1, row * size * 4, (row + 1) * size * 4);
}

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", header),
  chunk("IDAT", deflateSync(scanlines)),
  chunk("IEND", Buffer.alloc(0)),
]);

writeFileSync(resolve("src-tauri/icons/mascot-menu.png"), png);
