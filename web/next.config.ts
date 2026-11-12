import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Every page reads files that the trader rewrites every few seconds, so nothing is cached.
  devIndicators: false,
  turbopack: {
    rules: {
      "*.css": {
        loaders: ["@tailwindcss/turbopack"],
        as: "*.css",
      },
    },
  },
};

export default nextConfig;
