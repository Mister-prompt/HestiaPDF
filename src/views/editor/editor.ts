// Éditeur visuel « Modifier PDF » : toutes les pages (rendu PDFium) les unes
// sous les autres, chacune avec un calque SVG où l'on ajoute, sélectionne,
// déplace et redimensionne les éléments.

import { api, toCommandError, type EditDocument, type PdfInput } from "../../api";
import { h, type Scope } from "../../dom";
import { getLang, t, type Key } from "../../i18n";
import { icon, type IconName } from "../../icons";
import { readSetting, writeSetting } from "../../storage";
import { errorText, toast } from "../../ui";
import { zoomField } from "../zoom-field";
import { effectiveStyle, ensureFont, fontCss, missingChars, onFontLoaded } from "./fonts";
import {
  arrowHead,
  baselineOf,
  bounds,
  formatLength,
  hit,
  labelOf,
  measureGeometry,
  measureLength,
  measureLine,
  newId,
  setMeasureLabel,
  simplify,
  TEXT_LINE_HEIGHT,
  toWire,
  translate,
  type Box,
  type EditorObject,
  type ImageObj,
  type InkObj,
  type LineObj,
  type MeasureObj,
  type MeasureUnit,
  type ShapeObj,
  type TextObj,
  type Tool,
} from "./model";

const SVG_NS = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number | null | undefined>,
  ...children: SVGElement[]
): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v !== null && v !== undefined) el.setAttribute(k, String(v));
  for (const c of children) el.appendChild(c);
  return el;
}

export interface PageSizePt {
  width: number;
  height: number;
}

/** Styles par défaut (mémorisés pendant la session). */
export const defaults = {
  text: { font: "helvetica", size: 16, color: "#111111", bold: false, italic: false, opacity: 1 },
  shape: { stroke: "#b91c1c" as string | null, fill: null as string | null, strokeWidth: 2, opacity: 1 },
  line: { color: "#b91c1c", width: 2, opacity: 1 },
  ink: { color: "#b91c1c", width: 3, opacity: 1 },
  highlight: { color: "#facc15", width: 14, opacity: 0.4 },
  measure: {
    color: "#1d4ed8",
    width: 1,
    opacity: 1,
    /** Les nouvelles mesures sont-elles ajoutées au PDF ? */
    keep: false,
    unit: (readSetting("measureUnit") === "in" ? "in" : "cm") as MeasureUnit,
  },
};

/** Longueur affichée d'une mesure, dans l'unité et la langue choisies. */
export function lengthText(obj: Pick<MeasureObj, "x1" | "y1" | "x2" | "y2">, unit = defaults.measure.unit): string {
  return formatLength(measureLength(obj, unit), unit, getLang() === "fr" ? "fr-FR" : "en-US");
}
setMeasureLabel((obj) => lengthText(obj));

/** Une page affichée : rendu PDFium + calque d'édition. */
interface PageView {
  index: number;
  el: HTMLElement;
  img: HTMLImageElement;
  layer: SVGSVGElement;
}

type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w" | "p1" | "p2";

type Drag =
  | { kind: "move"; id: string; sx: number; sy: number; orig: EditorObject; changed: boolean; wasSelected: boolean }
  | { kind: "resize"; id: string; handle: Handle; sx: number; sy: number; orig: EditorObject; box: Box; changed: boolean }
  | { kind: "create"; obj: ShapeObj | LineObj | MeasureObj; sx: number; sy: number }
  | { kind: "draw"; obj: InkObj };

interface EditorOptions {
  input: () => PdfInput;
  pages: PageSizePt[];
  scope: Scope;
  pageThumb: (index: number, width: number) => HTMLElement;
  /** Sélection ou outil changé : le panneau de propriétés doit être redessiné. */
  onSelectionChange: () => void;
  /** Contenu modifié : résumé et bouton à mettre à jour. */
  onEdit: () => void;
}

const TOOLS: { tool: Tool; icon: IconName; label: Key }[] = [
  { tool: "select", icon: "cursor", label: "edit.tool.select" },
  { tool: "text", icon: "font", label: "edit.tool.text" },
  { tool: "image", icon: "gallery", label: "edit.tool.image" },
  { tool: "rect", icon: "shapeRect", label: "edit.tool.rect" },
  { tool: "ellipse", icon: "shapeEllipse", label: "edit.tool.ellipse" },
  { tool: "line", icon: "shapeLine", label: "edit.tool.line" },
  { tool: "arrow", icon: "shapeArrow", label: "edit.tool.arrow" },
  { tool: "ink", icon: "pen", label: "edit.tool.ink" },
  { tool: "highlight", icon: "highlighter", label: "edit.tool.highlight" },
  { tool: "measure", icon: "ruler", label: "edit.tool.measure" },
];

export class Editor {
  readonly root: HTMLElement;
  tool: Tool = "select";
  pageIndex = 0;
  selectedId: string | null = null;
  dirty = false;

  private objects = new Map<number, EditorObject[]>();
  private undoStack: string[] = [];
  private redoStack: string[] = [];
  private zoom = 1;
  private fit = true;
  private drag: Drag | null = null;
  private editing: { obj: TextObj; area: HTMLTextAreaElement; isNew: boolean; before: string } | null = null;
  /** Dernier rendu de chaque page (largeur en pixels, URL). */
  private images = new Map<number, { key: string; url: string }>();
  /** Pages proches de la zone visible : leur rendu est chargé. */
  private nearby = new Set<number>();
  private imageTimer = 0;
  private imageGeneration = 0;
  /** Page la plus visible (numéro affiché, miniature en surbrillance). */
  private visiblePage = 0;

  private toolbar = h("div", { class: "editor-toolbar" });
  private strip = h("div", { class: "editor-pages" });
  private stage = h("div", { class: "editor-stage" });
  private column = h("div", { class: "editor-column" });
  private views: PageView[] = [];
  private pageLabel = h("span", { class: "editor-page-label" });
  private zoomField = zoomField(0.25, 4, (z) => this.setZoom(z));
  private toolButtons = new Map<Tool, HTMLButtonElement>();
  /** Annonces pour les lecteurs d'écran (sélection, création…). */
  private announcer = h("div", { class: "sr-only", attrs: { "aria-live": "polite" } });
  private coarsePointer = window.matchMedia("(pointer: coarse)").matches;
  private undoBtn!: HTMLButtonElement;
  private redoBtn!: HTMLButtonElement;

