import path from "node:path";
import type { NextConfig } from "next";

// Static export is the only build mode. The Rust server embeds apps/web/out, so
// nothing here may need a Node server at run time: no route handlers, server
// actions, middleware, rewrites, redirects, or the image optimiser.
const config: NextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  reactStrictMode: true,
  // `next dev` would otherwise write AGENTS.md and CLAUDE.md into this folder.
  agentRules: false,
  // The repository root has its own package.json and pnpm-lock.yaml for the git
  // hook tooling (husky, lint-staged), so without this Next would infer the
  // repository root as the workspace root and watch everything, including the
  // Rust target directory. The app is this directory.
  turbopack: { root: path.resolve(import.meta.dirname) },
};

export default config;
