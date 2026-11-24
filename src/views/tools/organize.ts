// « Organiser » : toutes les pages d'un ou plusieurs PDF dans une grille.
// Glisser-déposer pour changer l'ordre, sélection multiple (Ctrl / Maj +
// clic, case à cocher, ou cadre tracé comme sur le bureau Windows),
// rotation, suppression, pages blanches, et aperçu du résultat.

import { api, type PageSpec } from "../../api";
import { h } from "../../dom";
import { t, tn } from "../../i18n";
import { icon, type IconName } from "../../icons";
import { errorText, trapFocus } from "../../ui";
import { entriesBlocker, infoBox, type Entry, type ToolConfig, type ToolPage } from "../tool";

/** Une page du document final. `entryId` nul : page blanche. */
interface Item {
  uid: number;
  entryId: string | null;
  page: number;
  /** Rotation ajoutée (0, 90, 180, 270). */
  rotation: number;
  /** Taille affichée en points, rotation propre du PDF comprise. */
  width: number;
  height: number;
}

/** Couleurs des fichiers sources (chiffre blanc lisible sur chacune). */
const FILE_COLORS = ["#1d4ed8", "#b91c1c", "#047857", "#6d28d9", "#b45309", "#be185d", "#0e7490", "#3f6212"];
const A4 = { width: 595, height: 842 };
/** Cadre d'une miniature, en pixels CSS. */
const FRAME = { width: 132, height: 172 };
const HISTORY_LIMIT = 100;

let uidSeq = 0;

class Organizer {
  items: Item[] = [];
  selected = new Set<number>();
  /** Point de départ des sélections Maj + clic. */
  private anchor: number | null = null;
  private known = new Set<string>();
  private undoStack: Item[][] = [];
  private redoStack: Item[][] = [];
  /** Glisser ou cadre en cours : on ne redessine pas la grille. */
  private interacting = false;
  private dirty = false;
  private focusUid: number | null = null;

  readonly root: HTMLElement;
  private head = h("div", { class: "org-head" });
  private grid = h("div", { class: "org-grid", attrs: { role: "listbox", "aria-multiselectable": "true" } });
  private previewCache = new Map<string, string>();

  constructor(readonly page: ToolPage) {
    this.root = h("div", { class: "org-root" }, this.head, this.grid);
    this.root.addEventListener("pointerdown", (e) => this.onPointerDown(e));
    const onKey = (e: KeyboardEvent) => this.onKey(e);
    document.addEventListener("keydown", onKey);
    page.scope.add(() => document.removeEventListener("keydown", onKey));
  }

  // --- Données --------------------------------------------------------------

  private entry(id: string | null): Entry | undefined {
    return id ? this.page.entries.find((e) => e.id === id) : undefined;
  }

  /** Ajoute les pages des PDF prêts, retire celles des PDF enlevés. */
  sync(): void {
    const ids = new Set(this.page.entries.map((e) => e.id));
    const gone = [...this.known].filter((id) => !ids.has(id));
    if (this.known.size > 0 && gone.length === this.known.size) {
      // Tous les fichiers précédents sont partis (« Recommencer », ou retirés
      // un à un) : nouveau document, pages blanches comprises.
      this.items = [];
      this.known.clear();
      this.selected.clear();
      this.anchor = null;
      this.undoStack = [];
      this.redoStack = [];
    } else if (gone.length) {
      this.items = this.items.filter((i) => i.entryId === null || ids.has(i.entryId));
      gone.forEach((id) => this.known.delete(id));
      // L'historique pourrait ramener des pages d'un fichier retiré.
      this.undoStack = [];
      this.redoStack = [];
      this.pruneSelection();
    }
    for (const entry of this.page.ready) {
      if (this.known.has(entry.id) || !entry.info) continue;
      this.known.add(entry.id);
      entry.info.pages.forEach((size, page) => {
        this.items.push({ uid: ++uidSeq, entryId: entry.id, page, rotation: 0, width: size.width, height: size.height });
      });
    }
  }

  private pruneSelection(): void {
    const uids = new Set(this.items.map((i) => i.uid));
    for (const uid of this.selected) if (!uids.has(uid)) this.selected.delete(uid);
    if (this.anchor !== null && !uids.has(this.anchor)) this.anchor = null;
  }

  private snapshot(): void {
    this.undoStack.push(this.items.map((i) => ({ ...i })));
    if (this.undoStack.length > HISTORY_LIMIT) this.undoStack.shift();
    this.redoStack = [];
  }

