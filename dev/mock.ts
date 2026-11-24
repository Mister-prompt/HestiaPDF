// Aperçu de l'interface dans un navigateur, SANS le moteur Rust :
//   npm run dev   puis ouvrir http://localhost:1420/dev/preview.html
// Les commandes Tauri sont simulées avec de fausses données. Ce fichier
// n'est jamais inclus dans l'application publiée (seul index.html l'est).

import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

interface FakeFile {
  path: string;
  name: string;
  size: number;
  kind: "pdf" | "image" | "word" | "powerpoint" | "excel";
  pages?: number;
  password?: string;
  color?: string;
}

const DIR = "C:\\Utilisateurs\\Moi\\Documents\\";
const FILES: FakeFile[] = [
  { path: `${DIR}rapport-annuel.pdf`, name: "rapport-annuel.pdf", size: 2_480_000, kind: "pdf", pages: 12, color: "#b91c1c" },
  { path: `${DIR}annexe.pdf`, name: "annexe.pdf", size: 310_000, kind: "pdf", pages: 3, color: "#2563eb" },
  { path: `${DIR}contrat-signé.pdf`, name: "contrat-signé.pdf", size: 820_000, kind: "pdf", pages: 4, password: "hestia", color: "#059669" },
  { path: `${DIR}vacances.jpg`, name: "vacances.jpg", size: 3_600_000, kind: "image", color: "#d97706" },
  { path: `${DIR}scan-facture.png`, name: "scan-facture.png", size: 1_200_000, kind: "image", color: "#7c3aed" },
  { path: `${DIR}lettre-candidature.docx`, name: "lettre-candidature.docx", size: 48_000, kind: "word" },
  { path: `${DIR}devis-client.docx`, name: "devis-client.docx", size: 112_000, kind: "word" },
  { path: `${DIR}présentation-projet.pptx`, name: "présentation-projet.pptx", size: 5_400_000, kind: "powerpoint" },
  { path: `${DIR}budget-2026.xlsx`, name: "budget-2026.xlsx", size: 64_000, kind: "excel" },
];

const NO_LOCK = new URLSearchParams(location.search).has("nolock");
const byPath = (p: string) => FILES.find((f) => f.path === p);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function fakePage(file: FakeFile, page: number, width: number, landscape = false): Promise<ArrayBuffer> {
  const w = Math.max(64, Math.round(width));
  const h = Math.round(landscape ? w * 0.7 : w * 1.414);
  const canvas = new OffscreenCanvas(w, h);
  const g = canvas.getContext("2d")!;
  g.fillStyle = "#fff";
  g.fillRect(0, 0, w, h);
  g.fillStyle = file.color ?? "#b91c1c";
  g.fillRect(0, 0, w, h * 0.1);
  g.fillStyle = "#222";
  g.font = `bold ${Math.round(w * 0.11)}px Segoe UI, sans-serif`;
  g.fillText(file.kind === "image" ? "IMG" : `Page ${page + 1}`, w * 0.08, h * 0.25);
  g.fillStyle = "#ccc";
  for (let i = 0; i < 9; i++) g.fillRect(w * 0.08, h * (0.35 + i * 0.06), w * (0.84 - (i % 3) * 0.15), h * 0.018);
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.85 });
  return blob.arrayBuffer();
}

type Channelish = { onmessage: (p: { done: number; total: number }) => void } | undefined;

async function progress(channel: Channelish, total: number, ms = 1600): Promise<void> {
  for (let i = 0; i <= total; i++) {
    channel?.onmessage?.({ done: i, total });
    await sleep(ms / total);
  }
}

const out = (name: string, size: number) => ({ path: `${DIR}${name}`, size });

