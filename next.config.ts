import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // node-pty is a native module that can't be bundled by webpack
  serverExternalPackages: ["node-pty"],
};

export default nextConfig;