  private undo(): void {
    const previous = this.undoStack.pop();
    if (!previous) return;
    this.redoStack.push(this.items);
    this.items = previous;
    this.pruneSelection();
    this.changed();
  }

  private redo(): void {
    const next = this.redoStack.pop();
    if (!next) return;
    this.undoStack.push(this.items);
    this.items = next;
    this.pruneSelection();
    this.changed();
  }

  /** Les pages sélectionnées, dans l'ordre du document. */
  private selection(): Item[] {
    return this.items.filter((i) => this.selected.has(i.uid));
  }

  private rotate(delta: number): void {
    const targets = this.selection();
    if (!targets.length) return;
    this.snapshot();
    for (const item of targets) item.rotation = (item.rotation + delta + 360) % 360;
    this.changed();
  }

  private remove(): void {
    if (!this.selected.size) return;
    this.snapshot();
    const firstIndex = this.items.findIndex((i) => this.selected.has(i.uid));
    this.items = this.items.filter((i) => !this.selected.has(i.uid));
    this.selected.clear();
    // Le focus passe à la page qui a pris la place de la première supprimée.
    const next = this.items[Math.min(firstIndex, this.items.length - 1)];
    this.focusUid = next?.uid ?? null;
    this.anchor = null;
    this.changed();
  }

  /** Page blanche après la sélection (ou à la fin), au format de sa voisine. */
  private insertBlank(): void {
    const selected = this.selection();
    const last = selected[selected.length - 1];
    const at = last ? this.items.indexOf(last) + 1 : this.items.length;
    const model = last ?? this.items[this.items.length - 1];
    const turned = model ? model.rotation % 180 === 90 : false;
    const size = model ? { width: turned ? model.height : model.width, height: turned ? model.width : model.height } : A4;
    this.snapshot();
    const blank: Item = { uid: ++uidSeq, entryId: null, page: 0, rotation: 0, ...size };
    this.items.splice(at, 0, blank);
    this.selected = new Set([blank.uid]);
    this.anchor = blank.uid;
    this.focusUid = blank.uid;
    this.changed();
  }

  /** Déplace la sélection pour qu'elle commence à l'indice `to` (avant retrait). */
  private moveSelection(to: number): void {
    const moving = this.selection();
    if (!moving.length) return;
    const before = this.items.slice(0, to).filter((i) => this.selected.has(i.uid)).length;
    const rest = this.items.filter((i) => !this.selected.has(i.uid));
    const at = Math.max(0, Math.min(rest.length, to - before));
    const next = [...rest.slice(0, at), ...moving, ...rest.slice(at)];
    if (next.every((item, k) => item === this.items[k])) return;
    this.snapshot();
    this.items = next;
    this.changed();
  }

  /** Déplace la sélection d'un cran (clavier : Alt + flèche). */
  private nudge(step: -1 | 1): void {
    const indices = this.items.flatMap((item, k) => (this.selected.has(item.uid) ? [k] : []));
    if (!indices.length) return;
    const first = indices[0]!;
    const last = indices[indices.length - 1]!;
    if (step < 0 && first > 0) this.moveSelection(first - 1);
    if (step > 0 && last < this.items.length - 1) this.moveSelection(last + 2);
  }

  private changed(): void {
    this.render();
    this.page.refreshSidebar();
  }

  // --- Rendu ----------------------------------------------------------------

  render(): void {
    if (this.interacting) {
      this.dirty = true;
      return;
    }
    this.dirty = false;
    this.renderHead();
    // Le focus clavier reste sur la même page après le redessin (ou passe
    // à celle qu'une action vient de désigner), sans voler le focus ailleurs.
    const focused = document.activeElement instanceof HTMLElement ? document.activeElement.closest<HTMLElement>(".org-tile") : null;
    const keep = focused && this.grid.contains(focused) ? Number(focused.dataset.uid) : null;
    const target = this.focusUid ?? keep;
    this.focusUid = null;
    this.grid.replaceChildren(...this.items.map((item, index) => this.renderTile(item, index)));
    if (target !== null) this.grid.querySelector<HTMLElement>(`[data-uid="${target}"]`)?.focus({ preventScroll: true });
    this.grid.setAttribute("aria-label", t("organize.gridLabel"));
  }

  private colorOf(entryId: string | null): { color: string; letter: string } {
    const index = Math.max(0, this.page.entries.findIndex((e) => e.id === entryId));
    return { color: FILE_COLORS[index % FILE_COLORS.length]!, letter: String.fromCharCode(65 + (index % 26)) };
  }

