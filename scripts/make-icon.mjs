// Generates a branded 1024x1024 source PNG (no external deps) for `tauri icon`.
// A violet→blue rounded-square with a white "sparkle" mark, matching the app brand.
import { writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";

const S = 1024;
const R = 210; // corner radius
const buf = Buffer.alloc(S * S * 4);

const from = [139, 92, 246]; // violet-500
const to = [59, 130, 246]; // blue-500

const inRoundedRect = (x, y) => {
  const rx = Math.min(x, S - 1 - x);
  const ry = Math.min(y, S - 1 - y);
  if (rx >= R || ry >= R) return true;
  const dx = R - rx;
  const dy = R - ry;
  return dx * dx + dy * dy <= R * R;
};

// Sparkle: union of two four-point stars (diamonds with concave arms).
const cx = S / 2;
const cy = S / 2;
const star = (x, y, rad, thin) => {
  const nx = Math.abs(x - cx);
  const ny = Math.abs(y - cy);
  // Superellipse-ish concave diamond → pointed star arms.
  const p = thin ? 0.55 : 0.62;
  const v = Math.pow(nx / rad, p) + Math.pow(ny / rad, p);
  return v <= 1;
};

for (let y = 0; y < S; y++) {
  for (let x = 0; x < S; x++) {
    const i = (y * S + x) * 4;
    if (!inRoundedRect(x, y)) {
      buf[i + 3] = 0; // transparent outside the rounded square
      continue;
    }
    // Diagonal gradient.
    const t = (x + y) / (2 * S);
    let r = Math.round(from[0] + (to[0] - from[0]) * t);
    let g = Math.round(from[1] + (to[1] - from[1]) * t);
    let b = Math.round(from[2] + (to[2] - from[2]) * t);

    const bigStar = star(x, y, 300, false);
    const smallStar = star(x + 210, y + 210, 120, true); // small accent spark
    if (bigStar || smallStar) {
      r = g = b = 255;
    }
    buf[i] = r;
    buf[i + 1] = g;
    buf[i + 2] = b;
    buf[i + 3] = 255;
  }
}

// Encode PNG (RGBA, filter 0 per scanline).
const raw = Buffer.alloc((S * 4 + 1) * S);
for (let y = 0; y < S; y++) {
  raw[y * (S * 4 + 1)] = 0;
  buf.copy(raw, y * (S * 4 + 1) + 1, y * S * 4, (y + 1) * S * 4);
}

const crcTable = (() => {
  const t = [];
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
const crc32 = (b) => {
  let c = 0xffffffff;
  for (let i = 0; i < b.length; i++) c = crcTable[(c ^ b[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const t = Buffer.from(type, "ascii");
  const body = Buffer.concat([t, data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([len, body, crc]);
};

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // color type RGBA
const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

writeFileSync(new URL("../app-icon.png", import.meta.url), png);
console.log(`Wrote app-icon.png (${png.length} bytes)`);
