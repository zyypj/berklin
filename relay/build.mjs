import { build } from "../ui/node_modules/esbuild/lib/main.js";
import { mkdirSync } from "node:fs";

mkdirSync("dist", { recursive: true });
await build({
  entryPoints: ["relay.js"],
  bundle: true,
  platform: "node",
  target: "node18",
  external: ["bufferutil", "utf-8-validate"],
  outfile: "dist/server.js",
  minify: true,
  legalComments: "none",
  logLevel: "info",
});