  private button(name: IconName, label: string, fn: () => void, disabled = false, text = false): HTMLButtonElement {
    return h(
      "button",
      {
        class: text ? "btn btn-ghost" : "icon-btn",
        type: "button",
        title: label,
        disabled,
        attrs: text ? {} : { "aria-label": label },
        on: { click: fn },
      },
      icon(name, 18),
      text ? label : null,
    );
  }

  private renderHead(): void {
    const n = this.selected.size;
    const status =
      n > 0
        ? t("organize.selected", { n })
        : `${tn("common.page", "common.pages", this.items.length)} · ${tn("common.file", "common.files", this.page.entries.length)}`;
    const toolbar = h(
      "div",
      { class: "org-toolbar", attrs: { role: "toolbar", "aria-label": t("organize.toolbar") } },
      h("span", { class: "org-status", text: status, attrs: { "aria-live": "polite" } }),
      h("span", { class: "spacer" }),
      this.button("undo", t("edit.undo"), () => this.undo(), this.undoStack.length === 0),
      this.button("redo", t("edit.redo"), () => this.redo(), this.redoStack.length === 0),
      h("span", { class: "org-sep" }),
      this.button("rotateLeft", t("edit.rotateLeft"), () => this.rotate(-90), n === 0),
      this.button("rotateRight", t("edit.rotateRight"), () => this.rotate(90), n === 0),
      this.button("trash", t("organize.delete"), () => this.remove(), n === 0),
      this.button("plus", t("organize.blank"), () => this.insertBlank(), false, true),
      h("span", { class: "org-sep" }),
      this.button("add", t("drop.addFiles"), () => void this.page.pick(), false, true),
    );

    const chips = h("div", { class: "org-files" });
    for (const entry of this.page.entries) {
      const { color, letter } = this.colorOf(entry.id);
      const count = this.items.filter((i) => i.entryId === entry.id).length;
      const status =
        entry.status === "locked"
          ? h("button", { class: "btn btn-ghost sm", type: "button", on: { click: () => void this.page.unlock(entry) } }, icon("unlock", 14), t("password.unlock"))
          : entry.status === "error" && entry.error
            ? h("span", { class: "org-file-error", text: errorText({ ...entry.error, file: null }) })
            : entry.status === "loading"
              ? h("span", { class: "spinner", style: { width: "14px", height: "14px", "border-width": "2px" } })
              : h("span", { class: "org-file-count", text: tn("common.page", "common.pages", count) });
      chips.appendChild(
        h(
          "div",
          { class: `org-file${entry.status === "error" ? " has-error" : ""}`, title: entry.file.name },
          h("span", { class: "org-file-tag", text: letter, style: { background: color } }),
          h("span", { class: "org-file-name", text: entry.file.name }),
          status,
          h(
            "button",
            {
              class: "icon-btn sm",
              type: "button",
              title: t("organize.removeFile"),
              attrs: { "aria-label": `${t("organize.removeFile")} : ${entry.file.name}` },
              on: { click: () => this.page.removeEntry(entry.id) },
            },
            icon("cross", 14),
          ),
        ),
      );
    }
    this.head.replaceChildren(toolbar, chips);
  }

  /** Page (ou page blanche) tournée, ajustée dans le cadre de la miniature. */
  private pageBox(
    item: Item,
    frame: { width: number; height: number },
    content: HTMLElement,
    relative = false,
  ): HTMLElement {
    const turned = item.rotation % 180 === 90;
    const rw = turned ? item.height : item.width;
    const rh = turned ? item.width : item.height;
    const scale = Math.min(frame.width / rw, frame.height / rh);
    // En pourcentage du cadre (aperçu, dont la largeur suit la fenêtre) ou en pixels.
    const w = relative ? `${((item.width * scale) / frame.width) * 100}%` : `${item.width * scale}px`;
    const hgt = relative ? `${((item.height * scale) / frame.height) * 100}%` : `${item.height * scale}px`;
    return h(
      "div",
      {
        class: "org-page",
        style: {
          width: w,
          height: hgt,
          transform: `translate(-50%, -50%) rotate(${item.rotation}deg)`,
        },
      },
      content,
    );
  }

