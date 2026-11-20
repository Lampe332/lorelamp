import { defineConfig } from "vite";
import babel from "@rolldown/plugin-babel";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import pkg from "./package.json" with { type: "json" };

// Dev: `npm run dev` proxies API + WebSocket to a running rp-server on :7860.
export default defineConfig({
  // React Compiler (official Babel plugin): memoizes components and hooks automatically. Components
  // that break the rules of React are left as they are, so the output never behaves differently.
  plugins: [react(), babel({ presets: [reactCompilerPreset()] })],
  // The version the UI writes into the activity log at start.
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  server: {
    proxy: { "/api": { target: "http://127.0.0.1:7860", ws: true } },
  },
});