mockIPC(
  async (cmd, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    switch (cmd) {
      case "app_info":
        return { version: "0.2.2" };
      case "startup_files":
        return [];
      case "pick_files":
        await sleep(300);
        // ?nolock : sans le PDF protégé (captures automatiques sans fenêtre de mot de passe).
        return FILES.filter((f) => f.kind === args.kind && !(NO_LOCK && f.password)).map(({ path, name, size, kind }) => ({ path, name, size, kind }));
      case "inspect_pdf": {
        const f = byPath(args.input.path);
        if (!f) throw { code: "forbidden", file: null, detail: "" };
        if (f.password && args.input.password !== f.password) {
          throw { code: args.input.password ? "wrong_password" : "password_required", file: f.name, detail: "" };
        }
        await sleep(120);
        return {
          pageCount: f.pages ?? 1,
          pages: Array.from({ length: f.pages ?? 1 }, () => ({ width: 595, height: 842 })),
          fileSize: f.size,
          encrypted: !!f.password,
        };
      }
      case "render_page":
        await sleep(60);
        return fakePage(byPath(args.input.path)!, args.page, args.width);
      case "image_thumbnail":
        return fakePage(byPath(args.path)!, 0, args.size, true);
      case "cancel_job":
      case "reveal_in_folder":
        return null;
      case "merge_pdfs":
        await sleep(500);
        await progress(args.onProgress, args.inputs.length);
        return { cancelled: false, outputs: [out("fusion.pdf", 3_100_000)], items: [], folder: null, hadPassword: args.inputs.some((i: any) => i.password) };
      case "organize_pdf":
        // Pour vérifier depuis la console ce qui partirait vers Rust.
        (window as any).__lastOrganize = { inputs: args.inputs, pages: args.pages };
        await sleep(400);
        await progress(args.onProgress, args.pages.length);
        return { cancelled: false, outputs: [out("organise.pdf", 2_400_000)], items: [], folder: null, hadPassword: args.inputs.some((i: any) => i.password) };
      case "split_pdf":
        await sleep(500);
        await progress(args.onProgress, 3);
        return { cancelled: false, outputs: [out("rapport_pages-1-6.pdf", 1e6), out("rapport_pages-7-12.pdf", 1e6)], items: [], folder: DIR, hadPassword: false };
      case "compress_pdfs": {
        await sleep(500);
        await progress(args.onProgress, 4);
        const items = args.inputs.map((i: any, k: number) => {
          const f = byPath(i.path)!;
          return { name: f.name, outputs: [out(f.name.replace(".pdf", "_compressé.pdf"), f.size * 0.38)], error: null, originalSize: f.size, compressedSize: Math.round(f.size * (k === 1 ? 1 : 0.38)), alreadyOptimal: k === 1 };
        });
        return { cancelled: false, outputs: items.flatMap((i: any) => i.outputs), items, folder: items.length > 1 ? DIR : null, hadPassword: false };
      }
      case "pdf_to_jpg":
        await sleep(500);
        await progress(args.onProgress, 12);
        return { cancelled: false, outputs: Array.from({ length: 12 }, (_, i) => out(`rapport_page-${i + 1}.jpg`, 4e5)), items: [{ name: "rapport-annuel.pdf", outputs: [], error: null, originalSize: null, compressedSize: null, alreadyOptimal: false }], folder: DIR, hadPassword: false };
      case "jpg_to_pdf":
        await sleep(500);
        await progress(args.onProgress, args.inputs.length);
        return { cancelled: false, outputs: [out("images.pdf", 5e6)], items: [], folder: null, hadPassword: false };
      case "office_status":
        await sleep(500);
        return { word: true, powerpoint: true, excel: true, libreOffice: false, offlinePrinter: NO_LOCK ? null : "HP Deskjet 2540 series" };
      case "office_to_pdf_files":
        await sleep(400);
        await progress(args.onProgress, args.inputs.length, 2400);
        return args.inputs.length === 1
          ? { cancelled: false, outputs: [out(byPath(args.inputs[0])!.name.replace(/.w+$/, ".pdf"), 380_000)], items: [], folder: null, hadPassword: false }
          : { cancelled: false, outputs: args.inputs.map((p: string) => out(byPath(p)!.name.replace(/.w+$/, ".pdf"), 380_000)), items: [], folder: DIR, hadPassword: false };
      case "pdf_to_office_files": {
        await sleep(400);
        await progress(args.onProgress, 13, 1800);
        const ext = args.format;
        return { cancelled: false, outputs: args.inputs.map((i: any) => out(byPath(i.path)!.name.replace(".pdf", `.${ext}`), 240_000)), items: [], folder: args.inputs.length > 1 ? DIR : null, hadPassword: false, noText: false };
      }
      case "edit_pdf":
        // Pour vérifier depuis la console ce qui partirait vers Rust.
        (window as any).__lastEdit = args.edits;
        await sleep(400);
        await progress(args.onProgress, args.edits.pages.length + 2);
        return { cancelled: false, outputs: [out("rapport_modifié.pdf", 2_600_000)], items: [], folder: null, hadPassword: !!args.input.password, replacedChars: 0, rebuilt: false };
      default:
        throw { code: "internal", file: null, detail: `commande simulée inconnue : ${cmd}` };
    }
  },
  { shouldMockEvents: true },
);

mockWindows("main");
await import("../src/main.ts");
