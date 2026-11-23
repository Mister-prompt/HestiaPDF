// Polices proposées dans « Modifier PDF ».
//
// - Arial, Times New Roman et Courier New s'affichent avec les polices du
//   système et deviennent les polices standard du PDF (mêmes chasses).
// - Les autres sont livrées avec l'application (public/fonts, licence OFL) :
//   l'aperçu charge le même fichier que celui que Rust intègre au PDF.
//
// Le catalogue doit rester identique à crates/hestia-core/src/fonts.rs
// (vérifié par le test Rust `interface_catalog_matches`).

export type FontGroup = "sans" | "serif" | "mono" | "display" | "script";

export interface FontDef {
  id: string;
  label: string;
  group: FontGroup;
  /** Pile CSS de l'aperçu. */
  css: string;
  /** Gras disponible. */
  bold: boolean;
  /** Italique disponible (et gras italique si `bold`). */
  italic: boolean;
  /** Début des noms de fichiers, pour une police livrée avec l'application. */
  stem?: string;
}

const STANDARD: FontDef[] = [
  { id: "helvetica", label: "Arial", group: "sans", css: "Arial, Helvetica, 'Liberation Sans', sans-serif", bold: true, italic: true },
  { id: "times", label: "Times New Roman", group: "serif", css: "'Times New Roman', Times, 'Liberation Serif', serif", bold: true, italic: true },
  { id: "courier", label: "Courier New", group: "mono", css: "'Courier New', Courier, 'Liberation Mono', monospace", bold: true, italic: true },
];

const FALLBACK: Record<FontGroup, string> = {
  sans: "sans-serif",
  serif: "serif",
  mono: "monospace",
  display: "sans-serif",
  script: "cursive",
};

// [identifiant, fichiers, gras, italique, nom affiché, famille]
const BUNDLED: [string, string, boolean, boolean, string, FontGroup][] = [
  ["opensans", "OpenSans", true, true, "Open Sans", "sans"],
  ["roboto", "Roboto", true, true, "Roboto", "sans"],
  ["lato", "Lato", true, true, "Lato", "sans"],
  ["montserrat", "Montserrat", true, true, "Montserrat", "sans"],
  ["poppins", "Poppins", true, true, "Poppins", "sans"],
  ["nunito", "Nunito", true, true, "Nunito", "sans"],
  ["oswald", "Oswald", true, false, "Oswald", "display"],
  ["bebasneue", "BebasNeue", false, false, "Bebas Neue", "display"],
  ["lora", "Lora", true, true, "Lora", "serif"],
  ["playfair", "PlayfairDisplay", true, true, "Playfair Display", "serif"],
  ["ebgaramond", "EBGaramond", true, true, "EB Garamond", "serif"],
  ["robotomono", "RobotoMono", true, true, "Roboto Mono", "mono"],
  ["caveat", "Caveat", true, false, "Caveat", "script"],
  ["dancingscript", "DancingScript", true, false, "Dancing Script", "script"],
  ["pacifico", "Pacifico", false, false, "Pacifico", "script"],
];

export const FONT_GROUPS: FontGroup[] = ["sans", "serif", "mono", "display", "script"];

export const FONTS: FontDef[] = [
  ...STANDARD,
  ...BUNDLED.map(([id, stem, bold, italic, label, group]) => ({
    id,
    stem,
    bold,
    italic,
    label,
    group,
    css: `"hst-${id}", ${FALLBACK[group]}`,
  })),
];

export function fontDef(id: string): FontDef {
  return FONTS.find((f) => f.id === id) ?? STANDARD[0]!;
}

export function fontCss(id: string): string {
  return fontDef(id).css;
}

/** Style réellement utilisé : le gras ou l'italique absents ne sont jamais simulés (comme dans le PDF). */
export function effectiveStyle(id: string, bold: boolean, italic: boolean): { bold: boolean; italic: boolean } {
  const def = fontDef(id);
  return { bold: bold && def.bold, italic: italic && def.italic };
}

function fileOf(def: FontDef, bold: boolean, italic: boolean): string | null {
  if (!def.stem) return null;
  const s = effectiveStyle(def.id, bold, italic);
  const suffix = s.bold ? (s.italic ? "BoldItalic" : "Bold") : s.italic ? "Italic" : "Regular";
  return `${def.stem}-${suffix}.ttf`;
}

// --- Chargement --------------------------------------------------------------

const ready = new Set<string>();
const loading = new Map<string, Promise<void>>();
const coverage = new Map<string, (cp: number) => boolean>();
const listeners = new Set<() => void>();

