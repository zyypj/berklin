import * as esbuild from "esbuild";
import { copyFileSync, mkdirSync, rmSync } from "node:fs";

const watch = process.argv.includes("--watch");
rmSync("dist", { recursive: true, force: true });
mkdirSync("dist", { recursive: true });
copyFileSync("src/index.html", "dist/index.html");

const options = {
  entryPoints: { app: "src/main.jsx" },
  bundle: true,
  minify: !watch,
  format: "iife",
  target: ["chrome100"],
  jsx: "automatic",
  jsxImportSource: "preact",
  outdir: "dist",
  legalComments: "none",
  loader: { ".css": "css" },
  logLevel: "info",
};

if (watch) {
  const ctx = await esbuild.context(options);
  await ctx.watch();
} else {
  await esbuild.build(options);
}