  constructor(private opts: EditorOptions) {
    const total = opts.pages.length;
    this.views = opts.pages.map((_, index) => {
      const layer = svg("svg", {
        class: "editor-layer",
        tabindex: index === 0 ? 0 : -1,
        role: "application",
        "aria-roledescription": t("edit.canvasRole"),
        "aria-label": `${t("viewer.page", { n: index + 1, total })}. ${t("edit.canvasHelp")}`,
      });
      const img = h("img", { alt: "", draggable: false });
      const el = h("div", { class: "editor-page", dataset: { page: String(index) } }, img, layer);
      layer.addEventListener("pointerdown", (e) => this.onPointerDown(e, index));
      // Empêche le clic sur la page de retirer le focus (saisie de texte immédiate).
      layer.addEventListener("mousedown", (e) => e.preventDefault());
      // Au clavier, la page qui reçoit le focus devient la page de travail.
      layer.addEventListener("focus", () => this.activate(index));
      return { index, el, img, layer };
    });
    this.column.append(...this.views.map((v) => v.el));
    this.stage.appendChild(this.column);
    this.buildToolbar();
    this.buildStrip();
    this.root = h("div", { class: "editor" }, this.toolbar, h("div", { class: "editor-body" }, this.strip, this.stage), this.announcer);

    // Rendus chargés seulement pour les pages proches de la zone visible.
    const watcher = new IntersectionObserver(
      (records) => {
        for (const r of records) {
          const index = Number((r.target as HTMLElement).dataset.page);
          if (r.isIntersecting) this.nearby.add(index);
          else this.nearby.delete(index);
        }
        this.scheduleImages();
      },
      { root: this.stage, rootMargin: "800px 0px" },
    );
    this.views.forEach((v) => watcher.observe(v.el));
    opts.scope.add(() => watcher.disconnect());
    // Calcul léger (positions des pages) : directement à chaque défilement.
    this.stage.addEventListener("scroll", () => this.updateVisiblePage(), { passive: true });

    // Une police vient d'arriver : l'aperçu et les cadres se recalent dessus.
    opts.scope.add(
      onFontLoaded(() => {
        this.renderAll();
        if (this.editing) this.styleTextArea();
      }),
    );
    const onKey = (e: KeyboardEvent) => this.onKey(e);
    document.addEventListener("keydown", onKey);
    opts.scope.add(() => document.removeEventListener("keydown", onKey));
    const resize = new ResizeObserver(() => {
      if (this.fit) this.fitWidth();
    });
    resize.observe(this.stage);
    opts.scope.add(() => resize.disconnect());
    this.stage.addEventListener(
      "wheel",
      (e) => {
        if (!e.ctrlKey) return;
        e.preventDefault();
        this.setZoom(this.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
      },
      { passive: false },
    );
    requestAnimationFrame(() => this.fitWidth());
  }

  // --- Données ------------------------------------------------------------------

  get pageSize(): PageSizePt {
    return this.opts.pages[this.pageIndex] ?? { width: 595, height: 842 };
  }

  /** Calque de la page de travail. */
  private get layer(): SVGSVGElement {
    return this.views[this.pageIndex]!.layer;
  }

  private get view(): PageView {
    return this.views[this.pageIndex]!;
  }

  private list(page = this.pageIndex): EditorObject[] {
    let list = this.objects.get(page);
    if (!list) {
      list = [];
      this.objects.set(page, list);
    }
    return list;
  }

  get selected(): EditorObject | undefined {
    return this.selectedId ? this.list().find((o) => o.id === this.selectedId) : undefined;
  }

  /** Éléments qui seront écrits dans le PDF (les mesures seulement affichées ne comptent pas). */
  count(): { objects: number; pages: number } {
    let objects = 0;
    let pages = 0;
    for (const list of this.objects.values()) {
      const n = list.filter((o) => o.type !== "measure" || o.keep).length;
      if (n) {
        objects += n;
        pages++;
      }
    }
    return { objects, pages };
  }

  allText(): string {
    return [...this.objects.values()]
      .flat()
      .filter((o): o is TextObj => o.type === "text")
      .map((o) => o.lines.join("\n"))
      .join("\n");
  }

  toEditDocument(): EditDocument {
    const pages = [...this.objects.entries()]
      .map(([page, list]) => ({ page, objects: list.flatMap(toWire) }))
      .filter((p) => p.objects.length > 0)
      .sort((a, b) => a.page - b.page);
    return { pages };
  }

  // --- Historique ---------------------------------------------------------------

  private snapshot(): string {
    return JSON.stringify([...this.objects.entries()]);
  }

  private restore(state: string): void {
    this.objects = new Map(JSON.parse(state) as [number, EditorObject[]][]);
    if (this.selectedId && !this.list().some((o) => o.id === this.selectedId)) this.selectedId = null;
  }

  /** À appeler juste avant une modification. */
  checkpoint(state = this.snapshot()): void {
    this.undoStack.push(state);
    if (this.undoStack.length > 100) this.undoStack.shift();
    this.redoStack = [];
    this.dirty = true;
    this.updateHistoryButtons();
  }

  undo(): void {
    this.commitText();
    const prev = this.undoStack.pop();
    if (prev === undefined) return;
    this.redoStack.push(this.snapshot());
    this.restore(prev);
    this.renderAll();
    this.afterChange(true);
  }

  redo(): void {
    const next = this.redoStack.pop();
    if (next === undefined) return;
    this.undoStack.push(this.snapshot());
    this.restore(next);
    this.renderAll();
    this.afterChange(true);
  }

  private afterChange(selection = false): void {
    this.dirty = true;
    this.render();
    this.updateStripBadges();
    this.updateHistoryButtons();
    if (selection) this.opts.onSelectionChange();
    this.opts.onEdit();
  }

  private updateHistoryButtons(): void {
    if (this.undoBtn) this.undoBtn.disabled = this.undoStack.length === 0;
    if (this.redoBtn) this.redoBtn.disabled = this.redoStack.length === 0;
  }

  // --- Modifications depuis le panneau de propriétés ----------------------------

  private pendingCheckpoint = true;

  /** Modifie l'objet sélectionné ; un seul point d'annulation par réglage. */
  updateSelected(mutate: (obj: EditorObject) => void, endOfGesture = true): void {
    const obj = this.selected;
    if (!obj) return;
    if (this.pendingCheckpoint) this.checkpoint();
    this.pendingCheckpoint = endOfGesture;
    mutate(obj);
    if (this.editing?.obj === obj) this.styleTextArea();
    this.afterChange();
  }

  /** Le prochain réglage créera un nouveau point d'annulation. */
  endGesture(): void {
    this.pendingCheckpoint = true;
  }

  deleteSelected(): void {
    const obj = this.selected;
    if (!obj) return;
    this.announce(t("edit.deletedAnnounce", { item: this.describe(obj) }));
    this.checkpoint();
    const list = this.list();
    list.splice(list.indexOf(obj), 1);
    this.selectedId = null;
    this.afterChange(true);
  }

  duplicateSelected(): void {
    const obj = this.selected;
    if (!obj) return;
    this.checkpoint();
    const copy = structuredClone(obj);
    copy.id = newId();
    translate(copy, 12, 12);
    this.list().push(copy);
    this.selectedId = copy.id;
    this.afterChange(true);
  }

  reorderSelected(toFront: boolean): void {
    const obj = this.selected;
    if (!obj) return;
    this.checkpoint();
    const list = this.list();
    list.splice(list.indexOf(obj), 1);
    if (toFront) list.push(obj);
    else list.unshift(obj);
    this.afterChange();
  }

  // --- Outils -------------------------------------------------------------------

  setTool(tool: Tool): void {
    this.commitText();
    if (tool === "image") {
      void this.insertImage();
      return;
    }
    this.applyTool(tool);
    this.render();
    this.opts.onSelectionChange();
  }

  private applyTool(tool: Tool): void {
    this.tool = tool;
    if (tool !== "select") this.selectedId = null;
    for (const [k, b] of this.toolButtons) b.setAttribute("aria-pressed", String(k === tool));
    this.column.dataset.tool = tool;
  }

  /** Unité des mesures (toutes les étiquettes sont mises à jour). */
  setMeasureUnit(unit: MeasureUnit): void {
    defaults.measure.unit = unit;
    writeSetting("measureUnit", unit);
    this.renderAll();
    this.opts.onSelectionChange();
  }

  /** Image sélectionnée : quart de tour, miroir horizontal ou vertical. */
  transformImage(kind: "left" | "right" | "flipH" | "flipV"): void {
    this.updateSelected((o) => {
      if (o.type !== "image") return;
      if (kind === "left" || kind === "right") {
        // Le cadre tourne autour de son centre.
        const cx = o.x + o.w / 2;
        const cy = o.y + o.h / 2;
        [o.w, o.h] = [o.h, o.w];
        o.x = cx - o.w / 2;
        o.y = cy - o.h / 2;
        o.rotation = (o.rotation + (kind === "right" ? 90 : 270)) % 360;
      } else {
        // Miroir de l'image affichée : M·R(θ) = R(−θ)·M ; vertical = horizontal + demi-tour.
        o.flipH = !o.flipH;
        o.rotation = ((kind === "flipH" ? 360 : 540) - o.rotation) % 360;
      }
    });
  }

  /** Caractères que les polices choisies ne savent pas dessiner. */
  unsupportedChars(): string[] {
    const bad = new Set<string>();
    for (const o of [...this.objects.values()].flat()) {
      if (o.type === "text") for (const ch of missingChars(o.lines.join(""), o.font, o.bold, o.italic)) bad.add(ch);
    }
    return [...bad];
  }

  private async insertImage(): Promise<void> {
    // L'image va sur la page affichée au moment du clic, même si l'on fait défiler entre-temps.
    const targetPage = this.visiblePage;
    try {
      const [file] = await api.pickFiles("image", false);
      if (!file) return;
      const bytes = await api.imageThumbnail(file.path, 1024);
      const url = this.opts.scope.objectUrl(new Blob([bytes], { type: "image/jpeg" }));
      const natural = await new Promise<{ w: number; h: number }>((resolve) => {
        const probe = new Image();
        probe.onload = () => resolve({ w: probe.naturalWidth || 1, h: probe.naturalHeight || 1 });
        probe.onerror = () => resolve({ w: 4, h: 3 });
        probe.src = url;
      });
      const { width, height } = this.opts.pages[targetPage] ?? this.pageSize;
      const w = Math.min(width * 0.45, natural.w);
      const hgt = (w * natural.h) / natural.w;
      const obj: ImageObj = {
        id: newId(),
        type: "image",
        x: (width - w) / 2,
        y: Math.max(0, (height - hgt) / 2),
        w,
        h: hgt,
        path: file.path,
        url,
        opacity: 1,
        rotation: 0,
        flipH: false,
        grayscale: false,
      };
      this.checkpoint();
      this.activate(targetPage);
      this.list(targetPage).push(obj);
      this.selectedId = obj.id;
      this.setTool("select");
      this.afterChange(true);
    } catch (e) {
      toast(errorText(toCommandError(e)), "error");
    }
  }

  // --- Barre d'outils, pages ------------------------------------------------------

  private buildToolbar(): void {
    const group = h("div", { class: "editor-tools", attrs: { role: "toolbar", "aria-label": t("edit.tools") } });
    for (const def of TOOLS) {
      const b = h(
        "button",
        {
          class: "tool-btn",
          type: "button",
          title: t(def.label),
          attrs: { "aria-pressed": String(def.tool === this.tool), "aria-label": t(def.label) },
          on: { click: () => this.setTool(def.tool) },
        },
        icon(def.icon, 20),
        h("span", { text: t(def.label) }),
      );
      this.toolButtons.set(def.tool, b);
      group.appendChild(b);
    }
    const iconBtn = (name: IconName, label: Key, fn: () => void) =>
      h("button", { class: "icon-btn", type: "button", title: t(label), attrs: { "aria-label": t(label) }, on: { click: fn } }, icon(name, 18));
    this.undoBtn = iconBtn("undo", "edit.undo", () => this.undo());
    this.redoBtn = iconBtn("redo", "edit.redo", () => this.redo());
    this.updateHistoryButtons();
    this.toolbar.append(
      group,
      h("span", { class: "header-spacer" }),
      this.pageLabel,
      h("span", { class: "sep" }),
      this.undoBtn,
      this.redoBtn,
      h("span", { class: "sep" }),
      iconBtn("minus", "viewer.zoomOut", () => this.setZoom(this.zoom / 1.2)),
      this.zoomField.el,
      iconBtn("plus", "viewer.zoomIn", () => this.setZoom(this.zoom * 1.2)),
    );
  }

  private buildStrip(): void {
    this.strip.replaceChildren();
    this.opts.pages.forEach((_, i) => {
      const tile = h(
        "button",
        {
          class: `editor-thumb${i === this.visiblePage ? " is-current" : ""}`,
          type: "button",
          dataset: { page: String(i) },
          on: { click: () => this.goToPage(i) },
        },
        this.opts.pageThumb(i, 180),
        h("span", { class: "editor-thumb-num", text: String(i + 1) }),
        h("span", { class: "editor-thumb-badge", dataset: { badge: String(i) } }),
      );
      this.strip.appendChild(tile);
    });
    this.updateStripBadges();
  }

  private updateStripBadges(): void {
    this.strip.querySelectorAll<HTMLElement>("[data-badge]").forEach((el) => {
      // Les mesures seulement affichées ne vont pas dans le PDF : pas comptées.
      const n = (this.objects.get(Number(el.dataset.badge)) ?? []).filter((o) => o.type !== "measure" || o.keep).length;
      el.textContent = n ? String(n) : "";
      el.style.display = n ? "" : "none";
    });
  }

  /** Fait défiler jusqu'à la page `index`, qui devient la page de travail. */
  goToPage(index: number): void {
    const view = this.views[index];
    if (!view) return;
    this.stage.scrollTo({ top: Math.max(0, view.el.offsetTop - 16) });
    this.setVisiblePage(index);
    this.activate(index);
  }

  /**
   * Page de travail : celle où se trouvent la sélection et le texte en cours.
   * Changer de page valide le texte en cours et désélectionne.
   */
  private activate(index: number): void {
    if (index === this.pageIndex || !this.views[index]) return;
    this.commitText();
    const previous = this.pageIndex;
    const hadSelection = this.selectedId !== null;
    this.selectedId = null;
    this.pageIndex = index;
    this.views.forEach((v) => v.layer.setAttribute("tabindex", v.index === index ? "0" : "-1"));
    this.renderPage(previous);
    this.renderPage(index);
    if (hadSelection) this.opts.onSelectionChange();
  }

  private setVisiblePage(index: number): void {
    if (index === this.visiblePage) return;
    this.visiblePage = index;
    this.pageLabel.textContent = t("viewer.page", { n: index + 1, total: this.opts.pages.length });
    this.strip.querySelectorAll<HTMLElement>(".editor-thumb").forEach((el) => el.classList.toggle("is-current", el.dataset.page === String(index)));
    this.strip.querySelector(".is-current")?.scrollIntoView({ block: "nearest" });
  }

  /** Après un défilement : page la plus visible ; elle devient la page de travail si rien n'est en cours ailleurs. */
  private updateVisiblePage(): void {
    const probe = this.stage.scrollTop + this.stage.clientHeight * 0.35;
    let index = 0;
    for (const v of this.views) {
      if (v.el.offsetTop <= probe) index = v.index;
      else break;
    }
    this.setVisiblePage(index);
    if (!this.selectedId && !this.editing && !this.drag && index !== this.pageIndex) this.activate(index);
  }

  // --- Zoom et rendu des pages ----------------------------------------------------

  private fitWidth(): void {
    const available = Math.max(200, this.stage.clientWidth - 64);
    const widest = Math.max(1, ...this.opts.pages.map((p) => p.width));
    this.zoom = Math.min(1.6, Math.max(0.25, available / widest));
    this.fit = true;
    this.layout();
  }

  private setZoom(z: number): void {
    // Le point au centre de la vue y reste après le zoom.
    const s = this.stage;
    const rx = (s.scrollLeft + s.clientWidth / 2) / Math.max(1, s.scrollWidth);
    const ry = (s.scrollTop + s.clientHeight / 2) / Math.max(1, s.scrollHeight);
    this.zoom = Math.min(4, Math.max(0.25, z));
    this.fit = false;
    this.layout();
    s.scrollLeft = rx * s.scrollWidth - s.clientWidth / 2;
    s.scrollTop = ry * s.scrollHeight - s.clientHeight / 2;
  }

  private layout(): void {
    for (const v of this.views) {
      const { width, height } = this.opts.pages[v.index]!;
      v.el.style.width = `${width * this.zoom}px`;
      v.el.style.height = `${height * this.zoom}px`;
      v.layer.setAttribute("viewBox", `0 0 ${width} ${height}`);
    }
    this.zoomField.set(this.zoom);
    this.pageLabel.textContent = t("viewer.page", { n: this.visiblePage + 1, total: this.opts.pages.length });
    this.scheduleImages();
    this.renderAll();
    if (this.editing) this.styleTextArea();
  }

  /** Charge (ou recharge à la bonne résolution) le rendu des pages proches, la plus visible d'abord. */
  private scheduleImages(): void {
    window.clearTimeout(this.imageTimer);
    this.imageTimer = window.setTimeout(() => void this.loadImages(++this.imageGeneration), 120);
  }

  private async loadImages(generation: number): Promise<void> {
    const order = [...this.nearby].sort((a, b) => Math.abs(a - this.visiblePage) - Math.abs(b - this.visiblePage));
    for (const index of order) {
      // Zoom ou défilement entre-temps : une nouvelle série a pris le relais.
      if (generation !== this.imageGeneration || this.opts.scope.isDisposed) return;
      await this.loadImage(this.views[index]!);
    }
  }

  private async loadImage(view: PageView): Promise<void> {
    const size = this.opts.pages[view.index]!;
    const target = Math.min(3000, Math.max(300, Math.ceil((size.width * this.zoom * (window.devicePixelRatio || 1)) / 200) * 200));
    const key = String(target);
    const current = this.images.get(view.index);
    if (current?.key === key) return;
    // Garde l'image actuelle (étirée) pendant le rendu de la nouvelle.
    try {
      const bytes = await api.renderPage(this.opts.input(), view.index, target);
      if (this.opts.scope.isDisposed) return;
      const url = this.opts.scope.objectUrl(new Blob([bytes], { type: "image/jpeg" }));
      this.images.set(view.index, { key, url });
      view.img.src = url;
      if (current) URL.revokeObjectURL(current.url);
    } catch (e) {
      if (view.index === this.visiblePage) toast(errorText(toCommandError(e)), "error");
    }
  }

  // --- Dessin des calques ---------------------------------------------------------

  /** Redessine la page de travail (le seul calque qui change pendant un geste). */
  render(): void {
    this.renderPage(this.pageIndex);
  }

  private renderAll(): void {
    for (const v of this.views) this.renderPage(v.index);
  }

  private renderPage(index: number): void {
    const view = this.views[index];
    if (!view) return;
    const children: SVGElement[] = [];
    for (const obj of this.objects.get(index) ?? []) {
      if (this.editing?.obj.id === obj.id) continue;
      children.push(this.renderObject(obj));
    }
    const sel = index === this.pageIndex ? this.selected : undefined;
    if (sel && this.tool === "select" && !this.editing) children.push(this.renderSelection(sel));
    view.layer.replaceChildren(...children);
  }

  private renderObject(obj: EditorObject): SVGElement {
    const common = { opacity: obj.opacity < 1 ? obj.opacity : null, "data-id": obj.id };
    switch (obj.type) {
      case "text": {
        ensureFont(obj.font, obj.bold, obj.italic);
        const style = effectiveStyle(obj.font, obj.bold, obj.italic);
        const text = svg("text", {
          ...common,
          fill: obj.color,
          "font-family": fontCss(obj.font),
          "font-size": obj.size,
          "font-weight": style.bold ? "bold" : "normal",
          "font-style": style.italic ? "italic" : "normal",
          "xml:space": "preserve",
        });
        obj.lines.forEach((line, i) => {
          const tspan = svg("tspan", { x: obj.x, y: baselineOf(obj, i) });
          tspan.textContent = line || " ";
          text.appendChild(tspan);
        });
        return text;
      }
      case "rect":
        return svg("rect", {
          ...common,
          x: obj.x,
          y: obj.y,
          width: Math.max(0.1, obj.w),
          height: Math.max(0.1, obj.h),
          fill: obj.fill ?? "none",
          stroke: obj.stroke && obj.strokeWidth > 0 ? obj.stroke : "none",
          "stroke-width": obj.strokeWidth,
        });
      case "ellipse":
        return svg("ellipse", {
          ...common,
          cx: obj.x + obj.w / 2,
          cy: obj.y + obj.h / 2,
          rx: Math.max(0.1, obj.w / 2),
          ry: Math.max(0.1, obj.h / 2),
          fill: obj.fill ?? "none",
          stroke: obj.stroke && obj.strokeWidth > 0 ? obj.stroke : "none",
          "stroke-width": obj.strokeWidth,
        });
      case "line": {
        const g = svg("g", { ...common, stroke: obj.color, fill: obj.color, "stroke-width": obj.width, "stroke-linecap": "round" });
        if (obj.arrow) {
          const head = arrowHead(obj.x1, obj.y1, obj.x2, obj.y2, obj.width);
          g.append(
            svg("line", { x1: obj.x1, y1: obj.y1, x2: head.base[0], y2: head.base[1] }),
            svg("polygon", { points: `${obj.x2},${obj.y2} ${head.left.join(",")} ${head.right.join(",")}`, stroke: "none" }),
          );
        } else {
          g.append(svg("line", { x1: obj.x1, y1: obj.y1, x2: obj.x2, y2: obj.y2 }));
        }
        return g;
      }
      case "measure": {
        // Trait de cote : pointillé tant qu'elle n'est qu'affichée, plein si elle va dans le PDF.
        const label = labelOf(obj);
        const geo = measureGeometry(obj, label);
        const g = svg("g", { ...common, class: `editor-measure${obj.keep ? " is-kept" : ""}` });
        const lines = svg("g", {
          stroke: obj.color,
          "stroke-width": obj.width,
          "stroke-linecap": "round",
          "stroke-dasharray": obj.keep ? null : `${obj.width * 4} ${obj.width * 3}`,
        });
        lines.append(svg("line", { x1: obj.x1, y1: obj.y1, x2: obj.x2, y2: obj.y2 }));
        for (const [x1, y1, x2, y2] of geo.ticks) lines.append(svg("line", { x1, y1, x2, y2, "stroke-dasharray": "none" }));
        const text = svg("text", {
          x: geo.textX,
          y: geo.textY + 0.95 * geo.size,
          fill: obj.color,
          "font-family": fontCss("helvetica"),
          "font-size": geo.size,
          "xml:space": "preserve",
        });
        text.textContent = label;
        g.append(
          lines,
          svg("rect", { x: geo.box.x, y: geo.box.y, width: geo.box.w, height: geo.box.h, rx: 2, fill: "#ffffff", "fill-opacity": 0.85 }),
          text,
        );
        return g;
      }
      case "ink": {
        const pts = obj.points.length === 1 ? [obj.points[0]!, obj.points[0]!] : obj.points;
        return svg("polyline", {
          ...common,
          points: pts.map((p) => p.join(",")).join(" "),
          fill: "none",
          stroke: obj.color,
          "stroke-width": obj.width,
          "stroke-linecap": "round",
          "stroke-linejoin": "round",
        });
      }
      case "image": {
        // Miroir puis rotation, autour du centre du cadre affiché (comme dans le PDF).
        const quarter = obj.rotation % 180 !== 0;
        const iw = quarter ? obj.h : obj.w;
        const ih = quarter ? obj.w : obj.h;
        return svg("image", {
          ...common,
          href: obj.url,
          x: -iw / 2,
          y: -ih / 2,
          width: iw,
          height: ih,
          preserveAspectRatio: "none",
          transform: `translate(${obj.x + obj.w / 2} ${obj.y + obj.h / 2}) rotate(${obj.rotation}) scale(${obj.flipH ? -1 : 1} 1)`,
          // Luminance Rec. 709, comme la conversion faite par Rust.
          style: obj.grayscale ? "filter: grayscale(1)" : null,
        });
      }
    }
  }

  private handleSize(): number {
    // Au doigt, des poignées assez grandes pour être attrapées.
    return (this.coarsePointer ? 22 : 9) / this.zoom;
  }

  private handlesFor(obj: EditorObject): { handle: Handle; x: number; y: number }[] {
    // Texte : la taille se règle uniquement par la taille de police.
    if (obj.type === "text") return [];
    if (obj.type === "line" || obj.type === "measure") {
      return [
        { handle: "p1", x: obj.x1, y: obj.y1 },
        { handle: "p2", x: obj.x2, y: obj.y2 },
      ];
    }
    const b = bounds(obj);
    const corners: { handle: Handle; x: number; y: number }[] = [
      { handle: "nw", x: b.x, y: b.y },
      { handle: "ne", x: b.x + b.w, y: b.y },
      { handle: "se", x: b.x + b.w, y: b.y + b.h },
      { handle: "sw", x: b.x, y: b.y + b.h },
    ];
    if (obj.type === "rect" || obj.type === "ellipse") {
      corners.push(
        { handle: "n", x: b.x + b.w / 2, y: b.y },
        { handle: "e", x: b.x + b.w, y: b.y + b.h / 2 },
        { handle: "s", x: b.x + b.w / 2, y: b.y + b.h },
        { handle: "w", x: b.x, y: b.y + b.h / 2 },
      );
    }
    return corners;
  }

  private renderSelection(obj: EditorObject): SVGElement {
    const g = svg("g", { class: "editor-selection" });
    const pad = 3 / this.zoom;
    if (obj.type !== "line" && obj.type !== "measure") {
      const b = bounds(obj);
      g.appendChild(
        svg("rect", {
          class: "sel-box",
          x: b.x - pad,
          y: b.y - pad,
          width: b.w + 2 * pad,
          height: b.h + 2 * pad,
          "stroke-width": 1.5 / this.zoom,
          "stroke-dasharray": `${5 / this.zoom} ${4 / this.zoom}`,
        }),
      );
    }
    const s = this.handleSize();
    for (const hd of this.handlesFor(obj)) {
      g.appendChild(
        svg("rect", {
          class: `sel-handle h-${hd.handle}`,
          "data-handle": hd.handle,
          x: hd.x - s / 2,
          y: hd.y - s / 2,
          width: s,
          height: s,
          rx: s / 4,
          "stroke-width": 1.5 / this.zoom,
        }),
      );
    }
    return g;
  }

  // --- Souris -----------------------------------------------------------------------

  private toPage(e: PointerEvent | MouseEvent): { x: number; y: number } {
    const r = this.layer.getBoundingClientRect();
    return {
      x: ((e.clientX - r.left) / r.width) * this.pageSize.width,
      y: ((e.clientY - r.top) / r.height) * this.pageSize.height,
    };
  }

  private objectAt(x: number, y: number): EditorObject | undefined {
    const list = this.list();
    const tol = 4 / this.zoom;
    for (let i = list.length - 1; i >= 0; i--) if (hit(list[i]!, x, y, tol)) return list[i];
    return undefined;
  }

  private onPointerDown(e: PointerEvent, page: number): void {
    if (e.button !== 0) return;
    if (page !== this.pageIndex) {
      // Clic sur une autre page : elle devient la page de travail.
      const wasEditing = !!this.editing;
      this.activate(page);
      if (wasEditing && this.tool === "text") return;
    }
    if (this.editing) {
      this.commitText();
      if (this.tool === "text") return;
    }
    const p = this.toPage(e);
    const target = e.target as Element;
    const layer = this.layer;
    layer.setPointerCapture(e.pointerId);

    switch (this.tool) {
      case "select": {
        const handle = target.getAttribute("data-handle") as Handle | null;
        const sel = this.selected;
        if (handle && sel) {
          this.drag = { kind: "resize", id: sel.id, handle, sx: p.x, sy: p.y, orig: structuredClone(sel), box: bounds(sel), changed: false };
          break;
        }
        const obj = this.objectAt(p.x, p.y);
        if (obj) {
          const wasSelected = this.selectedId === obj.id;
          if (!wasSelected) {
            this.selectedId = obj.id;
            this.opts.onSelectionChange();
          }
          this.drag = { kind: "move", id: obj.id, sx: p.x, sy: p.y, orig: structuredClone(obj), changed: false, wasSelected };
        } else if (this.selectedId) {
          this.selectedId = null;
          this.opts.onSelectionChange();
        }
        this.render();
        break;
      }
      case "text": {
        // Cliquer sur un texte existant le rouvre ; ailleurs, nouveau texte.
        const existing = this.objectAt(p.x, p.y);
        if (existing?.type === "text") this.editText(existing, false);
        else this.createText(p.x, p.y);
        break;
      }
      case "rect":
      case "ellipse": {
        const d = defaults.shape;
        const obj: ShapeObj = { id: newId(), type: this.tool, x: p.x, y: p.y, w: 0, h: 0, stroke: d.stroke, fill: d.fill, strokeWidth: d.strokeWidth, opacity: d.opacity };
        this.checkpoint();
        this.list().push(obj);
        this.drag = { kind: "create", obj, sx: p.x, sy: p.y };
        break;
      }
      case "line":
      case "arrow": {
        const d = defaults.line;
        const obj: LineObj = { id: newId(), type: "line", x1: p.x, y1: p.y, x2: p.x, y2: p.y, color: d.color, width: d.width, opacity: d.opacity, arrow: this.tool === "arrow" };
        this.checkpoint();
        this.list().push(obj);
        this.drag = { kind: "create", obj, sx: p.x, sy: p.y };
        break;
      }
      case "measure": {
        const d = defaults.measure;
        const obj: MeasureObj = { id: newId(), type: "measure", x1: p.x, y1: p.y, x2: p.x, y2: p.y, color: d.color, width: d.width, opacity: d.opacity, keep: d.keep };
        this.checkpoint();
        this.list().push(obj);
        this.drag = { kind: "create", obj, sx: p.x, sy: p.y };
        this.render();
        break;
      }
      case "ink":
      case "highlight": {
        const d = this.tool === "ink" ? defaults.ink : defaults.highlight;
        const obj: InkObj = { id: newId(), type: "ink", points: [[p.x, p.y]], color: d.color, width: d.width, opacity: d.opacity };
        this.checkpoint();
        this.list().push(obj);
        this.drag = { kind: "draw", obj };
        this.render();
        break;
      }
      default:
        break;
    }

    const move = (ev: PointerEvent) => this.onPointerMove(ev);
    const up = (ev: PointerEvent) => {
      layer.removeEventListener("pointermove", move);
      layer.removeEventListener("pointerup", up);
      layer.removeEventListener("pointercancel", up);
      this.onPointerUp(ev);
    };
    layer.addEventListener("pointermove", move);
    layer.addEventListener("pointerup", up);
    layer.addEventListener("pointercancel", up);
  }

  private onPointerMove(e: PointerEvent): void {
    const d = this.drag;
    if (!d) return;
    const p = this.toPage(e);
    switch (d.kind) {
      case "move": {
        const obj = this.list().find((o) => o.id === d.id);
        if (!obj) return;
        const dx = p.x - d.sx;
        const dy = p.y - d.sy;
        if (!d.changed) {
          if (Math.hypot(dx, dy) * this.zoom < 3) return;
          this.checkpoint(this.snapshotWith(d.id, d.orig));
          d.changed = true;
        }
        Object.assign(obj, structuredClone(d.orig));
        translate(obj, dx, dy);
        break;
      }
      case "resize": {
        const obj = this.list().find((o) => o.id === d.id);
        if (!obj) return;
        if (!d.changed) {
          this.checkpoint(this.snapshotWith(d.id, d.orig));
          d.changed = true;
        }
        this.applyResize(obj, d, p.x, p.y, e.shiftKey);
        break;
      }
      case "create": {
        const o = d.obj;
        if (o.type === "line" || o.type === "measure") {
          o.x2 = p.x;
          o.y2 = p.y;
          if (e.shiftKey) {
            // Maj : ligne horizontale, verticale ou à 45°.
            const ang = Math.round(Math.atan2(p.y - o.y1, p.x - o.x1) / (Math.PI / 4)) * (Math.PI / 4);
            const len = Math.hypot(p.x - o.x1, p.y - o.y1);
            o.x2 = o.x1 + Math.cos(ang) * len;
            o.y2 = o.y1 + Math.sin(ang) * len;
          }
        } else {
          let w = p.x - d.sx;
          let hgt = p.y - d.sy;
          if (e.shiftKey) {
            const m = Math.max(Math.abs(w), Math.abs(hgt));
            w = Math.sign(w || 1) * m;
            hgt = Math.sign(hgt || 1) * m;
          }
          o.x = Math.min(d.sx, d.sx + w);
          o.y = Math.min(d.sy, d.sy + hgt);
          o.w = Math.abs(w);
          o.h = Math.abs(hgt);
        }
        break;
      }
      case "draw": {
        const last = d.obj.points[d.obj.points.length - 1]!;
        if (Math.hypot(p.x - last[0], p.y - last[1]) * this.zoom >= 1.5) d.obj.points.push([p.x, p.y]);
        break;
      }
    }
    this.render();
  }

  private onPointerUp(_e: PointerEvent): void {
    const d = this.drag;
    this.drag = null;
    if (!d) return;
    if (d.kind === "create") {
      const o = d.obj;
      if (o.type === "measure") {
        // Un simple clic ne mesure rien : on retire la mesure vide.
        if (Math.hypot(o.x2 - o.x1, o.y2 - o.y1) * this.zoom < 3) {
          const list = this.list();
          list.splice(list.indexOf(o), 1);
          this.undoStack.pop();
          this.updateHistoryButtons();
          this.render();
          return;
        }
        // L'outil reste actif pour enchaîner les mesures ; le panneau affiche la dernière.
        this.selectedId = o.id;
        this.afterChange(true);
        this.announce(t("edit.measure.announce", { length: lengthText(o) }));
        return;
      }
      // Simple clic : taille par défaut.
      if (o.type === "line") {
        if (Math.hypot(o.x2 - o.x1, o.y2 - o.y1) < 4) {
          o.x2 = o.x1 + 120;
          o.y2 = o.y1;
        }
      } else if (o.w < 4 || o.h < 4) {
        o.w = 140;
        o.h = 90;
        o.x -= 70;
        o.y -= 45;
      }
      this.selectedId = o.id;
      this.setTool("select");
      this.afterChange(true);
      return;
    }
    if (d.kind === "draw") {
      d.obj.points = simplify(d.obj.points, 0.35);
      this.afterChange();
      return;
    }
    if (d.changed) {
      this.afterChange();
      return;
    }
    // Nouveau clic sur un texte déjà sélectionné (ou double-clic) : on le modifie.
    if (d.kind === "move" && d.wasSelected) {
      const obj = this.list().find((o) => o.id === d.id);
      if (obj?.type === "text") this.editText(obj, false);
    }
  }

  /** État courant, mais avec l'objet `id` remis à sa version d'origine. */
  private snapshotWith(id: string, orig: EditorObject): string {
    const entries = [...this.objects.entries()].map(([k, list]) => [k, list.map((o) => (o.id === id ? orig : o))] as const);
    return JSON.stringify(entries);
  }

  private applyResize(obj: EditorObject, d: Extract<Drag, { kind: "resize" }>, px: number, py: number, shift: boolean): void {
    const o = d.orig;
    if ((obj.type === "line" && o.type === "line") || (obj.type === "measure" && o.type === "measure")) {
      if (d.handle === "p1") {
        obj.x1 = px;
        obj.y1 = py;
      } else {
        obj.x2 = px;
        obj.y2 = py;
      }
      return;
    }
    const b = d.box;
    const hd = d.handle;
    let x0 = b.x;
    let y0 = b.y;
    let x1 = b.x + b.w;
    let y1 = b.y + b.h;
    if (hd.includes("w")) x0 = Math.min(px, x1 - 4);
    if (hd.includes("e")) x1 = Math.max(px, x0 + 4);
    if (hd.includes("n")) y0 = Math.min(py, y1 - 4);
    if (hd.includes("s")) y1 = Math.max(py, y0 + 4);
    let w = x1 - x0;
    let hgt = y1 - y0;
    const corner = hd.length === 2;
    const keepRatio = (obj.type === "image" || shift) && corner;
    if (keepRatio && b.w > 0 && b.h > 0) {
      const scale = Math.max(w / b.w, hgt / b.h);
      w = b.w * scale;
      hgt = b.h * scale;
      if (hd.includes("w")) x0 = b.x + b.w - w;
      if (hd.includes("n")) y0 = b.y + b.h - hgt;
    }
    switch (obj.type) {
      case "rect":
      case "ellipse":
      case "image":
        obj.x = x0;
        obj.y = y0;
        obj.w = w;
        obj.h = hgt;
        break;
      case "ink": {
        if (o.type !== "ink") return;
        const sx = b.w > 0 ? w / b.w : 1;
        const sy = b.h > 0 ? hgt / b.h : 1;
        obj.points = o.points.map(([qx, qy]) => [x0 + (qx - b.x) * sx, y0 + (qy - b.y) * sy]);
        break;
      }
      default:
        break;
    }
  }

  // --- Texte : saisie directement sur la page ----------------------------------------

  private createText(x: number, y: number): void {
    const d = defaults.text;
    const obj: TextObj = {
      id: newId(),
      type: "text",
      x,
      y: Math.max(0, y - d.size * 0.6),
      size: d.size,
      font: d.font,
      bold: d.bold,
      italic: d.italic,
      color: d.color,
      opacity: d.opacity,
      lines: [""],
    };
    const before = this.snapshot();
    this.list().push(obj);
    this.selectedId = obj.id;
    this.editText(obj, true, before);
    this.opts.onSelectionChange();
  }

  editText(obj: TextObj, isNew: boolean, before = this.snapshot()): void {
    this.commitText();
    const area = h("textarea", {
      class: "editor-textarea",
      value: obj.lines.join("\n"),
      attrs: { spellcheck: "true", "aria-label": t("edit.tool.text") },
    });
    area.addEventListener("input", () => {
      obj.lines = area.value.split("\n");
      this.styleTextArea();
    });
    area.addEventListener("keydown", (ev) => {
      if (ev.key === "Escape" || (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey))) {
        ev.preventDefault();
        ev.stopPropagation();
        this.commitText();
      }
    });
    // Le focus part vers le panneau (taille, couleur…) : on valide sans le
    // redessiner, pour que le réglage cliqué reste sous le pointeur.
    area.addEventListener("blur", (ev) => {
      const to = ev.relatedTarget as Node | null;
      this.commitText({ quiet: !!to && !this.root.contains(to) });
    });
    this.editing = { obj, area, isNew, before };
    this.selectedId = obj.id;
    this.view.el.appendChild(area);
    this.styleTextArea();
    this.render();
    area.focus();
    if (isNew) area.select();
    else area.setSelectionRange(area.value.length, area.value.length);
  }

