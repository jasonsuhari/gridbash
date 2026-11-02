import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: { port: 5179, strictPort: true },
  build: { target: "safari16" },
});
