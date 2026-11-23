# Composants tiers

Hestia PDF s'appuie sur les composants suivants. Leurs licences autorisent un
usage commercial comme open source.

| Composant | Rôle | Licence |
|---|---|---|
| [PDFium](https://pdfium.googlesource.com/pdfium/) (binaires [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries), version chromium/8125) | Lecture, rendu et assemblage des PDF | BSD-3-Clause / Apache-2.0 (+ bibliothèques incluses, voir `src-tauri/resources/pdfium/licenses/`) |
| [pdfium-render](https://github.com/ajrcarey/pdfium-render) | Interface Rust vers PDFium | MIT OU Apache-2.0 |
| [lopdf](https://github.com/J-F-Liu/lopdf) | Lecture/écriture bas niveau (compression, création) | MIT |
| [image](https://github.com/image-rs/image) | Décodage des images | MIT OU Apache-2.0 |
| [jpeg-encoder](https://github.com/vstroebel/jpeg-encoder) | Encodage JPEG | MIT OU Apache-2.0 |
| [subsetter](https://github.com/typst/subsetter) | Intégration des polices (glyphes utilisés seulement) | MIT OU Apache-2.0 |
| [ttf-parser](https://github.com/harfbuzz/ttf-parser) | Lecture des polices | MIT OU Apache-2.0 |
| [Tauri](https://tauri.app) | Fenêtre, interface, sécurité | MIT OU Apache-2.0 |
| [Papicons](https://github.com/PapillonApp/Papicons) — © Association Papillon | Icônes de l'interface | MIT (voir `src/assets/papicons/LICENSE`) |
| Polices de « Modifier PDF » : Open Sans, Roboto, Lato, Montserrat, Poppins, Nunito, Oswald, Bebas Neue, Lora, Playfair Display, EB Garamond, Roboto Mono, Caveat, Dancing Script, Pacifico (via [Google Fonts](https://fonts.google.com)) | Livrées avec l'application et intégrées aux PDF modifiés | SIL Open Font License 1.1 (voir `public/fonts/licenses/`) |
| [Vite](https://vite.dev), [TypeScript](https://www.typescriptlang.org) | Outils de compilation (non distribués) | MIT / Apache-2.0 |

Les icônes d'outils et le logo d'Hestia PDF ont été créés pour ce projet.
Les logos de marques présents dans le pack Papicons (Pronote, Apple, etc.)
ne sont pas utilisés.