  private styleTextArea(): void {
    const ed = this.editing;
    if (!ed) return;
    const { obj, area } = ed;
    const z = this.zoom;
    const width = Math.max(obj.size * 3, ...obj.lines.map((l) => measureLine(l, obj))) + obj.size;
    const style = effectiveStyle(obj.font, obj.bold, obj.italic);
    Object.assign(area.style, {
      left: `${obj.x * z}px`,
      top: `${obj.y * z}px`,
      width: `${width * z}px`,
      height: `${Math.max(1, obj.lines.length) * TEXT_LINE_HEIGHT * obj.size * z + 4}px`,
      fontSize: `${obj.size * z}px`,
      lineHeight: String(TEXT_LINE_HEIGHT),
      fontFamily: fontCss(obj.font),
      fontWeight: style.bold ? "bold" : "normal",
      fontStyle: style.italic ? "italic" : "normal",
      color: obj.color,
      opacity: String(obj.opacity),
    });
  }

  /**
   * Termine la saisie de texte en cours (un texte vide est supprimé).
   * `quiet` : le panneau de propriétés n'est pas redessiné.
   */
  commitText({ quiet = false } = {}): void {
    const ed = this.editing;
    if (!ed) return;
    this.editing = null;
    const hadFocus = document.activeElement === ed.area;
    ed.area.remove();
    if (hadFocus) this.layer.focus({ preventScroll: true });
    const { obj } = ed;
    obj.lines = ed.area.value.replace(/\r/g, "").split("\n");
    while (obj.lines.length > 1 && obj.lines[obj.lines.length - 1] === "") obj.lines.pop();
    const empty = obj.lines.every((l) => l.trim() === "");
    const list = this.list();
    if (empty) {
      const idx = list.indexOf(obj);
      if (idx >= 0) list.splice(idx, 1);
      this.selectedId = null;
      if (!ed.isNew) this.checkpoint(ed.before);
      this.afterChange(true);
      return;
    }
    // Un point d'annulation pour toute la saisie.
    if (ed.before !== this.snapshot()) this.checkpoint(ed.before);
    if (this.tool === "text") this.applyTool("select");
    this.selectedId = obj.id;
    this.afterChange(!quiet);
  }

