// Accès typé aux commandes Rust (Tauri). L'interface ne touche jamais au
// disque elle-même : tout passe par ces commandes, vérifiées côté Rust.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type FileKind = "pdf" | "image" | "word" | "powerpoint" | "excel";

/** Documents bureautiques (convertis en PDF avec Office ou LibreOffice). */
export const OFFICE_KINDS: FileKind[] = ["word", "powerpoint", "excel"];

/** Logiciels disponibles pour les conversions Office. */
export interface OfficeInfo {
  word: boolean;
  powerpoint: boolean;
  excel: boolean;
  libreOffice: boolean;
  /** Imprimante par défaut hors ligne (Word et Excel peuvent ne plus répondre). */
  offlinePrinter: string | null;
}

export type OfficeFormat = "docx" | "pptx" | "xlsx";

export interface PdfToOfficeOutcome extends JobOutcome {
  /** Un PDF ne contenait pas de texte (scan) : seules les images ont été reprises. */
  noText: boolean;
}

export interface FileInfo {
  path: string;
  name: string;
  size: number;
  kind: FileKind;
}

export interface PageSize {
  width: number;
  height: number;
}

export interface PdfInfo {
  pageCount: number;
  pages: PageSize[];
  fileSize: number;
  encrypted: boolean;
}

export interface PdfInput {
  path: string;
  password?: string | null;
}

export interface OutputFile {
  path: string;
  size: number;
}

export interface CommandError {
  code: string;
  file: string | null;
  detail: string;
}

export interface ItemOutcome {
  name: string;
  outputs: OutputFile[];
  error: CommandError | null;
  originalSize: number | null;
  compressedSize: number | null;
  alreadyOptimal: boolean;
}

export interface JobOutcome {
  cancelled: boolean;
  outputs: OutputFile[];
  items: ItemOutcome[];
  folder: string | null;
  hadPassword: boolean;
}

export interface Progress {
  done: number;
  total: number;
}

export type SplitMode =
  | { mode: "ranges"; ranges: string; merge: boolean }
  | { mode: "every"; size: number }
  | { mode: "allPages" }
  | { mode: "select"; pages: number[]; merge: boolean };

/** Une page du PDF organisé (voir hestia-core/src/organize.rs). */
export type PageSpec =
  | { kind: "source"; file: number; page: number; rotation: number }
  | { kind: "blank"; width: number; height: number; rotation: number };

export type CompressionLevel = "low" | "recommended" | "extreme";

export type ImageExport = { mode: "pages"; quality: "normal" | "high" } | { mode: "extract" };

export interface ImageInput {
  path: string;
  rotation: number;
}

export interface ImagesToPdfOptions {
  pageSize: "a4" | "letter" | "fit";
  orientation: "auto" | "portrait" | "landscape";
  margin: "none" | "small" | "large";
}

/** Élément ajouté par « Modifier PDF » (format attendu par Rust). */
export type EditObject =
  | {
      type: "text";
      x: number;
      y: number;
      size: number;
      /** Police standard (helvetica, times, courier) ou livrée avec l'application. */
      font: string;
      bold: boolean;
      italic: boolean;
      color: string;
      opacity: number;
      lines: string[];
    }
  | {
      type: "rect" | "ellipse";
      x: number;
      y: number;
      w: number;
      h: number;
      stroke: string | null;
      fill: string | null;
      strokeWidth: number;
      opacity: number;
    }
  | { type: "line"; x1: number; y1: number; x2: number; y2: number; color: string; width: number; opacity: number; arrow: boolean }
  | { type: "ink"; points: [number, number][]; color: string; width: number; opacity: number }
  | {
      type: "image";
      x: number;
      y: number;
      w: number;
      h: number;
      path: string;
      opacity: number;
      /** Rotation horaire : 0, 90, 180 ou 270. */
      rotation: number;
      flipH: boolean;
      grayscale: boolean;
    };

export interface EditDocument {
  pages: { page: number; objects: EditObject[] }[];
}

export interface EditOutcome extends JobOutcome {
  replacedChars: number;
  rebuilt: boolean;
}

