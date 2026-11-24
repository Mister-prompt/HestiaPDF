// Icônes : Papicons (Association Papillon, licence MIT) pour l'interface,
// icônes d'outils dessinées pour Hestia PDF dans le même style.
// Les SVG sont des fichiers de l'appli (contenu statique et vérifié).

import addSvg from "./assets/papicons/add.svg?raw";
import alertTriangleSvg from "./assets/papicons/alert-triangle.svg?raw";
import arrowLeftSvg from "./assets/papicons/arrow-left.svg?raw";
import arrowRightSvg from "./assets/papicons/arrow-right.svg?raw";
import checkSvg from "./assets/papicons/check.svg?raw";
import chevronLeftSvg from "./assets/papicons/chevron-left.svg?raw";
import chevronRightSvg from "./assets/papicons/chevron-right.svg?raw";
import crossSvg from "./assets/papicons/cross.svg?raw";
import folderSvg from "./assets/papicons/folder.svg?raw";
import gearsSvg from "./assets/papicons/gears.svg?raw";
import globeCrossSvg from "./assets/papicons/globe-cross.svg?raw";
import infoSvg from "./assets/papicons/info.svg?raw";
import lockSvg from "./assets/papicons/lock.svg?raw";
import minusSvg from "./assets/papicons/minus.svg?raw";
import moonSvg from "./assets/papicons/moon.svg?raw";
import paperSvg from "./assets/papicons/paper.svg?raw";
import plusSvg from "./assets/papicons/plus.svg?raw";
import sparklesSvg from "./assets/papicons/sparkles.svg?raw";
import sunSvg from "./assets/papicons/sun.svg?raw";
import trashSvg from "./assets/papicons/trash.svg?raw";
import unlockSvg from "./assets/papicons/unlock.svg?raw";
import galerySvg from "./assets/papicons/gallery.svg?raw";
import listSvg from "./assets/papicons/list.svg?raw";

import fontSvg from "./assets/papicons/font.svg?raw";
import penSvg from "./assets/papicons/pen.svg?raw";
import cursorSvg from "./assets/tools/cursor.svg?raw";
import shapeRectSvg from "./assets/tools/shape-rect.svg?raw";
import shapeEllipseSvg from "./assets/tools/shape-ellipse.svg?raw";
import shapeLineSvg from "./assets/tools/shape-line.svg?raw";
import shapeArrowSvg from "./assets/tools/shape-arrow.svg?raw";
import highlighterSvg from "./assets/tools/highlighter.svg?raw";
import rulerSvg from "./assets/tools/ruler.svg?raw";
import undoSvg from "./assets/tools/undo.svg?raw";
import redoSvg from "./assets/tools/redo.svg?raw";
import duplicateSvg from "./assets/tools/duplicate.svg?raw";
import layerUpSvg from "./assets/tools/layer-up.svg?raw";
import layerDownSvg from "./assets/tools/layer-down.svg?raw";
import compressSvg from "./assets/tools/compress.svg?raw";
import editSvg from "./assets/tools/edit.svg?raw";
import excelToPdfSvg from "./assets/tools/excel-to-pdf.svg?raw";
import gripSvg from "./assets/tools/grip.svg?raw";
import jpgToPdfSvg from "./assets/tools/jpg-to-pdf.svg?raw";
import mergeSvg from "./assets/tools/merge.svg?raw";
import pdfToExcelSvg from "./assets/tools/pdf-to-excel.svg?raw";
import pdfToJpgSvg from "./assets/tools/pdf-to-jpg.svg?raw";
import pdfToPptSvg from "./assets/tools/pdf-to-ppt.svg?raw";
import pdfToWordSvg from "./assets/tools/pdf-to-word.svg?raw";
import pptToPdfSvg from "./assets/tools/ppt-to-pdf.svg?raw";
import rotateSvg from "./assets/tools/rotate.svg?raw";
import rotateLeftSvg from "./assets/tools/rotate-left.svg?raw";
import rotateRightSvg from "./assets/tools/rotate-right.svg?raw";
import flipHSvg from "./assets/tools/flip-h.svg?raw";
import flipVSvg from "./assets/tools/flip-v.svg?raw";
import grayscaleSvg from "./assets/tools/grayscale.svg?raw";
import splitSvg from "./assets/tools/split.svg?raw";
import organizeSvg from "./assets/tools/organize.svg?raw";
import wordToPdfSvg from "./assets/tools/word-to-pdf.svg?raw";

const SOURCES = {
  add: addSvg,
  alert: alertTriangleSvg,
  arrowLeft: arrowLeftSvg,
  arrowRight: arrowRightSvg,
  check: checkSvg,
  chevronLeft: chevronLeftSvg,
  chevronRight: chevronRightSvg,
  cross: crossSvg,
  folder: folderSvg,
  gears: gearsSvg,
  offline: globeCrossSvg,
  info: infoSvg,
  lock: lockSvg,
  minus: minusSvg,
  moon: moonSvg,
  paper: paperSvg,
  plus: plusSvg,
  sparkles: sparklesSvg,
  sun: sunSvg,
  trash: trashSvg,
  unlock: unlockSvg,
  gallery: galerySvg,
  list: listSvg,
  rotate: rotateSvg,
  rotateLeft: rotateLeftSvg,
  rotateRight: rotateRightSvg,
  flipH: flipHSvg,
  flipV: flipVSvg,
  grayscale: grayscaleSvg,
  font: fontSvg,
  pen: penSvg,
  cursor: cursorSvg,
  shapeRect: shapeRectSvg,
  shapeEllipse: shapeEllipseSvg,
  shapeLine: shapeLineSvg,
  shapeArrow: shapeArrowSvg,
  highlighter: highlighterSvg,
  ruler: rulerSvg,
  undo: undoSvg,
  redo: redoSvg,
  duplicate: duplicateSvg,
  layerUp: layerUpSvg,
  layerDown: layerDownSvg,
  grip: gripSvg,
  merge: mergeSvg,
  split: splitSvg,
  organize: organizeSvg,
  compress: compressSvg,
  edit: editSvg,
  pdfToWord: pdfToWordSvg,
  pdfToPpt: pdfToPptSvg,
  pdfToExcel: pdfToExcelSvg,
  wordToPdf: wordToPdfSvg,
  pptToPdf: pptToPdfSvg,
  excelToPdf: excelToPdfSvg,
  pdfToJpg: pdfToJpgSvg,
  jpgToPdf: jpgToPdfSvg,
} as const;

export type IconName = keyof typeof SOURCES;

const parser = new DOMParser();
let uid = 0;

/** Crée une icône SVG (couleur = couleur du texte courant). */
export function icon(name: IconName, size = 20, extraClass = ""): SVGSVGElement {
  // Identifiants uniques pour les masques internes des icônes d'outils.
  const source = SOURCES[name].replaceAll("__ID__", `i${++uid}`);
  const doc = parser.parseFromString(source, "image/svg+xml");
  const svg = document.importNode(doc.documentElement, true) as unknown as SVGSVGElement;
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  svg.classList.add("icon");
  if (extraClass) svg.classList.add(extraClass);
  return svg;
}