/** Prévient quand une police vient d'être chargée (l'aperçu doit être redessiné). */
export function onFontLoaded(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Lance le chargement si besoin ; `true` si la police est prête à l'affichage. */
export function ensureFont(id: string, bold = false, italic = false): boolean {
  const def = fontDef(id);
  const file = fileOf(def, bold, italic);
  if (!file || ready.has(file)) return true;
  if (!loading.has(file)) {
    const s = effectiveStyle(id, bold, italic);
    const task = (async () => {
      const res = await fetch(`/fonts/${file}`);
      if (!res.ok) throw new Error(`${res.status}`);
      const data = await res.arrayBuffer();
      coverage.set(file, cmapLookup(data));
      const face = new FontFace(`hst-${def.id}`, data, { weight: s.bold ? "700" : "400", style: s.italic ? "italic" : "normal" });
      await face.load();
      document.fonts.add(face);
      ready.add(file);
      for (const fn of listeners) fn();
    })().catch(() => {
      // Police introuvable : l'aperçu garde la police de secours.
      loading.delete(file);
    });
    loading.set(file, task);
  }
  return false;
}

/** Charge la graisse normale de toutes les polices (aperçu de la liste). */
export function preloadFonts(): void {
  for (const f of FONTS) ensureFont(f.id);
}

// --- Caractères disponibles ----------------------------------------------------

/** Caractères présents dans les polices PDF standard (WinAnsi). */
const WIN_ANSI_EXTRA = new Set("€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ   \t");

function inWinAnsi(ch: string): boolean {
  const c = ch.codePointAt(0) ?? 0;
  return (c >= 0x20 && c <= 0x7e) || (c >= 0xa0 && c <= 0xff) || WIN_ANSI_EXTRA.has(ch);
}

/** Caractères que la police ne sait pas dessiner (ils deviendront « ? » dans le PDF). */
export function missingChars(text: string, id: string, bold: boolean, italic: boolean): string[] {
  const def = fontDef(id);
  const file = fileOf(def, bold, italic);
  const has = file ? coverage.get(file) : undefined;
  const bad = new Set<string>();
  for (const ch of text) {
    if (ch === "\n" || ch === "\t") continue;
    if (file) {
      // Police pas encore chargée : rien à signaler pour l'instant.
      if (has && !has(ch.codePointAt(0) ?? 0)) bad.add(ch);
    } else if (!inWinAnsi(ch)) {
      bad.add(ch);
    }
  }
  return [...bad];
}

/** Lecture de la table `cmap` (formats 4 et 12) d'une police TrueType. */
function cmapLookup(buf: ArrayBuffer): (cp: number) => boolean {
  const none = () => false;
  try {
    const v = new DataView(buf);
    let cmap = -1;
    const tables = v.getUint16(4);
    for (let i = 0; i < tables; i++) {
      const rec = 12 + i * 16;
      if (v.getUint32(rec) === 0x636d6170) cmap = v.getUint32(rec + 8); // « cmap »
    }
    if (cmap < 0) return none;
    let best = -1;
    let format = 0;
    const count = v.getUint16(cmap + 2);
    for (let i = 0; i < count; i++) {
      const rec = cmap + 4 + i * 8;
      const platform = v.getUint16(rec);
      const encoding = v.getUint16(rec + 2);
      if (!(platform === 0 || (platform === 3 && (encoding === 1 || encoding === 10)))) continue;
      const off = cmap + v.getUint32(rec + 4);
      const f = v.getUint16(off);
      if (f === 12 && format !== 12) [best, format] = [off, 12];
      else if (f === 4 && format === 0) [best, format] = [off, 4];
    }
    if (best < 0) return none;
    const at = best;
    if (format === 12) {
      const groups = v.getUint32(at + 12);
      return (cp) => {
        for (let g = 0; g < groups; g++) {
          const o = at + 16 + g * 12;
          const start = v.getUint32(o);
          if (cp >= start && cp <= v.getUint32(o + 4)) return v.getUint32(o + 8) + (cp - start) !== 0;
        }
        return false;
      };
    }
    const segX2 = v.getUint16(at + 6);
    const ends = at + 14;
    const starts = ends + segX2 + 2;
    const deltas = starts + segX2;
    const ranges = deltas + segX2;
    return (cp) => {
      if (cp > 0xffff) return false;
      for (let i = 0; i < segX2; i += 2) {
        if (cp > v.getUint16(ends + i)) continue;
        const start = v.getUint16(starts + i);
        if (cp < start) return false;
        const delta = v.getUint16(deltas + i);
        const ro = v.getUint16(ranges + i);
        if (ro === 0) return ((cp + delta) & 0xffff) !== 0;
        const g = v.getUint16(ranges + i + ro + (cp - start) * 2);
        return g !== 0 && ((g + delta) & 0xffff) !== 0;
      }
      return false;
    };
  } catch {
    return none;
  }
}