  // --- Clavier -------------------------------------------------------------------------

  private announce(text: string): void {
    // Vider puis remplir : le lecteur d'écran relit même un message identique.
    this.announcer.textContent = "";
    window.setTimeout(() => (this.announcer.textContent = text), 60);
  }

  private describe(obj: EditorObject): string {
    if (obj.type === "line") return t(obj.arrow ? "edit.tool.arrow" : "edit.tool.line");
    if (obj.type === "measure") return `${t("edit.tool.measure")} ${lengthText(obj)}`;
    if (obj.type === "ink") return t(obj.opacity < 0.7 && obj.width >= 10 ? "edit.tool.highlight" : "edit.tool.ink");
    if (obj.type === "text") return `${t("edit.tool.text")} « ${obj.lines.join(" ").slice(0, 40)} »`;
    return t(obj.type === "rect" ? "edit.tool.rect" : obj.type === "ellipse" ? "edit.tool.ellipse" : "edit.tool.image");
  }

  /** Tab / Maj+Tab sur la page : parcourt les éléments, puis rend la main. */
  private cycleSelection(backwards: boolean): boolean {
    const list = this.list();
    if (list.length === 0) return false;
    const current = list.findIndex((o) => o.id === this.selectedId);
    const next = current === -1 ? (backwards ? list.length - 1 : 0) : current + (backwards ? -1 : 1);
    if (next < 0 || next >= list.length) {
      this.selectedId = null;
      this.render();
      this.opts.onSelectionChange();
      return false;
    }
    if (this.tool !== "select") this.setTool("select");
    const obj = list[next]!;
    this.selectedId = obj.id;
    this.render();
    this.opts.onSelectionChange();
    this.announce(t("edit.selectedAnnounce", { item: this.describe(obj), n: next + 1, total: list.length }));
    return true;
  }