  private renderTile(item: Item, index: number): HTMLElement {
    const entry = this.entry(item.entryId);
    const selected = this.selected.has(item.uid);
    const content = entry ? this.page.pageThumb(entry, item.page, 220) : h("div", { class: "org-blank" });
    const many = this.page.entries.length > 1;
    const { color, letter } = this.colorOf(item.entryId);
    const label = entry
      ? t("organize.tileLabel", { n: index + 1, file: entry.file.name, page: item.page + 1 })
      : t("organize.tileBlank", { n: index + 1 });

    const tileAction = (name: IconName, text: string, fn: () => void) =>
      h(
        "button",
        {
          class: "icon-btn sm",
          type: "button",
          title: text,
          attrs: { "aria-label": text, tabindex: -1 },
          on: {
            click: (e) => {
              e.stopPropagation();
              this.selected = new Set([item.uid]);
              this.anchor = item.uid;
              fn();
            },
          },
        },
        icon(name, 16),
      );

    return h(
      "div",
      {
        class: `org-tile${selected ? " is-selected" : ""}`,
        dataset: { uid: String(item.uid) },
        title: entry ? `${entry.file.name} · ${t("organize.sourcePage", { page: item.page + 1 })}` : t("organize.blankPage"),
        attrs: { role: "option", "aria-selected": String(selected), "aria-label": label, tabindex: 0 },
      },
      h("div", { class: "org-frame" }, this.pageBox(item, FRAME, content)),
      h(
        "button",
        {
          class: "org-check",
          type: "button",
          title: t(selected ? "organize.unselect" : "organize.select"),
          attrs: { "aria-hidden": "true", tabindex: -1 },
          on: {
            click: (e) => {
              e.stopPropagation();
              this.toggle(item.uid);
            },
          },
        },
        icon("check", 14),
      ),
      h(
        "div",
        { class: "org-tile-actions" },
        tileAction("rotateRight", t("edit.rotateRight"), () => this.rotate(90)),
        tileAction("trash", t("organize.delete"), () => this.remove()),
      ),
      h(
        "div",
        { class: "org-label" },
        h("span", { class: "org-number", text: String(index + 1) }),
        many && entry ? h("span", { class: "org-source" }, h("i", { style: { background: color }, text: letter }), `p. ${item.page + 1}`) : null,
        !entry ? h("span", { class: "org-source", text: t("organize.blankShort") }) : null,
      ),
    );
  }

  /** Met à jour l'état sélectionné des tuiles sans tout redessiner. */
  private paintSelection(): void {
    for (const tile of this.grid.querySelectorAll<HTMLElement>(".org-tile")) {
      const on = this.selected.has(Number(tile.dataset.uid));
      tile.classList.toggle("is-selected", on);
      tile.setAttribute("aria-selected", String(on));
    }
  }

  private selectionChanged(): void {
    this.paintSelection();
    this.renderHead();
  }

  private toggle(uid: number): void {
    if (this.selected.has(uid)) this.selected.delete(uid);
    else this.selected.add(uid);
    this.anchor = uid;
    this.selectionChanged();
  }

  private selectRange(uid: number, additive: boolean): void {
    const from = this.items.findIndex((i) => i.uid === (this.anchor ?? uid));
    const to = this.items.findIndex((i) => i.uid === uid);
    if (from < 0 || to < 0) return;
    if (!additive) this.selected.clear();
    const [a, b] = from < to ? [from, to] : [to, from];
    for (let k = a; k <= b; k++) this.selected.add(this.items[k]!.uid);
  }

  /** Clic façon Windows : seul, Ctrl (ajoute / retire), Maj (plage). */
  private click(uid: number, e: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): void {
    if (e.shiftKey) {
      this.selectRange(uid, e.ctrlKey || e.metaKey);
    } else if (e.ctrlKey || e.metaKey) {
      this.toggle(uid);
      return;
    } else {
      this.selected = new Set([uid]);
      this.anchor = uid;
    }
    this.selectionChanged();
  }

  // --- Souris : glisser, cadre de sélection ---------------------------------

  private scroller(): HTMLElement | null {
    return this.root.closest<HTMLElement>(".files-area");
  }

  private onPointerDown(down: PointerEvent): void {
    if (down.button !== 0 || !down.isPrimary) return;
    const target = down.target as Element;
    if (target.closest("button, input, select, a, .org-head")) return;
    const tile = target.closest<HTMLElement>(".org-tile");
    if (tile) this.startTileGesture(down, tile);
    else if (down.pointerType === "mouse" || down.pointerType === "pen") this.startLasso(down);
  }

