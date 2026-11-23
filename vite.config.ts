import { defineConfig } from "vite";

// Configuration Vite adaptée à Tauri (port fixe, pas d'effacement de la console).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
  build: {
    target: "es2022",
    // Pas de sourcemaps ni de code inline dans l'appli publiée.
    sourcemap: false,
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    modulePreload: { polyfill: false },
  },
});
