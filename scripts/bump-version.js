"use strict";

const fs = require("fs");
const path = require("path");

const next = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(next || "")) {
  console.error("uso: node scripts/bump-version.js X.Y.Z");
  process.exit(1);
}

const root = path.join(__dirname, "..");
const file = (rel) => path.join(root, rel);

function rewrite(rel, pattern, replacement) {
  const p = file(rel);
  const before = fs.readFileSync(p, "utf8");
  const after = before.replace(pattern, replacement);
  if (before === after) throw new Error(`nada mudou em ${rel}`);
  fs.writeFileSync(p, after);
}

rewrite("Cargo.toml", /^version = "[^"]+"/m, `version = "${next}"`);
for (const rel of ["app/tauri.conf.json", "relay/package.json", "ui/package.json"]) {
  rewrite(rel, /"version": "[^"]+"/, `"version": "${next}"`);
}
const gradle = file("android/gradle.properties");
const props = fs.readFileSync(gradle, "utf8");
const code = Number(/BERKLIN_VERSION_CODE=(\d+)/.exec(props)[1]) + 1;
fs.writeFileSync(
  gradle,
  props.replace(/BERKLIN_VERSION_NAME=.*/, `BERKLIN_VERSION_NAME=${next}`).replace(/BERKLIN_VERSION_CODE=\d+/, `BERKLIN_VERSION_CODE=${code}`)
);
console.log(`versão ${next} (android versionCode ${code})`);