  /** Défilement automatique quand le pointeur approche du bord haut ou bas. */
  private autoScroll(getY: () => number, onScroll: () => void): () => void {
    const area = this.scroller();
    let frame = 0;
    const tick = () => {
      if (area) {
        const r = area.getBoundingClientRect();
        const y = getY();
        const edge = 48;
        const speed = y < r.top + edge ? -(r.top + edge - y) : y > r.bottom - edge ? y - (r.bottom - edge) : 0;
        if (speed !== 0) {
          const before = area.scrollTop;
          area.scrollTop += Math.max(-24, Math.min(24, speed / 2));
          if (area.scrollTop !== before) onScroll();
        }
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }

  /** Suit un geste de pointeur ; s'arrête proprement en cas d'interruption. */
  private track(
    pointerId: number,
    onMove: (e: PointerEvent) => void,
    onEnd: (commit: boolean, e: PointerEvent | null) => void,
  ): void {
    const move = (e: PointerEvent) => e.pointerId === pointerId && onMove(e);
    const up = (e: PointerEvent) => e.pointerId === pointerId && finish(true, e);
    const cancel = (e: PointerEvent) => e.pointerId === pointerId && finish(false, e);
    const blur = () => finish(false, null);
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        finish(false, null);
      }
    };
    const finish = (commit: boolean, e: PointerEvent | null) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("blur", blur);
      window.removeEventListener("keydown", key, true);
      onEnd(commit, e);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("blur", blur);
    window.addEventListener("keydown", key, true);
  }

  private startTileGesture(down: PointerEvent, tile: HTMLElement): void {
    const uid = Number(tile.dataset.uid);
    const mods = { ctrlKey: down.ctrlKey, metaKey: down.metaKey, shiftKey: down.shiftKey };
    // Comme dans l'Explorateur : un appui sur une page non sélectionnée la
    // sélectionne tout de suite (pour pouvoir la glisser aussitôt).
    const wasSelected = this.selected.has(uid);
    if (!wasSelected) this.click(uid, mods);
    tile.focus({ preventScroll: true });
    // Au doigt, la grille défile : pas de glisser.
    if (down.pointerType === "touch") {
      if (wasSelected) this.click(uid, mods);
      return;
    }
    down.preventDefault();

    const startX = down.clientX;
    const startY = down.clientY;
    let x = startX;
    let y = startY;
    let ghost: HTMLElement | null = null;
    let marker: HTMLElement | null = null;
    let insertAt: number | null = null;
    let stopScroll: (() => void) | null = null;

    const place = () => {
      if (!ghost || !marker) return;
      ghost.style.left = `${x + 12}px`;
      ghost.style.top = `${y + 12}px`;
      const hit = this.insertionPoint(x, y);
      insertAt = hit?.index ?? null;
      marker.style.display = hit ? "" : "none";
      if (hit) {
        const base = this.root.getBoundingClientRect();
        marker.style.left = `${hit.x - base.left}px`;
        marker.style.top = `${hit.top - base.top}px`;
        marker.style.height = `${hit.height}px`;
      }
    };

    this.track(
      down.pointerId,
      (e) => {
        x = e.clientX;
        y = e.clientY;
        if (!ghost) {
          if (Math.hypot(x - startX, y - startY) < 6) return;
          if (!this.selected.has(uid)) this.click(uid, { ctrlKey: false, metaKey: false, shiftKey: false });
          this.interacting = true;
          ghost = this.makeGhost(tile);
          marker = h("div", { class: "org-insert" });
          this.root.appendChild(marker);
          this.root.classList.add("is-dragging");
          for (const el of this.grid.querySelectorAll<HTMLElement>(".org-tile.is-selected")) el.classList.add("is-moving");
          stopScroll = this.autoScroll(() => y, place);
        }
        place();
      },
      (commit) => {
        stopScroll?.();
        if (!ghost) {
          // Simple clic : sélection façon Windows.
          if (commit && wasSelected) this.click(uid, mods);
          return;
        }
        ghost.remove();
        marker?.remove();
        this.root.classList.remove("is-dragging");
        this.interacting = false;
        this.focusUid = uid;
        if (commit && insertAt !== null) this.moveSelection(insertAt);
        this.render();
      },
    );
  }

  private makeGhost(tile: HTMLElement): HTMLElement {
    const frame = tile.querySelector(".org-frame")?.cloneNode(true) as HTMLElement | undefined;
    const count = this.selected.size;
    const ghost = h(
      "div",
      { class: `org-ghost${count > 1 ? " is-stack" : ""}` },
      frame ?? null,
      count > 1 ? h("span", { class: "org-ghost-count", text: String(count) }) : null,
    );
    document.body.appendChild(ghost);
    return ghost;
  }

