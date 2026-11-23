# Hestia PDF

**Vos outils PDF, 100 % en local.** Fusionner, diviser, compresser et convertir
des PDF sans jamais envoyer un fichier sur Internet.

- Rapide : interface native légère (Tauri), moteur PDF de Chromium (PDFium) chargé à la demande.
- Respect de la vie privée : aucune connexion réseau, aucune télémétrie, aucun compte.
- Sûr : le fichier d'origine n'est jamais remplacé, les écritures sont atomiques,
  l'interface n'a accès qu'aux fichiers que vous choisissez.

## Outils

| Outil | État |
|---|---|
| Fusionner PDF | ✅ |
| Diviser PDF (plages, pages fixes, extraction) | ✅ |
| Compresser PDF (3 niveaux, traitement par lot) | ✅ |
| PDF en JPG (pages ou images intégrées, par lot) | ✅ |
| JPG en PDF (JPG, PNG, WebP, BMP, TIFF, GIF ; orientation, marges, rotation) | ✅ |
| Visionneuse PDF | ✅ |
| PDF en Word / PowerPoint / Excel (moteur intégré, sans Office) | ✅ |
| Word / PowerPoint / Excel en PDF (avec Microsoft Office, ou LibreOffice) | ✅ |
| Modifier PDF (texte en 18 polices, images pivotables/miroir/noir et blanc, formes, flèches, dessin, surligneur ; annuler/rétablir) | ✅ |

## Développement

Prérequis : [Rust](https://rustup.rs) ≥ 1.90, Node.js ≥ 20, outils de compilation
Visual Studio (C++), WebView2 (déjà présent sur Windows 10/11).

```powershell
npm install
.\scripts\fetch-pdfium.ps1     # télécharge et vérifie PDFium (une seule fois)
npm run tauri dev              # lance l'appli en mode développement
```

Tests :

```powershell
cargo test --workspace         # moteur PDF (avec PDFium réel) + sécurité
npm run check                  # vérification TypeScript
```

Aperçu de l'interface sans le moteur Rust (données simulées, pratique pour retoucher le design) :

```powershell
npm run dev                    # puis ouvrir http://localhost:1420/dev/preview.html
```

Fichiers d'essai (PDF illustré, PDF lourd à compresser, PDF protégé par le mot de passe « hestia », photos) :

```powershell
cargo run -p hestia-core --example fixtures -- .\essais
```

Version de production (installateurs `.msi` et `.exe` dans `target/release/bundle/`) :

```powershell
npm run tauri build
```

## Architecture

```
crates/hestia-core/   Moteur PDF en Rust, indépendant de l'interface (testé seul)
src-tauri/            Application Tauri : commandes, sécurité, fenêtres natives
src/                  Interface (TypeScript sans framework, aucune dépendance d'exécution hormis l'API Tauri)
scripts/              Récupération vérifiée de PDFium, des icônes Papicons et des polices (OFL)
```

### Sécurité

- **Aucun réseau** : CSP stricte (`connect-src` limité à l'IPC Tauri et aux fichiers de l'application elle-même, comme les polices), aucun plugin
  HTTP, pas de mise à jour automatique ni de télémétrie.
- **Liste blanche de fichiers** : l'interface ne peut agir que sur les fichiers
  choisis par l'utilisateur (boîte de dialogue ou glisser-déposer) ; les chemins de
  sortie sont toujours choisis dans une boîte de dialogue ouverte par Rust.
- **Permissions par commande** (`src-tauri/capabilities/main.json`) : seules les
  commandes d'Hestia PDF sont exposées, aucune API Tauri de fichier ou de shell.
- **Navigation verrouillée** : la fenêtre ne peut afficher que l'appli elle-même.
- **Robustesse** : protections contre les « bombes de décompression », limites de
  taille d'image, isolement de PDFium sur un thread dédié, écritures atomiques,
  vérification du PDF compressé avant écriture.
- **Dépendances** : licences permissives uniquement (`deny.toml`), versions figées,
  PDFium vérifié par empreinte SHA-256.

Composants tiers : voir [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