  /** Entrée sur la page avec un outil choisi : crée l'élément au centre. */
  private createAtCenter(): void {
    const { width, height } = this.pageSize;
    const cx = width / 2;
    const cy = height / 2;
    if (this.tool === "text") {
      this.createText(cx - 80, cy);
      return;
    }
    if (this.tool === "ink" || this.tool === "highlight") {
      this.announce(t("edit.inkNeedsPointer"));
      return;
    }
    let obj: EditorObject | null = null;
    if (this.tool === "rect" || this.tool === "ellipse") {
      const d = defaults.shape;
      obj = { id: newId(), type: this.tool, x: cx - 80, y: cy - 50, w: 160, h: 100, stroke: d.stroke, fill: d.fill, strokeWidth: d.strokeWidth, opacity: d.opacity };
    } else if (this.tool === "line" || this.tool === "arrow") {
      const d = defaults.line;
      obj = { id: newId(), type: "line", x1: cx - 80, y1: cy, x2: cx + 80, y2: cy, color: d.color, width: d.width, opacity: d.opacity, arrow: this.tool === "arrow" };
    } else if (this.tool === "measure") {
      // Au clavier : une mesure de 2 pouces à ajuster avec les poignées.
      const d = defaults.measure;
      obj = { id: newId(), type: "measure", x1: cx - 72, y1: cy, x2: cx + 72, y2: cy, color: d.color, width: d.width, opacity: d.opacity, keep: d.keep };
    }
    if (!obj) return;
    this.checkpoint();
    this.list().push(obj);
    this.selectedId = obj.id;
    this.setTool("select");
    this.afterChange(true);
    this.announce(t("edit.createdAnnounce", { item: this.describe(obj) }));
  }