  /** Où insérer à cette position : avant ou après la page la plus proche. */
  private insertionPoint(x: number, y: number): { index: number; x: number; top: number; height: number } | null {
    const tiles = [...this.grid.querySelectorAll<HTMLElement>(".org-tile")];
    let best: { tile: HTMLElement; rect: DOMRect; dist: number } | null = null;
    for (const tile of tiles) {
      const rect = tile.getBoundingClientRect();
      const dx = x < rect.left ? rect.left - x : x > rect.right ? x - rect.right : 0;
      const dy = y < rect.top ? rect.top - y : y > rect.bottom ? y - rect.bottom : 0;
      // La ligne compte plus que la colonne : on vise la rangée sous le pointeur.
      const dist = dy * 4 + dx;
      if (!best || dist < best.dist) best = { tile, rect, dist };
    }
    if (!best) return null;
    const index = tiles.indexOf(best.tile);
    const after = x > best.rect.left + best.rect.width / 2;
    const gap = 7;
    return {
      index: index + (after ? 1 : 0),
      x: after ? best.rect.right + gap : best.rect.left - gap,
      top: best.rect.top,
      height: best.rect.height,
    };
  }

  private startLasso(down: PointerEvent): void {
    down.preventDefault();
    const base = new Set(down.ctrlKey || down.metaKey || down.shiftKey ? this.selected : []);
    const origin = this.root.getBoundingClientRect();
    // Coordonnées dans la page de contenu (stables pendant le défilement).
    const startX = down.clientX - origin.left;
    const startY = down.clientY - origin.top;
    let x = down.clientX;
    let y = down.clientY;
    let box: HTMLElement | null = null;
    let stopScroll: (() => void) | null = null;
    (document.activeElement as HTMLElement | null)?.blur?.();

    const update = () => {
      if (!box) return;
      const r = this.root.getBoundingClientRect();
      const cx = x - r.left;
      const cy = y - r.top;
      const left = Math.min(startX, cx);
      const top = Math.min(startY, cy);
      const width = Math.abs(cx - startX);
      const height = Math.abs(cy - startY);
      Object.assign(box.style, { left: `${left}px`, top: `${top}px`, width: `${width}px`, height: `${height}px` });
      const sel = { left: left + r.left, top: top + r.top, right: left + r.left + width, bottom: top + r.top + height };
      const next = new Set(base);
      for (const tile of this.grid.querySelectorAll<HTMLElement>(".org-tile")) {
        const t = tile.getBoundingClientRect();
        if (t.left < sel.right && t.right > sel.left && t.top < sel.bottom && t.bottom > sel.top) {
          next.add(Number(tile.dataset.uid));
        }
      }
      this.selected = next;
      this.paintSelection();
    };

    this.track(
      down.pointerId,
      (e) => {
        x = e.clientX;
        y = e.clientY;
        if (!box) {
          if (Math.hypot(x - down.clientX, y - down.clientY) < 4) return;
          this.interacting = true;
          box = h("div", { class: "org-lasso" });
          this.root.appendChild(box);
          stopScroll = this.autoScroll(() => y, update);
        }
        update();
      },
      (commit) => {
        stopScroll?.();
        this.interacting = false;
        if (!box) {
          // Clic dans le vide : on désélectionne (sauf avec Ctrl / Maj).
          if (commit && base.size === 0 && this.selected.size) {
            this.selected.clear();
            this.selectionChanged();
          }
          return;
        }
        box.remove();
        if (!commit) this.selected = base;
        if (this.dirty) this.render();
        else this.selectionChanged();
      },
    );
  }

  // --- Clavier ----------------------------------------------------------------

  private onKey(e: KeyboardEvent): void {
    if (!this.root.isConnected || this.interacting || e.defaultPrevented) return;
    if (document.querySelector(".modal-backdrop, .org-preview")) return;
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest("input, textarea, select, [contenteditable]")) return;
    // Raccourcis actifs seulement si le focus est dans la grille ou nulle part.
    if (active && active !== document.body && !this.root.contains(active)) return;

    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    const tile = active instanceof HTMLElement ? active.closest<HTMLElement>(".org-tile") : null;
    const done = () => e.preventDefault();

