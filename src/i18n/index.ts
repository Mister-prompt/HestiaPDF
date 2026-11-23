// Traductions : français par défaut, anglais si Windows est en anglais
// (ou si l'utilisateur le choisit dans les réglages).

import en from "./en";
import fr from "./fr";
import { readSetting, writeSetting } from "../storage";

export type Lang = "fr" | "en";
export type Key = keyof typeof fr;

const dictionaries: Record<Lang, Record<Key, string>> = { fr, en };

function detect(): Lang {
  const saved = readSetting("lang");
  if (saved === "fr" || saved === "en") return saved;
  const nav = (navigator.language || "fr").toLowerCase();
  return nav.startsWith("fr") ? "fr" : nav.startsWith("en") ? "en" : "fr";
}

let current: Lang = detect();
const listeners = new Set<() => void>();

export function getLang(): Lang {
  return current;
}

export function setLang(lang: Lang): void {
  if (lang === current) return;
  current = lang;
  writeSetting("lang", lang);
  document.documentElement.lang = lang;
  listeners.forEach((fn) => fn());
}

export function onLangChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Texte traduit, avec remplacement des variables {nom}. */
export function t(key: Key, vars?: Record<string, string | number>): string {
  const text = dictionaries[current][key] ?? fr[key] ?? key;
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (_, name: string) => (name in vars ? String(vars[name]) : `{${name}}`));
}

/** Singulier / pluriel. */
export function tn(one: Key, many: Key, n: number): string {
  return n === 1 ? t(one, { n }) : t(many, { n });
}

/** Taille de fichier lisible (Ko, Mo…). */
export function formatSize(bytes: number): string {
  const units = current === "fr" ? ["o", "Ko", "Mo", "Go"] : ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toLocaleString(current === "fr" ? "fr-FR" : "en-US", { maximumFractionDigits: digits })} ${units[unit]}`;
}

document.documentElement.lang = current;
