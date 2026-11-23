// Page d'outil générique : sélection des fichiers → options → traitement →
// résultat. Chaque outil fournit sa configuration (voir ./tools/*).

import type { ViewContext } from "../app";
import {
  api,
  Channel,
  toCommandError,
  type CommandError,
  type FileInfo,
  type JobOutcome,
  type PdfInfo,
  type PdfInput,
  type Progress,
} from "../api";
import { clear, h, type Scope } from "../dom";
import { formatSize, t, tn, type Key } from "../i18n";
import { icon, type IconName } from "../icons";
import { descKey, titleKey, toolById, type ToolDef, type ToolId } from "../tools";
import { askPassword, errorText, toast } from "../ui";
import { compressConfig } from "./tools/compress";
import { jpgToPdfConfig } from "./tools/jpg-to-pdf";
import { mergeConfig } from "./tools/merge";
import { confirmLeave, editConfig } from "./tools/edit";
import { officeToPdfConfig } from "./tools/office-to-pdf";
import { pdfToJpgConfig } from "./tools/pdf-to-jpg";
import { pdfToOfficeConfig } from "./tools/pdf-to-office";
import { splitConfig } from "./tools/split";

// ---------------------------------------------------------------------------
// Configuration d'un outil
// ---------------------------------------------------------------------------

export interface SidebarSpec {
  body: HTMLElement[];
  action: string;
  /** Raison pour laquelle on ne peut pas encore lancer (bouton désactivé). */
  blocker?: string | null;
  /** Petite phrase sous le bouton. */
  summary?: string | null;
}

export interface ToolConfig {
  multiple: boolean;
  /** Zone principale sans marges (éditeur). */
  fullBleed?: boolean;
  /** Travail non enregistré : on demande confirmation avant de quitter. */
  hasUnsavedWork?(page: ToolPage): boolean;
  reorder?: boolean;
  rotate?: boolean;
  pickLabel: Key;
  sidebar(page: ToolPage): SidebarSpec;
  /** Zone principale personnalisée (sinon : cartes de fichiers). */
  main?(page: ToolPage): HTMLElement;
  run(page: ToolPage, jobId: string, progress: Channel<Progress>): Promise<JobOutcome>;
  resultTitle(outcome: JobOutcome): string;
  /** Détails propres à l'outil sur l'écran de résultat. */
  resultDetails?(outcome: JobOutcome): HTMLElement | null;
}

const CONFIGS: Partial<Record<ToolId, ToolConfig>> = {
  merge: mergeConfig,
  split: splitConfig,
  compress: compressConfig,
  pdfToJpg: pdfToJpgConfig,
  jpgToPdf: jpgToPdfConfig,
  edit: editConfig,
  wordToPdf: officeToPdfConfig("word"),
  pptToPdf: officeToPdfConfig("powerpoint"),
  excelToPdf: officeToPdfConfig("excel"),
  pdfToWord: pdfToOfficeConfig("docx"),
  pdfToPpt: pdfToOfficeConfig("pptx"),
  pdfToExcel: pdfToOfficeConfig("xlsx"),
};

/** Icône de carte pour un document sans aperçu (Word, PowerPoint, Excel). */
const KIND_ICON: Partial<Record<FileInfo["kind"], IconName>> = {
  word: "wordToPdf",
  powerpoint: "pptToPdf",
  excel: "excelToPdf",
};

export function toolView(ctx: ViewContext, id: ToolId, files: FileInfo[]): HTMLElement {
  const config = CONFIGS[id];
  if (!config) throw new Error(`outil non configuré : ${id}`);
  return new ToolPage(ctx, toolById(id), config, files).el;
}

// ---------------------------------------------------------------------------
// Fichiers sélectionnés
// ---------------------------------------------------------------------------

export type EntryStatus = "loading" | "ready" | "locked" | "error";

export interface Entry {
  id: string;
  file: FileInfo;
  password: string | null;
  info: PdfInfo | null;
  status: EntryStatus;
  error: CommandError | null;
  rotation: number;
}

let entrySeq = 0;

type Stage = "empty" | "files" | "working" | "result" | "error";

