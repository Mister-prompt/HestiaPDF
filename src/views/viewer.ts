// Visionneuse PDF : défilement continu, zoom, rendu des pages visibles
// uniquement (rapide même pour les longs documents).

import type { ViewContext } from "../app";
import { api, toCommandError, type FileInfo, type PdfInfo, type PdfInput } from "../api";
import { h } from "../dom";
import { t } from "../i18n";
import { icon } from "../icons";
import { TOOLS, titleKey } from "../tools";
import { askPassword, errorText, toast } from "../ui";
import { zoomField } from "./zoom-field";

const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
const PT_TO_PX = 96 / 72;

interface PageSlot {
  el: HTMLElement;
  img: HTMLImageElement | null;
  renderedWidth: number;
  loading: boolean;
}

export function viewerView(ctx: ViewContext, file: FileInfo, initialPassword: string | null): HTMLElement {
  let input: PdfInput = { path: file.path, password: initialPassword };
  let info: PdfInfo | null = null;
  let zoom = 1;
  let fitMode = true;
  let fitCap = 1;
  const slots: PageSlot[] = [];
  const visible = new Set<number>();

  const pagesEl = h("div", { class: "viewer-pages" });
  const scroller = h("div", { class: "viewer-scroll" }, pagesEl);
  const indicator = h("span", { class: "page-indicator" });
  const zoomInput = zoomField(MIN_ZOOM, MAX_ZOOM, (z) => setZoom(z));

  const btn = (name: Parameters<typeof icon>[0], label: string, fn: () => void) =>
    h("button", { class: "icon-btn", title: label, attrs: { "aria-label": label }, on: { click: fn } }, icon(name, 18));

  const toolSelect = h(
    "select",
    {
      class: "select",
      attrs: { "aria-label": t("viewer.tools") },
      on: {
        change: (e) => {
          const id = (e.target as HTMLSelectElement).value;
          const tool = TOOLS.find((x) => x.id === id);
          if (tool) ctx.navigate({ name: "tool", id: tool.id, files: [file] });
        },
      },
    },
    h("option", { value: "", text: t("viewer.tools"), selected: true, disabled: true }),
    TOOLS.filter((x) => x.ready && x.accepts === "pdf").map((x) => h("option", { value: x.id, text: t(titleKey(x.id)) })),
  );

  const bar = h(
    "div",
    { class: "viewer-bar" },
    h(
      "button",
      { class: "btn btn-ghost", on: { click: () => ctx.navigate({ name: "home" }) } },
      icon("arrowLeft", 18),
      t("common.back"),
    ),
    h("span", { class: "sep" }),
    h("span", { class: "viewer-title selectable", text: file.name, title: file.name }),
    h("span", { class: "header-spacer" }),
    btn("chevronLeft", t("viewer.prev"), () => goTo(currentPage() - 1)),
    indicator,
    btn("chevronRight", t("viewer.next"), () => goTo(currentPage() + 1)),
    h("span", { class: "sep" }),
    btn("minus", t("viewer.zoomOut"), () => setZoom(zoom / 1.2)),
    zoomInput.el,
    btn("plus", t("viewer.zoomIn"), () => setZoom(zoom * 1.2)),
    h("button", { class: "btn btn-ghost", text: t("viewer.fit"), on: { click: () => fitWidth() } }),
    h("span", { class: "sep" }),
    toolSelect,
  );

  const root = h("div", { class: "viewer" }, bar, scroller);

  // --- Rendu -----------------------------------------------------------------

  const observer = new IntersectionObserver(
    (items) => {
      for (const item of items) {
        const index = Number((item.target as HTMLElement).dataset.index);
        if (item.isIntersecting) visible.add(index);
        else visible.delete(index);
      }
      renderVisible();
    },
    { root: scroller, rootMargin: "600px 0px" },
  );
  ctx.scope.add(() => observer.disconnect());

  function pageCssSize(index: number): { w: number; h: number } {
    const size = info?.pages[index] ?? { width: 595, height: 842 };
    return { w: size.width * PT_TO_PX * zoom, h: size.height * PT_TO_PX * zoom };
  }

  function layout(): void {
    slots.forEach((slot, i) => {
      const { w, h: height } = pageCssSize(i);
      slot.el.style.width = `${w}px`;
      slot.el.style.height = `${height}px`;
    });
    zoomInput.set(zoom);
  }

  function targetWidth(index: number): number {
    const css = pageCssSize(index).w * (window.devicePixelRatio || 1);
    // Par paliers de 200 px pour éviter de redessiner à chaque cran de zoom.
    return Math.min(3000, Math.max(200, Math.ceil(css / 200) * 200));
  }

  let renderTimer = 0;
  function renderVisible(delay = 0): void {
    window.clearTimeout(renderTimer);
    renderTimer = window.setTimeout(() => {
      for (const index of [...visible].sort((a, b) => a - b)) void renderPage(index);
    }, delay);
  }
  ctx.scope.add(() => window.clearTimeout(renderTimer));

  async function renderPage(index: number): Promise<void> {
    const slot = slots[index];
    if (!slot || slot.loading) return;
    const width = targetWidth(index);
    if (slot.renderedWidth >= width) return;
    slot.loading = true;
    try {
      const bytes = await api.renderPage(input, index, width);
      if (ctx.scope.isDisposed) return;
      const url = ctx.scope.objectUrl(new Blob([bytes], { type: "image/jpeg" }));
      const old = slot.img;
      const img = h("img", { src: url, alt: "", draggable: false });
      if (old) {
        old.replaceWith(img);
        URL.revokeObjectURL(old.src);
      } else {
        slot.el.prepend(img);
      }
      slot.img = img;
      slot.renderedWidth = width;
    } catch (e) {
      if (!ctx.scope.isDisposed && index === 0) toast(errorText(toCommandError(e)), "error");
    } finally {
      slot.loading = false;
    }
    // Le zoom a pu changer pendant le rendu.
    if (!ctx.scope.isDisposed && visible.has(index) && slot.renderedWidth < targetWidth(index)) void renderPage(index);
  }

  function buildPages(): void {
    if (!info) return;
    pagesEl.replaceChildren();
    slots.length = 0;
    for (let i = 0; i < info.pageCount; i++) {
      const el = h("div", { class: "viewer-page", dataset: { index: String(i) } }, h("span", { class: "page-num", text: String(i + 1) }));
      slots.push({ el, img: null, renderedWidth: 0, loading: false });
      pagesEl.appendChild(el);
      observer.observe(el);
    }
    // À l'ouverture : la page entière en largeur, sans dépasser la taille réelle.
    fitWidth(1);
    updateIndicator();
  }

  // --- Navigation & zoom -----------------------------------------------------

  function currentPage(): number {
    const mid = scroller.scrollTop + scroller.clientHeight / 3;
    let current = 0;
    for (let i = 0; i < slots.length; i++) {
      if (slots[i]!.el.offsetTop <= mid) current = i;
      else break;
    }
    return current;
  }

  function updateIndicator(): void {
    if (!info) return;
    indicator.textContent = t("viewer.page", { n: currentPage() + 1, total: info.pageCount });
  }

  function goTo(index: number): void {
    const slot = slots[Math.max(0, Math.min(slots.length - 1, index))];
    const calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (slot) scroller.scrollTo({ top: slot.el.offsetTop - 20, behavior: calm ? "auto" : "smooth" });
  }

  function setZoom(value: number, anchorRatio?: number): void {
    const ratio = anchorRatio ?? (scroller.scrollTop + scroller.clientHeight / 2) / Math.max(1, scroller.scrollHeight);
    const ratioX = (scroller.scrollLeft + scroller.clientWidth / 2) / Math.max(1, scroller.scrollWidth);
    zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, value));
    fitMode = false;
    layout();
    scroller.scrollTop = ratio * scroller.scrollHeight - scroller.clientHeight / 2;
    // Garde aussi le même point au centre horizontalement.
    scroller.scrollLeft = ratioX * scroller.scrollWidth - scroller.clientWidth / 2;
    renderVisible(180);
  }

  /** Ajuste à la largeur ; `cap` limite le zoom (1 = taille réelle à l'ouverture). */
  function fitWidth(cap = MAX_ZOOM): void {
    if (!info || info.pages.length === 0) return;
    const widest = Math.max(...info.pages.map((p) => p.width)) * PT_TO_PX;
    const available = Math.max(200, scroller.clientWidth - 60);
    fitCap = cap;
    zoom = Math.min(cap, MAX_ZOOM, Math.max(MIN_ZOOM, available / widest));
    fitMode = true;
    layout();
    renderVisible(60);
  }

  let scrollRaf = 0;
  scroller.addEventListener("scroll", () => {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      updateIndicator();
    });
  });

  // Ctrl + molette : zoom.
  scroller.addEventListener(
    "wheel",
    (e) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      setZoom(zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
    },
    { passive: false },
  );

  const onKey = (e: KeyboardEvent) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.key === "+" || e.key === "=") {
      e.preventDefault();
      setZoom(zoom * 1.2);
    } else if (e.key === "-") {
      e.preventDefault();
      setZoom(zoom / 1.2);
    } else if (e.key === "0") {
      e.preventDefault();
      fitWidth();
    }
  };
  document.addEventListener("keydown", onKey);
  ctx.scope.add(() => document.removeEventListener("keydown", onKey));

  const resize = new ResizeObserver(() => {
    if (fitMode) fitWidth(fitCap);
  });
  resize.observe(scroller);
  ctx.scope.add(() => resize.disconnect());

  // Un autre PDF déposé s'ouvre à la place.
  ctx.onFiles((files) => {
    const pdf = files.find((f) => f.kind === "pdf");
    if (pdf) ctx.navigate({ name: "viewer", file: pdf });
    else toast(t("drop.unsupported"), "error");
  });

  // --- Ouverture -------------------------------------------------------------

  async function open(): Promise<void> {
    try {
      info = await api.inspectPdf(input);
    } catch (e) {
      const err = toCommandError(e);
      if (err.code === "password_required" || err.code === "wrong_password") {
        const pw = await askPassword(file.name, async (candidate) => {
          try {
            info = await api.inspectPdf({ path: file.path, password: candidate });
            return true;
          } catch {
            return false;
          }
        });
        if (!pw) {
          ctx.navigate({ name: "home" });
          return;
        }
        input = { path: file.path, password: pw };
      } else {
        toast(errorText(err), "error");
        ctx.navigate({ name: "home" });
        return;
      }
    }
    if (ctx.scope.isDisposed) return;
    // Laisse le temps à la mise en page de donner la largeur disponible.
    requestAnimationFrame(buildPages);
  }

  ctx.keepState(() => ({ name: "viewer", file, password: input.password ?? null }));
  void open();
  return root;
}
