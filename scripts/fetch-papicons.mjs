// Vendorise les icônes Papicons (Association Papillon, licence MIT)
// dans src/assets/papicons/, à partir d'un commit figé du dépôt officiel.
//
//   node scripts/fetch-papicons.mjs
//
// - Les logos de marques et les icônes privées du pack sont exclus.
// - `fill="black"` est remplacé par `fill="currentColor"` pour que les
//   icônes prennent la couleur du thème (carmin, texte, etc.).
// - Seules les icônes importées dans le code finissent dans l'appli (Vite).

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = "PapillonApp/Papicons";
// Commit figé pour un résultat reproductible (changer volontairement pour mettre à jour).
const COMMIT = process.env.PAPICONS_COMMIT ?? "00af3a2a15436f6710306b0ed3a6f24231060de7";

// Marques déposées, logos et icônes réservées à l'appli Papillon : exclus.
const EXCLUDED = new Set([
  "apple",
  "butterfly",
  "ecoledirecte",
  "logotype",
  "papillon-app",
  "papillon-icon",
  "pronote",
  "skolengo",
]);

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src", "assets", "papicons");

async function getJson(url) {
  const res = await fetch(url, { headers: { Accept: "application/vnd.github+json" } });
  if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}`);
  return res.json();
}

async function getText(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}`);
  return res.text();
}

const commit = await getJson(`https://api.github.com/repos/${REPO}/commits/${COMMIT}`);
const sha = commit.sha;
const tree = await getJson(`https://api.github.com/repos/${REPO}/git/trees/${sha}?recursive=1`);

const icons = tree.tree
  .map((e) => e.path)
  .filter((p) => /^icons\/[a-z0-9-]+\.svg$/.test(p))
  .map((p) => p.slice("icons/".length, -".svg".length))
  .filter((name) => !EXCLUDED.has(name));

await mkdir(outDir, { recursive: true });

for (const name of icons) {
  const raw = await getText(`https://raw.githubusercontent.com/${REPO}/${sha}/icons/${name}.svg`);
  if (!raw.trimStart().startsWith("<svg") || /<script|on\w+=|href=/i.test(raw)) {
    throw new Error(`Contenu inattendu dans ${name}.svg, abandon.`);
  }
  const svg = raw
    .replace(/fill="(black|#000|#000000)"/g, 'fill="currentColor"')
    .replace(/\s+width="24"\s+height="24"/, "")
    .replace(/\r\n/g, "\n")
    .trim();
  await writeFile(join(outDir, `${name}.svg`), svg + "\n", "utf8");
}

const license = await getText(`https://raw.githubusercontent.com/${REPO}/${sha}/LICENSE`);
await writeFile(join(outDir, "LICENSE"), license, "utf8");
await writeFile(
  join(outDir, "SOURCE.md"),
  `Icônes issues de https://github.com/${REPO} (commit ${sha}), licence MIT.\n` +
    `Exclues : ${[...EXCLUDED].join(", ")}.\n` +
    `Modification : fill="black" remplacé par fill="currentColor".\n`,
  "utf8",
);

console.log(`${icons.length} icônes Papicons copiées (commit ${sha.slice(0, 7)}).`);