export class ToolPage {
  readonly el: HTMLElement;
  entries: Entry[] = [];
  readonly scope: Scope;
  private stage: Stage = "empty";
  private body = h("div", { class: "tool-view-body", style: { display: "contents" } });
  private filesArea: HTMLElement | null = null;
  private sidebarEl: HTMLElement | null = null;
  private thumbs = new Map<string, string>();
  private pageThumbs = new Map<string, string>();
  private pending = new WeakMap<Element, { entry: Entry; index: number; width: number }>();
  private observer: IntersectionObserver | null = null;

  constructor(
    readonly ctx: ViewContext,
    readonly tool: ToolDef,
    readonly config: ToolConfig,
    initial: FileInfo[],
  ) {
    this.scope = ctx.scope;
    this.el = h("div", { class: "tool-view" }, this.renderHead(), this.body);
    ctx.onFiles((files) => this.addFiles(files));
    ctx.keepState(() => ({ name: "tool", id: tool.id, files: this.entries.map((e) => e.file) }));
    ctx.setLeaveGuard(async () => (this.stage === "files" && config.hasUnsavedWork?.(this) ? confirmLeave() : true));
    this.render();
    if (initial.length) this.addFiles(initial);
  }

  // --- Données --------------------------------------------------------------

  get ready(): Entry[] {
    return this.entries.filter((e) => e.status === "ready");
  }

  get busy(): boolean {
    return this.entries.some((e) => e.status === "loading");
  }

  pdfInput(e: Entry): PdfInput {
    return { path: e.file.path, password: e.password };
  }

  addFiles(files: FileInfo[]): void {
    if (this.stage === "working") return;
    const accepted = files.filter((f) => f.kind === this.tool.accepts);
    if (accepted.length < files.length) toast(t("drop.unsupported"), "error");
    if (accepted.length === 0) return;

    let list = accepted;
    if (!this.config.multiple) {
      if (accepted.length > 1) toast(t("drop.onlyOne"));
      list = accepted.slice(0, 1);
      this.entries.forEach((e) => this.releaseThumb(e.id));
      this.entries = [];
    }
    const added = list.map<Entry>((file) => ({
      id: `e${++entrySeq}`,
      file,
      password: null,
      info: null,
      status: file.kind === "pdf" ? "loading" : "ready",
      error: null,
      rotation: 0,
    }));
    this.entries.push(...added);
    if (this.stage === "empty" || this.stage === "result" || this.stage === "error") this.stage = "files";
    this.render();
    void this.loadEntries(added);
  }

  private async loadEntries(added: Entry[]): Promise<void> {
    for (const entry of added) {
      if (entry.file.kind === "pdf") await this.inspect(entry);
      else if (entry.file.kind === "image") void this.loadThumb(entry);
    }
    // Propose le mot de passe pour les fichiers protégés qui viennent d'être ajoutés.
    for (const entry of added) {
      if (entry.status === "locked" && !this.scope.isDisposed) await this.unlock(entry);
    }
  }

  private async inspect(entry: Entry): Promise<void> {
    try {
      entry.info = await api.inspectPdf(this.pdfInput(entry));
      entry.status = "ready";
      entry.error = null;
      void this.loadThumb(entry);
    } catch (e) {
      const err = toCommandError(e);
      entry.error = err;
      entry.status = err.code === "password_required" || err.code === "wrong_password" ? "locked" : "error";
    }
    if (!this.scope.isDisposed && this.entries.includes(entry)) this.refreshFiles();
  }

  async unlock(entry: Entry): Promise<void> {
    const password = await askPassword(entry.file.name, async (pw) => {
      try {
        entry.info = await api.inspectPdf({ path: entry.file.path, password: pw });
        return true;
      } catch (e) {
        const err = toCommandError(e);
        if (err.code === "wrong_password" || err.code === "password_required") return false;
        entry.error = err;
        entry.status = "error";
        return true;
      }
    });
    if (password && entry.status !== "error") {
      entry.password = password;
      entry.status = "ready";
      entry.error = null;
      void this.loadThumb(entry);
    }
    if (!this.scope.isDisposed) this.refreshFiles();
  }

  removeEntry(id: string): void {
    this.releaseThumb(id);
    this.entries = this.entries.filter((e) => e.id !== id);
    if (this.entries.length === 0) this.stage = "empty";
    this.render();
  }