export function isCommandError(e: unknown): e is CommandError {
  return typeof e === "object" && e !== null && "code" in e && "detail" in e;
}

/** Normalise n'importe quelle erreur en `CommandError`. */
export function toCommandError(e: unknown): CommandError {
  if (isCommandError(e)) return e;
  return { code: "unknown", file: null, detail: String(e) };
}

export const api = {
  pickFiles: (kind: FileKind, multiple: boolean) => invoke<FileInfo[]>("pick_files", { kind, multiple }),
  startupFiles: () => invoke<FileInfo[]>("startup_files"),
  inspectPdf: (input: PdfInput) => invoke<PdfInfo>("inspect_pdf", { input }),
  renderPage: (input: PdfInput, page: number, width: number) =>
    invoke<ArrayBuffer>("render_page", { input, page, width: Math.round(width) }),
  imageThumbnail: (path: string, size: number) => invoke<ArrayBuffer>("image_thumbnail", { path, size }),
  cancelJob: (jobId: string) => invoke<void>("cancel_job", { jobId }),
  revealInFolder: (path: string) => invoke<void>("reveal_in_folder", { path }),
  appInfo: () => invoke<{ version: string }>("app_info"),

  mergePdfs: (jobId: string, inputs: PdfInput[], suggestedName: string, onProgress: Channel<Progress>) =>
    invoke<JobOutcome>("merge_pdfs", { jobId, inputs, suggestedName, onProgress }),
  organizePdf: (jobId: string, inputs: PdfInput[], pages: PageSpec[], suggestedName: string, onProgress: Channel<Progress>) =>
    invoke<JobOutcome>("organize_pdf", { jobId, inputs, pages, suggestedName, onProgress }),
  splitPdf: (jobId: string, input: PdfInput, mode: SplitMode, onProgress: Channel<Progress>) =>
    invoke<JobOutcome>("split_pdf", { jobId, input, mode, onProgress }),
  compressPdfs: (
    jobId: string,
    inputs: PdfInput[],
    level: CompressionLevel,
    suffix: string,
    onProgress: Channel<Progress>,
  ) => invoke<JobOutcome>("compress_pdfs", { jobId, inputs, level, suffix, onProgress }),
  pdfToJpg: (jobId: string, inputs: PdfInput[], mode: ImageExport, onProgress: Channel<Progress>) =>
    invoke<JobOutcome>("pdf_to_jpg", { jobId, inputs, mode, onProgress }),
  jpgToPdf: (
    jobId: string,
    inputs: ImageInput[],
    options: ImagesToPdfOptions,
    mergeAll: boolean,
    suggestedName: string,
    onProgress: Channel<Progress>,
  ) => invoke<JobOutcome>("jpg_to_pdf", { jobId, inputs, options, mergeAll, suggestedName, onProgress }),
  editPdf: (jobId: string, input: PdfInput, edits: EditDocument, suffix: string, onProgress: Channel<Progress>) =>
    invoke<EditOutcome>("edit_pdf", { jobId, input, edits, suffix, onProgress }),
  officeStatus: () => invoke<OfficeInfo>("office_status"),
  officeToPdf: (jobId: string, inputs: string[], onProgress: Channel<Progress>) =>
    invoke<JobOutcome>("office_to_pdf_files", { jobId, inputs, onProgress }),
  pdfToOffice: (jobId: string, inputs: PdfInput[], format: OfficeFormat, onProgress: Channel<Progress>) =>
    invoke<PdfToOfficeOutcome>("pdf_to_office_files", { jobId, inputs, format, onProgress }),
};

export interface DragState {
  active: boolean;
}

export function onDrag(fn: (s: DragState) => void): Promise<UnlistenFn> {
  return listen<DragState>("hestia://drag", (e) => fn(e.payload));
}

export function onDrop(fn: (files: FileInfo[]) => void): Promise<UnlistenFn> {
  return listen<FileInfo[]>("hestia://drop", (e) => fn(e.payload));
}

export { Channel };
