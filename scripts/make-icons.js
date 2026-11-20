"use strict";

const fs = require("fs");
const path = require("path");
const zlib = require("zlib");

const CRC_TABLE = new Int32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c;
});

function crc32(buf) {
  let c = -1;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}

function png(size, pixel) {
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    for (let x = 0; x < size; x++) {
      const [r, g, b, a] = pixel((x + 0.5) / size, (y + 0.5) / size, 1 / size);
      const o = y * (size * 4 + 1) + 1 + x * 4;
      raw[o] = r;
      raw[o + 1] = g;
      raw[o + 2] = b;
      raw[o + 3] = a;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function clamp01(v) {
  return Math.max(0, Math.min(1, v));
}

function roundedRectDist(x, y, half, radius) {
  const qx = Math.abs(x - 0.5) - half + radius;
  const qy = Math.abs(y - 0.5) - half + radius;
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - radius;
}

function bubbleDist(x, y) {
  const body = roundedRectDist((x - 0.5) / 0.9 + 0.5, (y - 0.47) / 0.72 + 0.5, 0.29, 0.13) * 0.8;
  const tx = x - 0.36;
  const ty = y - 0.66;
  const tail = Math.max(-ty, ty - 0.12, tx - 0.12 + ty * 0.2, -tx - ty * 0.9);
  return Math.min(body, tail);
}

function pixel(x, y, px) {
  const bg = roundedRectDist(x, y, 0.47, 0.2);
  const bgA = clamp01(0.5 - bg / px);
  if (bgA <= 0) return [0, 0, 0, 0];
  const t = clamp01((x + y) / 2);
  let r = 255 + (226 - 255) * t;
  let g = 160 + (87 - 160) * t;
  let b = 70 + (10 - 70) * t;
  const bub = bubbleDist(x, y);
  const bubA = clamp01(0.5 - bub / px);
  r = r + (255 - r) * bubA;
  g = g + (255 - g) * bubA;
  b = b + (255 - b) * bubA;
  const dots = [0.38, 0.5, 0.62].some((cx) => Math.hypot(x - cx, y - 0.47) < 0.04);
  if (dots && bubA > 0.5) {
    r = 240;
    g = 110;
    b = 20;
  }
  return [Math.round(r), Math.round(g), Math.round(b), Math.round(bgA * 255)];
}

function ico(images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  const entries = [];
  let offset = 6 + images.length * 16;
  for (const { size, data } of images) {
    const e = Buffer.alloc(16);
    e[0] = size >= 256 ? 0 : size;
    e[1] = size >= 256 ? 0 : size;
    e.writeUInt16LE(1, 4);
    e.writeUInt16LE(32, 6);
    e.writeUInt32LE(data.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += data.length;
    entries.push(e);
  }
  return Buffer.concat([header, ...entries, ...images.map((i) => i.data)]);
}

const out = process.argv[2] || path.join(__dirname, "..", "app", "icons");
fs.mkdirSync(out, { recursive: true });
const sizes = [16, 24, 32, 48, 64, 128, 256];
const images = sizes.map((size) => ({ size, data: png(size, pixel) }));
fs.writeFileSync(path.join(out, "icon.png"), png(512, pixel));
fs.writeFileSync(path.join(out, "icon.ico"), ico(images));
fs.writeFileSync(path.join(out, "32x32.png"), images[2].data);
fs.writeFileSync(path.join(out, "128x128.png"), images[5].data);
console.log("icons written to", out);