  moveEntry(from: number, to: number): void {
    if (from === to || from < 0 || to < 0 || from >= this.entries.length || to >= this.entries.length) return;
    const [moved] = this.entries.splice(from, 1);
    if (moved) this.entries.splice(to, 0, moved);
    this.refreshFiles();
  }

  sortByName(): void {
    this.entries.sort((a, b) => a.file.name.localeCompare(b.file.name, undefined, { numeric: true, sensitivity: "base" }));
    this.refreshFiles();
  }

  private async loadThumb(entry: Entry): Promise<void> {
    if (this.thumbs.has(entry.id)) return;
    try {
      const bytes =
        entry.file.kind === "pdf"
          ? await api.renderPage(this.pdfInput(entry), 0, 360)
          : await api.imageThumbnail(entry.file.path, 360);
      if (this.scope.isDisposed || !this.entries.includes(entry)) return;
      this.thumbs.set(entry.id, this.scope.objectUrl(new Blob([bytes], { type: "image/jpeg" })));
    } catch (e) {
      if (entry.file.kind === "image") {
        entry.status = "error";
        entry.error = toCommandError(e);
      }
    }
    if (!this.scope.isDisposed) this.refreshFiles();
  }

  private releaseThumb(id: string): void {
    const url = this.thumbs.get(id);
    if (url) URL.revokeObjectURL(url);
    this.thumbs.delete(id);
  }

  thumbUrl(entry: Entry): string | undefined {
    return this.thumbs.get(entry.id);
  }

  async pick(): Promise<void> {
    try {
      const files = await api.pickFiles(this.tool.accepts, this.config.multiple);
      if (files.length) this.addFiles(files);
    } catch (e) {
      toast(errorText(toCommandError(e)), "error");
    }
  }

  // --- Rendu ------------------------------------------------------------------

  private renderHead(): HTMLElement {
    return h(
      "div",
      { class: "tool-head" },
      h(
        "button",
        { class: "btn btn-ghost back", on: { click: () => this.ctx.navigate({ name: "home" }) } },
        icon("arrowLeft", 18),
        t("common.back"),
      ),
    );
  }

  render(): void {
    clear(this.body);
    this.filesArea = null;
    this.sidebarEl = null;
    switch (this.stage) {
      case "empty":
        this.body.appendChild(this.renderEmpty());
        break;
      case "files":
        this.body.appendChild(this.renderFilesStage());
        break;
      default:
        break;
    }
  }

  private renderEmpty(): HTMLElement {
    const many = this.config.multiple;
    const dropzone = h(
      "div",
      { class: "dropzone" },
      h("span", { class: "tool-icon" }, icon(this.tool.icon, 64)),
      h("h2", { text: t(titleKey(this.tool.id)) }),
      h("p", { text: t(descKey(this.tool.id)) }),
      h(
        "button",
        { class: "btn btn-primary btn-xl", on: { click: () => void this.pick() } },
        t(this.config.pickLabel),
      ),
      h("span", { class: "hint", text: t(many ? "drop.orDrop" : "drop.orDropOne") }),
      this.tool.accepts === "image" ? h("span", { class: "hint", text: t("drop.imagesHint") }) : null,
      this.tool.accepts === "word" ? h("span", { class: "hint", text: t("drop.wordHint") }) : null,
      this.tool.accepts === "powerpoint" ? h("span", { class: "hint", text: t("drop.powerpointHint") }) : null,
      this.tool.accepts === "excel" ? h("span", { class: "hint", text: t("drop.excelHint") }) : null,
    );
    return h("div", { class: "tool-empty" }, dropzone);
  }

  private renderFilesStage(): HTMLElement {
    this.filesArea = h("div", { class: `files-area${this.config.fullBleed ? " flush" : ""}` });
    this.sidebarEl = h("aside", { class: "sidebar" });
    this.refreshFiles();
    return h("div", { class: "tool-body" }, this.filesArea, this.sidebarEl);
  }

  /** Redessine la zone des fichiers et le panneau d'options. */
  refreshFiles(): void {
    this.refreshMain();
    this.refreshSidebar();
  }

