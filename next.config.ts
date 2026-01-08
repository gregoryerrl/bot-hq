import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // node-pty is a native module that can't be bundled by webpack
  serverExternalPackages: ["node-pty"],
  // Enable instrumentation for server startup hooks
  experimental: {
    instrumentationHook: true,
  },
};

export default nextConfig;
