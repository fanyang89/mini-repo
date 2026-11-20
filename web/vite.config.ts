import path from "node:path"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"
import { defineConfig } from "vite"

const backend = process.env.MINI_REPO_BACKEND ?? "http://127.0.0.1:5000"

export default defineConfig({
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "src") },
  },
  server: {
    proxy: {
      "/mini-repo/api": backend,
      "/incoming": backend,
      "/repos": backend,
    },
  },
})
