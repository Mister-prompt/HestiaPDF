// Modèle de l'éditeur « Modifier PDF ».
// Coordonnées en points PDF, origine en haut à gauche de la page affichée.
// Les constantes et la géométrie doivent rester identiques à
// crates/hestia-core/src/edit.rs pour que le PDF ressemble à l'aperçu.

import type { EditObject as WireObject } from "../../api";
import { effectiveStyle, fontCss } from "./fonts";

export const TEXT_LINE_HEIGHT = 1.25;
export const TEXT_BASELINE = 0.95;

export type Tool = "select" | "text" | "image" | "rect" | "ellipse" | "line" | "arrow" | "ink" | "highlight" | "measure";

interface Base {
  id: string;
  opacity: number;
}

export interface TextObj extends Base {
  type: "text";
  x: number;
  y: number;
  size: number;
  /** Identifiant de police (voir fonts.ts). */
  font: string;
  bold: boolean;
  italic: boolean;
  color: string;
  lines: string[];
}

export interface ShapeObj extends Base {
  type: "rect" | "ellipse";
  x: number;
  y: number;
  w: number;
  h: number;
  stroke: string | null;
  fill: string | null;
  strokeWidth: number;
}

export interface LineObj extends Base {
  type: "line";
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  color: string;
  width: number;
  arrow: boolean;
}

export interface InkObj extends Base {
  type: "ink";
  points: [number, number][];
  color: string;
  width: number;
}

export interface ImageObj extends Base {
  type: "image";
  x: number;
  y: number;
  w: number;
  h: number;
  path: string;
  /** URL d'affichage (miniature), jamais envoyée à Rust. */
  url: string;
  /** Rotation horaire (0, 90, 180, 270), appliquée après le miroir. Le cadre (x, y, w, h) est celui affiché. */
  rotation: number;
  flipH: boolean;
  grayscale: boolean;
}

/** Mesure entre deux points ; écrite dans le PDF seulement si `keep`. */
export interface MeasureObj extends Base {
  type: "measure";
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  color: string;
  width: number;
  keep: boolean;
}

export type EditorObject = TextObj | ShapeObj | LineObj | InkObj | ImageObj | MeasureObj;

// --- Mesures -----------------------------------------------------------------

export type MeasureUnit = "cm" | "in";

/** Taille de l'étiquette d'une mesure, en points. */
export const MEASURE_LABEL_SIZE = 10;

/** Longueur réelle sur la feuille imprimée à 100 % (1 pt = 1/72 de pouce). */
export function measureLength(obj: Pick<MeasureObj, "x1" | "y1" | "x2" | "y2">, unit: MeasureUnit): number {
  const inches = Math.hypot(obj.x2 - obj.x1, obj.y2 - obj.y1) / 72;
  return unit === "cm" ? inches * 2.54 : inches;
}

export function formatLength(value: number, unit: MeasureUnit, locale: string): string {
  const number = value.toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return `${number} ${unit === "cm" ? "cm" : "in"}`;
}

/** Géométrie d'une mesure : traits d'extrémité et cadre de l'étiquette. */
export function measureGeometry(obj: MeasureObj, label: string) {
  const dx = obj.x2 - obj.x1;
  const dy = obj.y2 - obj.y1;
  const len = Math.max(0.001, Math.hypot(dx, dy));
  // Normale orientée vers le haut (ou vers la droite pour un trait vertical).
  let nx = -dy / len;
  let ny = dx / len;
  if (ny > 0 || (Math.abs(ny) < 1e-6 && nx < 0)) {
    nx = -nx;
    ny = -ny;
  }
  const tick = Math.max(4, obj.width * 3);
  const ticks: [number, number, number, number][] = [
    [obj.x1 + nx * tick, obj.y1 + ny * tick, obj.x1 - nx * tick, obj.y1 - ny * tick],
    [obj.x2 + nx * tick, obj.y2 + ny * tick, obj.x2 - nx * tick, obj.y2 - ny * tick],
  ];
  const size = MEASURE_LABEL_SIZE;
  const textW = measureLine(label, { size, font: "helvetica", bold: false, italic: false });
  const padX = 3;
  const boxW = textW + 2 * padX;
  const boxH = size * TEXT_LINE_HEIGHT + 2;
  const gap = tick + 3;
  const cx = (obj.x1 + obj.x2) / 2 + nx * (gap + Math.abs(ny) * (boxH / 2) + Math.abs(nx) * (boxW / 2));
  const cy = (obj.y1 + obj.y2) / 2 + ny * (gap + Math.abs(ny) * (boxH / 2) + Math.abs(nx) * (boxW / 2));
  const box: Box = { x: cx - boxW / 2, y: cy - boxH / 2, w: boxW, h: boxH };
  return { ticks, box, textX: box.x + padX, textY: box.y + 1, size };
}

/** Texte de l'étiquette, fixé par l'éditeur (unité et langue choisies). */
let measureLabel = (obj: MeasureObj) => formatLength(measureLength(obj, "cm"), "cm", "fr-FR");

