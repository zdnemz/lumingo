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
};

export default config;
