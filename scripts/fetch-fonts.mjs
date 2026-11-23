// Télécharge les polices libres (SIL Open Font License) proposées dans
// « Modifier PDF » et leurs licences, dans public/fonts/.
//
//   node scripts/fetch-fonts.mjs
//
// Les fichiers sont versionnés avec le projet : ce script ne sert qu'à les
// mettre à jour. L'application, elle, ne télécharge jamais rien.
//
// Source : Google Fonts (instances statiques TrueType, jeu de caractères
// complet) et le dépôt github.com/google/fonts pour les licences.

import { mkdir, writeFile, readdir, rm } from "node:fs/promises";
import { join } from "node:path";

const OUT = join(import.meta.dirname, "..", "public", "fonts");

/** id (fichiers), nom Google Fonts, dossier de licence, styles disponibles. */
const FAMILIES = [
  ["OpenSans", "Open Sans", "ofl/opensans", ["R", "B", "I", "BI"]],
  ["Roboto", "Roboto", "ofl/roboto", ["R", "B", "I", "BI"]],
  ["Lato", "Lato", "ofl/lato", ["R", "B", "I", "BI"]],
  ["Montserrat", "Montserrat", "ofl/montserrat", ["R", "B", "I", "BI"]],
  ["Poppins", "Poppins", "ofl/poppins", ["R", "B", "I", "BI"]],
  ["Nunito", "Nunito", "ofl/nunito", ["R", "B", "I", "BI"]],
  ["Oswald", "Oswald", "ofl/oswald", ["R", "B"]],
  ["BebasNeue", "Bebas Neue", "ofl/bebasneue", ["R"]],
  ["Lora", "Lora", "ofl/lora", ["R", "B", "I", "BI"]],
  ["PlayfairDisplay", "Playfair Display", "ofl/playfairdisplay", ["R", "B", "I", "BI"]],
  ["EBGaramond", "EB Garamond", "ofl/ebgaramond", ["R", "B", "I", "BI"]],
  ["RobotoMono", "Roboto Mono", "ofl/robotomono", ["R", "B", "I", "BI"]],
  ["Caveat", "Caveat", "ofl/caveat", ["R", "B"]],
  ["DancingScript", "Dancing Script", "ofl/dancingscript", ["R", "B"]],
  ["Pacifico", "Pacifico", "ofl/pacifico", ["R"]],
];

const STYLE = {
  R: { suffix: "Regular", ital: 0, wght: 400 },
  B: { suffix: "Bold", ital: 0, wght: 700 },
  I: { suffix: "Italic", ital: 1, wght: 400 },
  BI: { suffix: "BoldItalic", ital: 1, wght: 700 },
};

async function get(url, as = "text") {
  // Sans User-Agent de navigateur, Google Fonts renvoie des TrueType complets.
  const res = await fetch(url, { headers: { "User-Agent": "fetch-fonts" } });
  if (!res.ok) throw new Error(`${res.status} ${url}`);
  return as === "bytes" ? new Uint8Array(await res.arrayBuffer()) : res.text();
}

await mkdir(join(OUT, "licenses"), { recursive: true });
for (const f of await readdir(OUT)) if (f.endsWith(".ttf")) await rm(join(OUT, f));

let total = 0;
for (const [id, family, licenseDir, styles] of FAMILIES) {
  const hasItalic = styles.includes("I");
  const axes = hasItalic ? "ital,wght" : "wght";
  const tuples = styles
    .map((s) => STYLE[s])
    .map((s) => (hasItalic ? `${s.ital},${s.wght}` : `${s.wght}`))
    .sort()
    .join(";");
  const css = await get(`https://fonts.googleapis.com/css2?family=${encodeURIComponent(family).replace(/%20/g, "+")}:${axes}@${tuples}`);
  const faces = [...css.matchAll(/font-style:\s*(\w+);\s*font-weight:\s*(\d+);[^}]*?src:\s*url\(([^)]+\.ttf)\)/g)];
  for (const s of styles) {
    const st = STYLE[s];
    const face = faces.find((m) => (m[1] === "italic") === (st.ital === 1) && Number(m[2]) === st.wght);
    if (!face) throw new Error(`${family} ${st.suffix} introuvable`);
    const bytes = await get(face[3], "bytes");
    const name = `${id}-${st.suffix}.ttf`;
    await writeFile(join(OUT, name), bytes);
    total += bytes.length;
    console.log(`${name.padEnd(32)} ${(bytes.length / 1024).toFixed(0).padStart(5)} Kio`);
  }
  const license = await get(`https://raw.githubusercontent.com/google/fonts/main/${licenseDir}/OFL.txt`);
  await writeFile(join(OUT, "licenses", `${id}-OFL.txt`), license);
}
console.log(`Total : ${(total / 1024 / 1024).toFixed(2)} Mio`);
