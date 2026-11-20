import { build } from "../ui/node_modules/esbuild/lib/main.js";
import { mkdirSync, readFileSync } from "node:fs";

const version = /^version = "(.+)"/m.exec(readFileSync("../Cargo.toml", "utf8"))[1];
mkdirSync("dist", { recursive: true });
await build({
  entryPoints: ["supervisor.js"],
  bundle: true,
  platform: "node",
  target: "node18",
  external: ["bufferutil", "utf-8-validate"],
  define: { __BERKLIN_VERSION__: JSON.stringify(version) },
  outfile: "dist/server.js",
  minify: true,
  legalComments: "none",
  logLevel: "info",
});
