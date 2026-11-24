// Catalogue des outils, dans l'ordre de l'écran d'accueil.

import type { FileKind } from "./api";
import type { Key } from "./i18n";
import type { IconName } from "./icons";

export type ToolId =
  | "merge"
  | "split"
  | "organize"
  | "compress"
  | "pdfToWord"
  | "pdfToPpt"
  | "pdfToExcel"
  | "wordToPdf"
  | "pptToPdf"
  | "excelToPdf"
  | "edit"
  | "pdfToJpg"
  | "jpgToPdf";

export type Category = "organize" | "optimize" | "convert" | "edit";

export interface ToolDef {
  id: ToolId;
  icon: IconName;
  category: Category;
  /** Disponible dans cette version. */
  ready: boolean;
  accepts: FileKind;
}

export const TOOLS: ToolDef[] = [
  { id: "merge", icon: "merge", category: "organize", ready: true, accepts: "pdf" },
  { id: "split", icon: "split", category: "organize", ready: true, accepts: "pdf" },
  { id: "organize", icon: "organize", category: "organize", ready: true, accepts: "pdf" },
  { id: "compress", icon: "compress", category: "optimize", ready: true, accepts: "pdf" },
  { id: "pdfToWord", icon: "pdfToWord", category: "convert", ready: true, accepts: "pdf" },
  { id: "pdfToPpt", icon: "pdfToPpt", category: "convert", ready: true, accepts: "pdf" },
  { id: "pdfToExcel", icon: "pdfToExcel", category: "convert", ready: true, accepts: "pdf" },
  { id: "wordToPdf", icon: "wordToPdf", category: "convert", ready: true, accepts: "word" },
  { id: "pptToPdf", icon: "pptToPdf", category: "convert", ready: true, accepts: "powerpoint" },
  { id: "excelToPdf", icon: "excelToPdf", category: "convert", ready: true, accepts: "excel" },
  { id: "edit", icon: "edit", category: "edit", ready: true, accepts: "pdf" },
  { id: "pdfToJpg", icon: "pdfToJpg", category: "convert", ready: true, accepts: "pdf" },
  { id: "jpgToPdf", icon: "jpgToPdf", category: "convert", ready: true, accepts: "image" },
];

export function toolById(id: ToolId): ToolDef {
  const tool = TOOLS.find((t) => t.id === id);
  if (!tool) throw new Error(`outil inconnu : ${id}`);
  return tool;
}

export const titleKey = (id: ToolId) => `tool.${id}.title` as Key;
export const descKey = (id: ToolId) => `tool.${id}.desc` as Key;
