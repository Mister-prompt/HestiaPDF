import "./styles/app.css";

import { App, initTheme } from "./app";

initTheme();

// Comportements de navigateur inutiles dans une appli de bureau : menu
// contextuel, rechargement, impression de la page, recherche.
document.addEventListener("contextmenu", (e) => {
  const target = e.target as HTMLElement | null;
  if (!target?.closest("input, textarea")) e.preventDefault();
});
document.addEventListener("keydown", (e) => {
  const key = e.key.toLowerCase();
  const ctrl = e.ctrlKey || e.metaKey;
  if (key === "f5" || (ctrl && ["r", "p", "f", "g", "u", "s", "j"].includes(key)) || (ctrl && e.shiftKey && key === "i")) {
    e.preventDefault();
  }
});
// Empêche le navigateur d'ouvrir un fichier glissé hors des zones prévues.
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => e.preventDefault());

const root = document.getElementById("app");
if (root) void new App(root).start();
