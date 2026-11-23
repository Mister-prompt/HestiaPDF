// Modèle de l'éditeur « Modifier PDF ».
// Coordonnées en points PDF, origine en haut à gauche de la page affichée.
// Les constantes et la géométrie doivent rester identiques à
// crates/hestia-core/src/edit.rs pour que le PDF ressemble à l'aperçu.

import type { EditObject as WireObject } from "../../api";
import { effectiveStyle, fontCss } from "./fonts";

export const TEXT_LINE_HEIGHT = 1.25;
export const TEXT_BASELINE = 0.95;

export type Tool = "select" | "text" | "image" | "rect" | "ellipse" | "line" | "arrow" | "ink" | "highlight";

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

export type EditorObject = TextObj | ShapeObj | LineObj | InkObj | ImageObj;

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

export function toWire(obj: EditorObject): WireObject {
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