  /** Redessine seulement la zone principale (fichiers ou pages). */
  refreshMain(): void {
    if (this.stage !== "files" || !this.filesArea) return;
    const area = this.filesArea;
    const scroll = area.scrollTop;
    const content = this.config.main ? this.config.main(this) : this.renderCards();
    // Contenu persistant (éditeur) déjà en place : rien à refaire.
    if (area.childNodes.length === 1 && area.firstChild === content) return;
    clear(area);
    area.appendChild(content);
    area.scrollTop = scroll;
  }

  /** Met à jour le bouton d'action sans toucher aux champs (saisie en cours). */
  refreshAction(): void {
    const foot = this.sidebarEl?.querySelector(".sidebar-foot");
    if (!foot) return;
    foot.replaceWith(this.renderFoot(this.config.sidebar(this)));
  }

  refreshSidebar(): void {
    if (!this.sidebarEl) return;
    const spec = this.config.sidebar(this);
    const bodyEl = this.sidebarEl.querySelector(".sidebar-body");
    const scroll = bodyEl?.scrollTop ?? 0;
    this.sidebarEl.replaceChildren(
      h("div", { class: "sidebar-head" }, h("h3", { text: t(titleKey(this.tool.id)) })),
      h("div", { class: "sidebar-body" }, ...spec.body),
      this.renderFoot(spec),
    );
    const newBody = this.sidebarEl.querySelector(".sidebar-body");
    if (newBody) newBody.scrollTop = scroll;
  }

  private renderFoot(spec: SidebarSpec): HTMLElement {
    const blocker = this.busy ? t("common.loading") : spec.blocker ?? null;
    const runBtn = h(
      "button",
      {
        class: "btn btn-primary btn-lg btn-block",
        disabled: blocker !== null || this.ready.length === 0,
        on: { click: () => void this.run() },
      },
      spec.action,
      icon("arrowRight", 20),
    );
    const note = blocker ?? spec.summary ?? null;
    return h("div", { class: "sidebar-foot" }, note ? h("div", { class: "summary", text: note }) : null, runBtn);
  }

  /**
   * Miniature d'une page, chargée seulement quand elle devient visible
   * (indispensable pour les longs documents) puis gardée en cache.
   */
  pageThumb(entry: Entry, index: number, width: number): HTMLElement {
    const size = entry.info?.pages[index];
    const ratio = size && size.width > 0 ? size.width / size.height : 1 / Math.SQRT2;
    const box = h("div", { class: "page-img", style: { "aspect-ratio": String(ratio) } });
    const key = `${entry.id}:${index}`;
    const url = this.pageThumbs.get(key);
    if (url) {
      box.appendChild(h("img", { src: url, alt: "", draggable: false }));
      return box;
    }
    box.appendChild(h("div", { class: "spinner", style: { width: "22px", height: "22px", "border-width": "3px" } }));
    if (!this.observer) {
      this.observer = new IntersectionObserver(
        (items) => {
          for (const item of items) {
            if (!item.isIntersecting) continue;
            this.observer?.unobserve(item.target);
            const job = this.pending.get(item.target);
            if (job) void this.loadPageThumb(item.target as HTMLElement, job.entry, job.index, job.width);
          }
        },
        { rootMargin: "300px" },
      );
      this.scope.add(() => this.observer?.disconnect());
    }
    this.pending.set(box, { entry, index, width });
    this.observer.observe(box);
    return box;
  }

  private async loadPageThumb(box: HTMLElement, entry: Entry, index: number, width: number): Promise<void> {
    const key = `${entry.id}:${index}`;
    try {
      let url = this.pageThumbs.get(key);
      if (!url) {
        const bytes = await api.renderPage(this.pdfInput(entry), index, width);
        if (this.scope.isDisposed) return;
        url = this.scope.objectUrl(new Blob([bytes], { type: "image/jpeg" }));
        this.pageThumbs.set(key, url);
      }
      box.replaceChildren(h("img", { src: url, alt: "", draggable: false }));
    } catch {
      box.replaceChildren(icon("alert", 22));
    }
  }

