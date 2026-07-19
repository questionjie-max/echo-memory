import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 使用固定的开发端口；与 tauri.conf.json 的 devUrl 保持一致。
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],

  // Tauri 期望相对资产路径。
  base: "./",

  // 避免 Tauri 窗口内清屏。
  clearScreen: false,

  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      // 后端改动不应触发前端热重载。
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