    if (ctrl && key === "a") {
      this.selected = new Set(this.items.map((i) => i.uid));
      this.selectionChanged();
      done();
    } else if (ctrl && key === "z" && !e.shiftKey) {
      this.undo();
      done();
    } else if (ctrl && (key === "y" || (key === "z" && e.shiftKey))) {
      this.redo();
      done();
    } else if (key === "delete" || key === "backspace") {
      this.remove();
      done();
    } else if (key === "escape" && this.selected.size) {
      this.selected.clear();
      this.selectionChanged();
      done();
    } else if (e.altKey && (key === "arrowleft" || key === "arrowright")) {
      this.nudge(key === "arrowleft" ? -1 : 1);
      done();
    } else if (tile && (key === " " || key === "enter")) {
      this.click(Number(tile.dataset.uid), { ctrlKey: true, metaKey: false, shiftKey: e.shiftKey });
      done();
    } else if (["arrowleft", "arrowright", "arrowup", "arrowdown", "home", "end"].includes(key)) {
      this.moveFocus(tile, key, e);
      done();
    }
  }

  /** Flèches : page voisine (haut / bas : rangée voisine), comme l'Explorateur. */
  private moveFocus(tile: HTMLElement | null, key: string, e: KeyboardEvent): void {
    const tiles = [...this.grid.querySelectorAll<HTMLElement>(".org-tile")];
    if (!tiles.length) return;
    const index = tile ? tiles.indexOf(tile) : -1;
    let next = index;
    if (index < 0) next = 0;
    else if (key === "home") next = 0;
    else if (key === "end") next = tiles.length - 1;
    else if (key === "arrowleft") next = index - 1;
    else if (key === "arrowright") next = index + 1;
    else {
      const columns = Math.max(1, getComputedStyle(this.grid).gridTemplateColumns.split(" ").length);
      next = index + (key === "arrowup" ? -columns : columns);
    }
    const target = tiles[Math.max(0, Math.min(tiles.length - 1, next))];
    if (!target) return;
    target.focus();
    const uid = Number(target.dataset.uid);
    // Ctrl + flèche : déplace le focus sans changer la sélection.
    if (!(e.ctrlKey || e.metaKey)) this.click(uid, { ctrlKey: false, metaKey: false, shiftKey: e.shiftKey });
  }

  // --- Aperçu du résultat ---------------------------------------------------

  specs(): { inputs: Entry[]; pages: PageSpec[] } {
    const inputs: Entry[] = [];
    const pages = this.items.map<PageSpec>((item) => {
      const entry = this.entry(item.entryId);
      if (!entry) return { kind: "blank", width: item.width, height: item.height, rotation: item.rotation };
      let file = inputs.indexOf(entry);
      if (file < 0) file = inputs.push(entry) - 1;
      return { kind: "source", file, page: item.page, rotation: item.rotation };
    });
    return { inputs, pages };
  }

  openPreview(save: () => void): void {
    const previous = document.activeElement as HTMLElement | null;
    const width = 760;
    const pages = h("div", { class: "org-preview-pages" });
    const observer = new IntersectionObserver(
      (records) => {
        for (const r of records) {
          if (!r.isIntersecting) continue;
          observer.unobserve(r.target);
          void this.loadPreview(r.target as HTMLElement);
        }
      },
      { rootMargin: "600px" },
    );
    const total = this.items.length;
    this.items.forEach((item, index) => {
      const entry = this.entry(item.entryId);
      const turned = item.rotation % 180 === 90;
      const rw = turned ? item.height : item.width;
      const rh = turned ? item.width : item.height;
      const frame = { width, height: (width * rh) / rw };
      const content = entry ? h("div", { class: "org-preview-img" }, h("div", { class: "spinner" })) : h("div", { class: "org-blank" });
      const box = this.pageBox(item, frame, content, true);
      const sheet = h(
        "figure",
        { class: "org-preview-sheet", style: { "aspect-ratio": `${rw} / ${rh}` } },
        box,
      );
      if (entry) {
        content.dataset.key = `${entry.id}:${item.page}`;
        observer.observe(content);
      }
      pages.appendChild(
        h(
          "div",
          { class: "org-preview-item" },
          sheet,
          h("div", { class: "org-preview-caption", text: t("organize.previewPage", { n: index + 1, total }) }),
        ),
      );
    });

    const close = () => {
      observer.disconnect();
      document.removeEventListener("keydown", onKey, true);
      overlay.remove();
      previous?.focus?.();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      } else if (e.key === "Tab") {
        trapFocus(e, overlay);
      }
    };
    const closeBtn = h("button", { class: "btn", type: "button", on: { click: close } }, icon("cross", 18), t("organize.previewClose"));
    const overlay = h(
      "div",
      { class: "org-preview", attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "org-preview-title" } },
      h(
        "div",
        { class: "org-preview-bar" },
        h("h2", { text: t("organize.previewTitle"), attrs: { id: "org-preview-title" } }),
        h("span", { class: "org-preview-count", text: tn("common.page", "common.pages", total) }),
        h("span", { class: "spacer" }),
        closeBtn,
        h(
          "button",
          {
            class: "btn btn-primary",
            type: "button",
            on: {
              click: () => {
                close();
                save();
              },
            },
          },
          t("organize.action"),
          icon("arrowRight", 18),
        ),
      ),
      h("div", { class: "org-preview-scroll", attrs: { "aria-label": t("organize.previewTitle"), tabindex: 0 } }, pages),
    );
    document.body.appendChild(overlay);
    document.addEventListener("keydown", onKey, true);
    this.page.scope.add(() => overlay.isConnected && close());
    closeBtn.focus();
  }

  private async loadPreview(el: HTMLElement): Promise<void> {
    const key = el.dataset.key ?? "";
    const [entryId, pageText] = key.split(":");
    const entry = this.entry(entryId ?? null);
    if (!entry) return;
    const width = Math.min(2000, Math.round(760 * Math.max(1, window.devicePixelRatio || 1)));
    try {
      let url = this.previewCache.get(key);
      if (!url) {
        const bytes = await api.renderPage(this.page.pdfInput(entry), Number(pageText), width);
        if (this.page.scope.isDisposed) return;
        url = this.page.scope.objectUrl(new Blob([bytes], { type: "image/jpeg" }));
        this.previewCache.set(key, url);
      }
      el.replaceChildren(h("img", { src: url, alt: "", draggable: false }));
    } catch {
      el.replaceChildren(icon("alert", 28));
    }
  }
}