  private renderCards(): HTMLElement {
    const many = this.config.multiple;
    const toolbar = h(
      "div",
      { class: "files-toolbar" },
      h("span", { text: tn("common.file", "common.files", this.entries.length) }),
      h("span", { class: "spacer" }),
      this.config.reorder && this.entries.length > 1
        ? h(
            "button",
            { class: "btn btn-ghost", on: { click: () => this.sortByName() } },
            icon("list", 18),
            t("common.sortAz"),
          )
        : null,
      h(
        "button",
        { class: "btn", on: { click: () => void this.pick() } },
        icon("add", 18),
        many ? t("drop.addFiles") : t(this.config.pickLabel),
      ),
    );
    const grid = h("div", { class: "files-grid" });
    this.entries.forEach((entry, index) => grid.appendChild(this.renderCard(entry, index)));
    if (many) {
      grid.appendChild(
        h(
          "button",
          { class: "add-card", on: { click: () => void this.pick() } },
          h("span", { class: "plus" }, icon("plus", 22)),
          t("drop.addFiles"),
        ),
      );
    }
    return h("div", null, toolbar, grid);
  }

  private renderCard(entry: Entry, index: number): HTMLElement {
    const thumbUrl = this.thumbs.get(entry.id);
    let thumb: HTMLElement;
    if (entry.status === "locked") {
      thumb = h(
        "div",
        { class: "lock" },
        icon("lock", 30),
        t("password.required"),
        h("button", { class: "btn", on: { click: () => void this.unlock(entry) } }, icon("unlock", 16), t("password.unlock")),
      );
    } else if (entry.status === "error") {
      thumb = h("div", { class: "lock" }, icon("alert", 30));
    } else if (KIND_ICON[entry.file.kind]) {
      // Pas d'aperçu pour les documents Office : leur icône.
      thumb = h("div", { class: "doc-thumb" }, icon(KIND_ICON[entry.file.kind]!, 56));
    } else if (thumbUrl) {
      thumb = h("img", {
        src: thumbUrl,
        alt: "",
        draggable: false,
        class: entry.rotation % 180 === 90 ? "rot-side" : "",
        style: entry.rotation ? { transform: `rotate(${entry.rotation}deg)` } : {},
      });
    } else {
      thumb = h("div", { class: "spinner", style: { width: "28px", height: "28px", "border-width": "3px" } });
    }

    const sub =
      entry.status === "error" && entry.error
        ? h("div", { class: "file-sub error", text: errorText({ ...entry.error, file: null }) })
        : h(
            "div",
            { class: "file-sub" },
            entry.info ? `${tn("common.page", "common.pages", entry.info.pageCount)} · ` : "",
            formatSize(entry.file.size),
          );

    const actions = h("div", { class: "file-actions" });
    const action = (name: IconName, label: string, fn: () => void, disabled = false) =>
      h(
        "button",
        {
          class: "icon-btn sm",
          title: label,
          disabled,
          attrs: { "aria-label": label },
          on: {
            click: (e) => {
              e.stopPropagation();
              fn();
            },
          },
        },
        icon(name, 16),
      );
    if (this.config.reorder && this.entries.length > 1) {
      actions.append(
        action("chevronLeft", t("common.moveLeft"), () => this.moveEntry(index, index - 1), index === 0),
        action("chevronRight", t("common.moveRight"), () => this.moveEntry(index, index + 1), index === this.entries.length - 1),
      );
    }
    if (this.config.rotate) {
      actions.append(
        action("rotate", t("common.rotate"), () => {
          entry.rotation = (entry.rotation + 90) % 360;
          this.refreshFiles();
        }),
      );
    }
    actions.append(action("trash", t("common.remove"), () => this.removeEntry(entry.id)));

    const card = h(
      "div",
      {
        class: `file-card${entry.status === "error" ? " has-error" : ""}`,
        dataset: { id: entry.id },
        title: entry.file.name,
      },
      this.config.reorder && this.entries.length > 1 ? h("span", { class: "order-badge", text: String(index + 1) }) : null,
      this.config.reorder && this.entries.length > 1
        ? h("span", { class: "drag-handle", title: t("common.dragHandle"), attrs: { "aria-hidden": "true" } }, icon("grip", 16))
        : null,
      h("div", { class: "thumb" }, thumb),
      actions,
      h("div", { class: "file-meta" }, h("div", { class: "file-name", text: entry.file.name }), sub),
    );
    if (this.config.reorder) this.enableDrag(card, index);
    return card;
  }