export function setMeasureLabel(fn: (obj: MeasureObj) => string): void {
  measureLabel = fn;
}

export const labelOf = (obj: MeasureObj) => measureLabel(obj);

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

let seq = 0;
export const newId = () => `o${Date.now().toString(36)}${(++seq).toString(36)}`;

// --- Texte -------------------------------------------------------------------

let measureCtx: CanvasRenderingContext2D | null = null;

/**
 * Largeur (pt) d'une ligne, sans crénage ni ligatures : exactement la somme
 * des chasses, comme le texte écrit dans le PDF.
 */
export function measureLine(text: string, obj: Pick<TextObj, "size" | "font" | "bold" | "italic">): number {
  measureCtx ??= document.createElement("canvas").getContext("2d");
  if (!measureCtx) return text.length * obj.size * 0.55;
  const s = effectiveStyle(obj.font, obj.bold, obj.italic);
  measureCtx.font = `${s.italic ? "italic " : ""}${s.bold ? "bold " : ""}${obj.size}px ${fontCss(obj.font)}`;
  measureCtx.fontKerning = "none";
  measureCtx.textRendering = "optimizeSpeed";
  return measureCtx.measureText(text).width;
}

export function textBox(obj: TextObj): Box {
  const width = Math.max(obj.size * 0.5, ...obj.lines.map((l) => measureLine(l, obj)));
  return { x: obj.x, y: obj.y, w: width, h: Math.max(1, obj.lines.length) * TEXT_LINE_HEIGHT * obj.size };
}

export const baselineOf = (obj: TextObj, line: number) => obj.y + (line * TEXT_LINE_HEIGHT + TEXT_BASELINE) * obj.size;

// --- Géométrie ---------------------------------------------------------------

/** Pointe de flèche (identique à `arrow_head` côté Rust). */
export function arrowHead(ax: number, ay: number, bx: number, by: number, width: number) {
  const dx = bx - ax;
  const dy = by - ay;
  const len = Math.max(0.001, Math.hypot(dx, dy));
  const ux = dx / len;
  const uy = dy / len;
  const headLen = Math.min(Math.max(width * 4, 8), len);
  const half = headLen * 0.55;
  const base: [number, number] = [bx - ux * headLen, by - uy * headLen];
  return {
    base,
    left: [base[0] - uy * half, base[1] + ux * half] as [number, number],
    right: [base[0] + uy * half, base[1] - ux * half] as [number, number],
  };
}

export function bounds(obj: EditorObject): Box {
  switch (obj.type) {
    case "text":
      return textBox(obj);
    case "rect":
    case "ellipse":
    case "image":
      return { x: obj.x, y: obj.y, w: obj.w, h: obj.h };
    case "measure": {
      const { box, ticks } = measureGeometry(obj, labelOf(obj));
      const xs = [box.x, box.x + box.w, ...ticks.flatMap((t) => [t[0], t[2]])];
      const ys = [box.y, box.y + box.h, ...ticks.flatMap((t) => [t[1], t[3]])];
      return { x: Math.min(...xs), y: Math.min(...ys), w: Math.max(...xs) - Math.min(...xs), h: Math.max(...ys) - Math.min(...ys) };
    }
    case "line": {
      const pad = obj.width / 2;
      const x = Math.min(obj.x1, obj.x2) - pad;
      const y = Math.min(obj.y1, obj.y2) - pad;
      return { x, y, w: Math.abs(obj.x2 - obj.x1) + 2 * pad, h: Math.abs(obj.y2 - obj.y1) + 2 * pad };
    }
    case "ink": {
      const xs = obj.points.map((p) => p[0]);
      const ys = obj.points.map((p) => p[1]);
      const pad = obj.width / 2;
      const x = Math.min(...xs) - pad;
      const y = Math.min(...ys) - pad;
      return { x, y, w: Math.max(...xs) - Math.min(...xs) + 2 * pad, h: Math.max(...ys) - Math.min(...ys) + 2 * pad };
    }
  }
}

function distToSegment(px: number, py: number, ax: number, ay: number, bx: number, by: number): number {
  const dx = bx - ax;
  const dy = by - ay;
  const l2 = dx * dx + dy * dy;
  const t = l2 === 0 ? 0 : Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / l2));
  return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
}

/** L'objet est-il sous le point (x, y) ? `tol` en points. */
export function hit(obj: EditorObject, x: number, y: number, tol: number): boolean {
  if (obj.type === "line") return distToSegment(x, y, obj.x1, obj.y1, obj.x2, obj.y2) <= obj.width / 2 + tol;
  if (obj.type === "measure") {
    if (distToSegment(x, y, obj.x1, obj.y1, obj.x2, obj.y2) <= obj.width / 2 + tol + 2) return true;
    const { box } = measureGeometry(obj, labelOf(obj));
    return x >= box.x - tol && x <= box.x + box.w + tol && y >= box.y - tol && y <= box.y + box.h + tol;
  }
  if (obj.type === "ink") {
    const pts = obj.points;
    if (pts.length === 1) return Math.hypot(x - pts[0]![0], y - pts[0]![1]) <= obj.width / 2 + tol;
    for (let i = 1; i < pts.length; i++) {
      if (distToSegment(x, y, pts[i - 1]![0], pts[i - 1]![1], pts[i]![0], pts[i]![1]) <= obj.width / 2 + tol) return true;
    }
    return false;
  }
  const b = bounds(obj);
  return x >= b.x - tol && x <= b.x + b.w + tol && y >= b.y - tol && y <= b.y + b.h + tol;
}