const organizers = new WeakMap<ToolPage, Organizer>();

function organizer(page: ToolPage): Organizer {
  let org = organizers.get(page);
  if (!org) {
    org = new Organizer(page);
    organizers.set(page, org);
  }
  return org;
}

export const organizeConfig: ToolConfig = {
  multiple: true,
  pickLabel: "drop.selectPdf",

  main(page) {
    const org = organizer(page);
    org.sync();
    org.render();
    return org.root;
  },

  sidebar(page) {
    const org = organizer(page);
    org.sync();
    const hasSource = org.items.some((i) => i.entryId !== null);
    const files = new Set(org.items.map((i) => i.entryId).filter(Boolean)).size;
    const preview = h(
      "button",
      {
        class: "btn btn-block",
        type: "button",
        disabled: !hasSource || page.busy,
        on: { click: () => org.openPreview(() => void page.run()) },
      },
      icon("paper", 18),
      t("organize.preview"),
    );
    const shortcuts = h(
      "dl",
      { class: "org-keys" },
      [
        [t("organize.kCtrlClick"), t("organize.keyCtrl")],
        [t("organize.kShiftClick"), t("organize.keyShift")],
        ["Ctrl + A", t("organize.keyAll")],
        [t("organize.kDelete"), t("organize.delete")],
        [t("organize.kNudge"), t("organize.keyNudge")],
        ["Ctrl + Z", t("organize.keyUndo")],
      ].flatMap(([k, v]) => [h("dt", null, h("kbd", { text: k! })), h("dd", { text: v! })]),
    );
    return {
      body: [infoBox(t("organize.help")), preview, h("div", { class: "field" }, h("span", { class: "field-label", text: t("organize.shortcuts") }), shortcuts)],
      action: t("organize.action"),
      blocker: entriesBlocker(page) ?? (hasSource ? null : t("organize.empty")),
      summary: `${tn("common.page", "common.pages", org.items.length)} · ${tn("common.file", "common.files", files)}`,
    };
  },

  run(page, jobId, progress) {
    const { inputs, pages } = organizer(page).specs();
    return api.organizePdf(
      jobId,
      inputs.map((e) => page.pdfInput(e)),
      pages,
      inputs.length === 1 && inputs[0] ? `${inputs[0].file.name.replace(/\.pdf$/i, "")}_${t("organize.suffix")}` : t("organize.outputName"),
      progress,
    );
  },

  resultTitle: () => t("organize.done"),
};