  /**
   * Réorganisation au pointeur (le glisser-déposer HTML5 est réservé aux
   * fichiers). À la souris, toute la carte se glisse ; au doigt, seule la
   * poignée, pour que le reste de la carte laisse défiler la liste.
   */
  private enableDrag(card: HTMLElement, index: number): void {
    card.style.cursor = "grab";
    card.addEventListener("pointerdown", (down) => {
      if (down.button !== 0 || (down.target as Element).closest("button")) return;
      if (down.pointerType !== "mouse" && !(down.target as Element).closest(".drag-handle")) return;
      if (!down.isPrimary) return;
      const pointerId = down.pointerId;
      const startX = down.clientX;
      const startY = down.clientY;
      const rect = card.getBoundingClientRect();
      let ghost: HTMLElement | null = null;
      let target: { index: number; after: boolean } | null = null;
      let highlighted: Element | null = null;

      const move = (ev: PointerEvent) => {
        if (ev.pointerId !== pointerId) return;
        if (!ghost) {
          if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 6) return;
          ghost = card.cloneNode(true) as HTMLElement;
          Object.assign(ghost.style, {
            position: "fixed",
            width: `${rect.width}px`,
            pointerEvents: "none",
            zIndex: "100",
            opacity: "0.92",
            transform: "rotate(2deg)",
            boxShadow: "var(--shadow-lg)",
          });
          document.body.appendChild(ghost);
          card.classList.add("is-dragging");
        }
        ghost.style.left = `${ev.clientX - (startX - rect.left)}px`;
        ghost.style.top = `${ev.clientY - (startY - rect.top)}px`;
        const over = document.elementFromPoint(ev.clientX, ev.clientY)?.closest(".file-card");
        if (highlighted && highlighted !== over) highlighted.classList.remove("is-drop-target");
        highlighted = null;
        target = null;
        if (over && over !== card) {
          const overIndex = this.entries.findIndex((e) => e.id === (over as HTMLElement).dataset.id);
          if (overIndex >= 0) {
            const r = over.getBoundingClientRect();
            target = { index: overIndex, after: ev.clientX > r.left + r.width / 2 };
            over.classList.add("is-drop-target");
            highlighted = over;
          }
        }
      };
      const finish = (commit: boolean) => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onCancel);
        window.removeEventListener("blur", onBlur);
        highlighted?.classList.remove("is-drop-target");
        card.classList.remove("is-dragging");
        if (ghost) {
          ghost.remove();
          if (commit && target) {
            let to = target.index + (target.after ? 1 : 0);
            if (to > index) to -= 1;
            this.moveEntry(index, to);
          }
        }
      };
      // Geste interrompu (défilement, perte du pointeur, changement de fenêtre) : on annule proprement.
      const onUp = (ev: PointerEvent) => ev.pointerId === pointerId && finish(true);
      const onCancel = (ev: PointerEvent) => ev.pointerId === pointerId && finish(false);
      const onBlur = () => finish(false);
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onCancel);
      window.addEventListener("blur", onBlur);
    });
  }

  // --- Traitement -------------------------------------------------------------

  private async run(): Promise<void> {
    const jobId = `job-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const channel = new Channel<Progress>();
    this.stage = "working";
    clear(this.body);

    const title = h("h2", { text: t("work.choosing"), attrs: { "aria-live": "polite" } });
    const hint = h("p", { text: t("work.choosingHint") });
    const bar = h("div", { class: "progress-bar" });
    const progress = h(
      "div",
      {
        class: "progress indeterminate",
        style: { display: "none" },
        attrs: { role: "progressbar", "aria-label": t("work.processing"), "aria-valuemin": 0, "aria-valuemax": 100 },
      },
      bar,
    );
    const spinner = h("div", { class: "spinner" });
    const cancel = h("button", {
      class: "btn",
      text: t("common.cancel"),
      style: { display: "none" },
      on: {
        click: () => {
          cancel.disabled = true;
          title.textContent = t("work.cancelling");
          void api.cancelJob(jobId);
        },
      },
    });
    this.body.appendChild(
      h("div", { class: "center-stage" }, h("div", { class: "status-card" }, spinner, title, hint, progress, cancel)),
    );

    let started = false;
    channel.onmessage = (p) => {
      if (!started) {
        started = true;
        spinner.style.display = "none";
        progress.style.display = "";
        cancel.style.display = "";
        title.textContent = t("work.processing");
        hint.textContent = t("work.processingHint");
      }
      if (p.total > 0) {
        const pct = Math.round((p.done / p.total) * 100);
        progress.classList.remove("indeterminate");
        bar.style.width = `${pct}%`;
        progress.setAttribute("aria-valuenow", String(pct));
        progress.setAttribute("aria-valuetext", `${pct} %`);
      }
    };
    try {
      const outcome = await this.config.run(this, jobId, channel);
      if (this.scope.isDisposed) return;
      if (outcome.cancelled) {
        this.stage = "files";
        this.render();
        return;
      }
      this.showResult(outcome);
    } catch (e) {
      if (this.scope.isDisposed) return;
      this.showError(toCommandError(e));
    }
  }

  private showResult(outcome: JobOutcome): void {
    this.stage = "result";
    clear(this.body);
    const singlePdf = outcome.outputs.length === 1 && outcome.outputs[0]?.path.toLowerCase().endsWith(".pdf");
    const first = outcome.outputs[0];
    const failures = outcome.items.filter((i) => i.error);
    const actions = h("div", { class: "result-actions" });

    if (singlePdf && first) {
      actions.append(
        h(
          "button",
          {
            class: "btn btn-primary btn-lg",
            on: {
              click: () =>
                this.ctx.navigate({
                  name: "viewer",
                  file: { path: first.path, name: baseName(first.path), size: first.size, kind: "pdf" },
                }),
            },
          },
          icon("paper", 20),
          t("work.openViewer"),
        ),
      );
    }
    if (first) {
      actions.append(
        h(
          "button",
          {
            class: `btn btn-lg ${singlePdf ? "" : "btn-primary"}`,
            on: { click: () => void api.revealInFolder(first.path).catch((e) => toast(errorText(toCommandError(e)), "error")) },
          },
          icon("folder", 20),
          t("work.reveal"),
        ),
      );
    }

    const details = this.config.resultDetails?.(outcome) ?? null;
    const createdLine =
      outcome.outputs.length > 1
        ? h("p", { text: t("work.filesCreated", { n: outcome.outputs.length }) })
        : first
          ? h("p", { class: "selectable", text: t("work.fileCreated", { name: baseName(first.path) }) })
          : null;

    this.body.appendChild(
      h(
        "div",
        { class: "center-stage" },
        h(
          "div",
          { class: "status-card" },
          h("div", { class: failures.length && !first ? "error-mark" : "success-mark" }, icon(failures.length && !first ? "alert" : "check", 38)),
          h("h2", { text: first ? this.config.resultTitle(outcome) : t("work.errorTitle"), attrs: { tabindex: -1, "data-autofocus": "" } }),
          details,
          createdLine,
          failures.length
            ? h(
                "div",
                { class: "result-list" },
                h("div", { class: "result-row" }, h("span", { class: "err", text: t("work.partial", { ok: outcome.items.length - failures.length, ko: failures.length }) })),
                failures.map((f) =>
                  h("div", { class: "result-row" }, h("span", { class: "name", text: f.name }), h("span", { class: "err", text: f.error ? errorText({ ...f.error, file: null }) : "" })),
                ),
              )
            : null,
          outcome.hadPassword ? h("div", { class: "info-box warning" }, icon("unlock", 18), t("password.notice")) : null,
          actions,
          h(
            "div",
            { class: "result-actions" },
            h("button", { class: "btn btn-ghost", on: { click: () => this.restart() } }, t("common.restart")),
            h("button", { class: "btn btn-ghost", on: { click: () => this.ctx.navigate({ name: "home" }) } }, t("work.backToTools")),
          ),
        ),
      ),
    );
    this.focusHeading();
  }

  /** Place le focus sur le titre de l'écran qui vient d'apparaître. */
  private focusHeading(): void {
    requestAnimationFrame(() => this.body.querySelector<HTMLElement>("[data-autofocus]")?.focus({ preventScroll: true }));
  }

  private showError(err: CommandError): void {
    this.stage = "error";
    clear(this.body);
    this.body.appendChild(
      h(
        "div",
        { class: "center-stage" },
        h(
          "div",
          { class: "status-card" },
          h("div", { class: "error-mark" }, icon("alert", 38)),
          h("h2", { text: t("work.errorTitle"), attrs: { tabindex: -1, "data-autofocus": "" } }),
          h("p", { text: errorText(err) }),
          err.detail && err.code !== "cancelled"
            ? h("details", { class: "field-help selectable" }, h("summary", { text: t("error.details") }), err.detail)
            : null,
          h(
            "div",
            { class: "result-actions" },
            h(
              "button",
              {
                class: "btn btn-primary",
                on: {
                  click: () => {
                    this.stage = this.entries.length ? "files" : "empty";
                    this.render();
                  },
                },
              },
              t("common.retry"),
            ),
            h("button", { class: "btn btn-ghost", on: { click: () => this.ctx.navigate({ name: "home" }) } }, t("work.backToTools")),
          ),
        ),
      ),
    );
    this.focusHeading();
  }

  private restart(): void {
    this.entries.forEach((e) => this.releaseThumb(e.id));
    this.entries = [];
    this.stage = "empty";
    this.render();
  }
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

// ---------------------------------------------------------------------------
// Petits composants d'options, partagés par les outils
// ---------------------------------------------------------------------------

let fieldSeq = 0;

/**
 * Groupe « libellé + champ ». Le libellé nomme le premier champ ou groupe
 * de boutons qu'il contient (lecteurs d'écran).
 */
export function field(label: string, ...content: (HTMLElement | null)[]): HTMLElement {
  const id = `field-${++fieldSeq}`;
  const wrap = h("div", { class: "field" }, h("span", { class: "field-label", text: label, attrs: { id } }), ...content);
  const control = wrap.querySelector(
    "input:not([type=checkbox]):not([type=color]), select, textarea, [role=group], [role=radiogroup]",
  );
  if (control && !control.hasAttribute("aria-label") && !control.hasAttribute("aria-labelledby")) {
    control.setAttribute("aria-labelledby", id);
  }
  return wrap;
}

export function segmented<T extends string>(
  options: { value: T; label: string }[],
  value: T,
  onChange: (v: T) => void,
  label?: string,
): HTMLElement {
  return h(
    "div",
    { class: "segmented", attrs: { role: "group", "aria-label": label } },
    options.map((o) =>
      h("button", {
        type: "button",
        text: o.label,
        attrs: { "aria-pressed": String(o.value === value) },
        on: { click: () => onChange(o.value) },
      }),
    ),
  );
}

export function choices<T extends string>(
  options: { value: T; title: string; desc?: string; tag?: string }[],
  value: T,
  onChange: (v: T) => void,
  label?: string,
): HTMLElement {
  return h(
    "div",
    { class: "choice-list", attrs: { role: "radiogroup", "aria-label": label } },
    options.map((o) =>
      h(
        "button",
        {
          class: "choice",
          type: "button",
          attrs: { role: "radio", "aria-checked": String(o.value === value) },
          on: { click: () => onChange(o.value) },
        },
        h("span", { class: "radio" }),
        h(
          "span",
          null,
          h("div", { class: "choice-title" }, o.title, o.tag ? h("span", { class: "choice-tag", text: o.tag }) : null),
          o.desc ? h("div", { class: "choice-desc", text: o.desc }) : null,
        ),
      ),
    ),
  );
}

export function checkbox(label: string, checked: boolean, onChange: (v: boolean) => void): HTMLElement {
  return h(
    "label",
    { class: "check-row" },
    h("input", { type: "checkbox", checked, on: { change: (e) => onChange((e.target as HTMLInputElement).checked) } }),
    h("span", { text: label }),
  );
}

/** Empêche de lancer tant qu'un fichier est verrouillé ou illisible. */
export function entriesBlocker(page: ToolPage): string | null {
  const locked = page.entries.find((e) => e.status === "locked");
  if (locked) return t("password.blocker", { name: locked.file.name });
  const broken = page.entries.find((e) => e.status === "error");
  if (broken?.error) return errorText({ ...broken.error, file: broken.file.name });
  return null;
}

export function infoBox(text: string, kind: "" | "warning" | "error" = ""): HTMLElement {
  return h("div", { class: `info-box ${kind}` }, icon(kind ? "alert" : "info", 18), h("span", { text }));
}