/** Déplace un objet de (dx, dy). */
export function translate(obj: EditorObject, dx: number, dy: number): void {
  switch (obj.type) {
    case "line":
    case "measure":
      obj.x1 += dx;
      obj.x2 += dx;
      obj.y1 += dy;
      obj.y2 += dy;
      break;
    case "ink":
      obj.points = obj.points.map(([px, py]) => [px + dx, py + dy]);
      break;
    default:
      obj.x += dx;
      obj.y += dy;
  }
}

/** Simplifie un tracé à main levée (Ramer-Douglas-Peucker). */
export function simplify(points: [number, number][], tolerance: number): [number, number][] {
  if (points.length < 3) return points;
  const keep = new Uint8Array(points.length);
  keep[0] = keep[points.length - 1] = 1;
  const stack: [number, number][] = [[0, points.length - 1]];
  while (stack.length) {
    const [a, b] = stack.pop()!;
    let maxD = 0;
    let idx = -1;
    for (let i = a + 1; i < b; i++) {
      const d = distToSegment(points[i]![0], points[i]![1], points[a]![0], points[a]![1], points[b]![0], points[b]![1]);
      if (d > maxD) {
        maxD = d;
        idx = i;
      }
    }
    if (idx >= 0 && maxD > tolerance) {
      keep[idx] = 1;
      stack.push([a, idx], [idx, b]);
    }
  }
  return points.filter((_, i) => keep[i]);
}

// --- Envoi à Rust ------------------------------------------------------------

const r2 = (v: number) => Math.round(v * 100) / 100;

/**
 * Objets envoyés à Rust pour un élément de l'éditeur. Une mesure devient
 * un trait, deux traits d'extrémité, un fond blanc et son étiquette, ou
 * rien du tout si elle n'est qu'affichée.
 */
export function toWire(obj: EditorObject): WireObject[] {
  if (obj.type !== "measure") return [toWireOne(obj)];
  if (!obj.keep) return [];
  const label = labelOf(obj);
  const g = measureGeometry(obj, label);
  const line = (x1: number, y1: number, x2: number, y2: number): WireObject => ({
    type: "line",
    x1: r2(x1),
    y1: r2(y1),
    x2: r2(x2),
    y2: r2(y2),
    color: obj.color,
    width: obj.width,
    opacity: obj.opacity,
    arrow: false,
  });
  return [
    line(obj.x1, obj.y1, obj.x2, obj.y2),
    ...g.ticks.map((t) => line(...t)),
    { type: "rect", x: r2(g.box.x), y: r2(g.box.y), w: r2(g.box.w), h: r2(g.box.h), stroke: null, fill: "#ffffff", strokeWidth: 0, opacity: 0.85 },
    {
      type: "text",
      x: r2(g.textX),
      y: r2(g.textY),
      size: g.size,
      font: "helvetica",
      bold: false,
      italic: false,
      color: obj.color,
      opacity: obj.opacity,
      lines: [label],
    },
  ];
}

function toWireOne(obj: Exclude<EditorObject, MeasureObj>): WireObject {
  switch (obj.type) {
    case "text":
      return {
        type: "text",
        x: r2(obj.x),
        y: r2(obj.y),
        size: r2(obj.size),
        font: obj.font,
        bold: obj.bold,
        italic: obj.italic,
        color: obj.color,
        opacity: obj.opacity,
        lines: obj.lines,
      };
    case "rect":
    case "ellipse":
      return {
        type: obj.type,
        x: r2(obj.x),
        y: r2(obj.y),
        w: r2(obj.w),
        h: r2(obj.h),
        stroke: obj.stroke,
        fill: obj.fill,
        strokeWidth: obj.strokeWidth,
        opacity: obj.opacity,
      };
    case "line":
      return {
        type: "line",
        x1: r2(obj.x1),
        y1: r2(obj.y1),
        x2: r2(obj.x2),
        y2: r2(obj.y2),
        color: obj.color,
        width: obj.width,
        opacity: obj.opacity,
        arrow: obj.arrow,
      };
    case "ink":
      return { type: "ink", points: obj.points.map(([x, y]) => [r2(x), r2(y)]), color: obj.color, width: obj.width, opacity: obj.opacity };
    case "image":
      return {
        type: "image",
        x: r2(obj.x),
        y: r2(obj.y),
        w: r2(obj.w),
        h: r2(obj.h),
        path: obj.path,
        opacity: obj.opacity,
        rotation: ((Math.round(obj.rotation / 90) % 4) + 4) % 4 * 90,
        flipH: obj.flipH,
        grayscale: obj.grayscale,
      };
  }
}