  private onKey(e: KeyboardEvent): void {
    if (!this.root.isConnected || document.querySelector(".modal-backdrop")) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest("input, textarea, select")) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    const onCanvas = this.views.some((v) => v.layer === document.activeElement);
    if (onCanvas && key === "tab" && !ctrl) {
      if (this.cycleSelection(e.shiftKey)) e.preventDefault();
      return;
    }
    if (onCanvas && key === "enter" && this.tool !== "select") {
      e.preventDefault();
      this.createAtCenter();
      return;
    }
    if (ctrl && key === "z" && !e.shiftKey) {
      e.preventDefault();
      this.undo();
    } else if (ctrl && (key === "y" || (key === "z" && e.shiftKey))) {
      e.preventDefault();
      this.redo();
    } else if (ctrl && key === "d") {
      e.preventDefault();
      this.duplicateSelected();
    } else if (key === "delete" || key === "backspace") {
      if (this.selected) {
        e.preventDefault();
        this.deleteSelected();
      }
    } else if (key === "escape") {
      if (this.tool !== "select") this.setTool("select");
      else if (this.selectedId) {
        this.selectedId = null;
        this.render();
        this.opts.onSelectionChange();
      }
    } else if (key === "enter" && this.selected?.type === "text") {
      e.preventDefault();
      this.editText(this.selected, false);
    } else if (key.startsWith("arrow") && this.selected) {
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      const dx = key === "arrowleft" ? -step : key === "arrowright" ? step : 0;
      const dy = key === "arrowup" ? -step : key === "arrowdown" ? step : 0;
      this.updateSelected((o) => translate(o, dx, dy));
    } else if (key === "pagedown") {
      e.preventDefault();
      this.goToPage(this.visiblePage + 1);
    } else if (key === "pageup") {
      e.preventDefault();
      this.goToPage(this.visiblePage - 1);
    }
  }
}
