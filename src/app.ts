// Coquille de l'application : en-tête, réglages, navigation entre les vues,
// glisser-déposer global.

import { getCurrentWindow } from "@tauri-apps/api/window";
import logoUrl from "./assets/logo.svg";
import { api, onDrag, onDrop, toCommandError, type FileInfo } from "./api";
import { clear, h, Scope } from "./dom";
import { getLang, onLangChange, setLang, t, type Lang } from "./i18n";
import { icon } from "./icons";
import { readSetting, writeSetting } from "./storage";
import { TOOLS, type ToolId } from "./tools";
import { errorText, toast, trapFocus } from "./ui";
import { homeView } from "./views/home";
import { soonView } from "./views/soon";
import { toolView } from "./views/tool";
import { viewerView } from "./views/viewer";

export type Route =
  | { name: "home" }
  | { name: "tool"; id: ToolId; files?: FileInfo[] }
  | { name: "viewer"; file: FileInfo; password?: string | null };

export interface ViewContext {
  scope: Scope;
  navigate: (route: Route) => void;
  /** La vue accepte les fichiers déposés depuis l'Explorateur. */
  onFiles: (handler: (files: FileInfo[]) => void) => void;
  /** Permet de retrouver l'état de la vue après un changement de langue. */
  keepState: (snapshot: () => Route) => void;
  /** Appelée avant de quitter la vue ; renvoie false pour rester. */
  setLeaveGuard: (guard: () => Promise<boolean>) => void;
}

type ThemePref = "system" | "light" | "dark";

function applyTheme(pref: ThemePref): void {
  if (pref === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", pref);
}

export function initTheme(): void {
  const saved = readSetting("theme");
  applyTheme(saved === "light" || saved === "dark" ? saved : "system");
}

export class App {
  private header = h("header", { class: "app-header" });
  private main = h("main", { class: "app-main", attrs: { id: "main" } });
  private overlay = h(
    "div",
    { class: "drop-overlay", attrs: { "aria-hidden": "true" } },
    h("div", { class: "drop-box" }, icon("add", 40), h("span", { text: t("drop.overlay") })),
  );
  private menu: HTMLElement | null = null;
  private menuToggle: HTMLButtonElement | null = null;
  private menuKeys = (e: KeyboardEvent) => {
    if (!this.menu) return;
    if (e.key === "Escape") {
      e.preventDefault();
      this.closeMenu(true);
    } else if (e.key === "Tab") {
      trapFocus(e, this.menu);
    }
  };
  private route: Route = { name: "home" };
  private scope = new Scope();
  private dropHandler: ((files: FileInfo[]) => void) | null = null;
  private snapshot: (() => Route) | null = null;
  private leaveGuard: (() => Promise<boolean>) | null = null;
  private version = "";

  constructor(private root: HTMLElement) {}

  async start(): Promise<void> {
    clear(this.root);
    this.root.append(this.header, this.main, this.overlay);
    this.renderHeader();

    onLangChange(() => {
      this.renderHeader();
      this.overlay.querySelector("span")!.textContent = t("drop.overlay");
      this.navigate(this.snapshot ? this.snapshot() : this.route);
    });

    await onDrag(({ active }) => {
      this.overlay.classList.toggle("is-visible", active && this.dropHandler !== null);
    });
    await onDrop((files) => {
      this.overlay.classList.remove("is-visible");
      if (!this.dropHandler) return;
      if (files.length === 0) {
        toast(t("drop.unsupported"), "error");
        return;
      }
      this.dropHandler(files);
    });

    // Fermeture de la fenêtre : même protection que la navigation.
    try {
      const win = getCurrentWindow();
      await win.onCloseRequested(async (event) => {
        const guard = this.leaveGuard;
        if (!guard) return;
        event.preventDefault();
        if (await guard()) {
          this.leaveGuard = null;
          await win.destroy();
        }
      });
    } catch {
      /* hors de Tauri (aperçu) : sans objet */
    }

    document.addEventListener("mousedown", (e) => {
      if (this.menu && !this.menu.contains(e.target as Node) && !(e.target as Element).closest?.("[data-menu-toggle]")) {
        this.closeMenu();
      }
    });

    api
      .appInfo()
      .then((info) => (this.version = info.version))
      .catch(() => undefined);

    // Fichier passé au lancement (« Ouvrir avec Hestia PDF »).
    const startup = await api.startupFiles().catch(() => [] as FileInfo[]);
    const firstPdf = startup.find((f) => f.kind === "pdf");
    if (firstPdf) this.navigate({ name: "viewer", file: firstPdf });
    else this.navigate({ name: "home" });
  }

  navigate = (route: Route, force = false): void => {
    const guard = this.leaveGuard;
    if (guard && !force) {
      void guard().then((ok) => ok && this.navigate(route, true));
      return;
    }
    this.leaveGuard = null;
    this.scope.dispose();
    this.scope = new Scope();
    this.dropHandler = null;
    this.snapshot = null;
    this.route = route;
    this.closeMenu();

    const ctx: ViewContext = {
      scope: this.scope,
      navigate: this.navigate,
      onFiles: (handler) => (this.dropHandler = handler),
      keepState: (fn) => (this.snapshot = fn),
      setLeaveGuard: (fn) => (this.leaveGuard = fn),
    };

    let view: HTMLElement;
    switch (route.name) {
      case "home":
        view = homeView(ctx);
        break;
      case "tool":
        view = route.id && isReady(route.id) ? toolView(ctx, route.id, route.files ?? []) : soonView(ctx, route.id);
        break;
      case "viewer":
        view = viewerView(ctx, route.file, route.password ?? null);
        break;
    }
    clear(this.main);
    this.main.scrollTop = 0;
    this.main.appendChild(view);
  };

  private renderHeader(): void {
    clear(this.header);
    const brand = h(
      "button",
      { class: "brand", title: t("app.home"), on: { click: () => this.navigate({ name: "home" }) } },
      h("img", { src: logoUrl, alt: "" }),
      h("span", { class: "brand-name" }, "Hestia ", h("b", { text: "PDF" })),
    );
    const badge = h(
      "span",
      { class: "local-badge", title: t("app.localTitle") },
      icon("offline", 16),
      t("app.local"),
    );
    const open = h(
      "button",
      { class: "btn btn-ghost", on: { click: () => void this.openPdf() } },
      icon("paper", 18),
      t("app.openPdf"),
    );
    const settings = h(
      "button",
      {
        class: "icon-btn",
        title: t("app.settings"),
        attrs: { "aria-label": t("app.settings"), "data-menu-toggle": "", "aria-expanded": "false", "aria-haspopup": "dialog" },
        on: { click: () => (this.menu ? this.closeMenu() : this.openMenu()) },
      },
      icon("gears", 20),
    );
    this.menuToggle = settings;
    // Langue : bascule directe, à côté des paramètres.
    const langs: [Lang, string, string][] = [
      ["fr", "FR", "Français"],
      ["en", "EN", "English"],
    ];
    const lang = h(
      "div",
      { class: "lang-switch", attrs: { role: "group", "aria-label": t("settings.language") } },
      langs.map(([code, short, name]) =>
        h("button", {
          type: "button",
          text: short,
          title: name,
          attrs: { lang: code, "aria-label": name, "aria-pressed": String(getLang() === code) },
          on: { click: () => setLang(code) },
        }),
      ),
    );
    this.header.append(brand, h("div", { class: "header-spacer" }), badge, open, lang, settings);
  }

  private async openPdf(): Promise<void> {
    try {
      const [file] = await api.pickFiles("pdf", false);
      if (file) this.navigate({ name: "viewer", file });
    } catch (e) {
      toast(errorText(toCommandError(e)), "error");
    }
  }

  private openMenu(): void {
    const themePref = (readSetting("theme") as ThemePref | null) ?? "system";
    const segmented = <T extends string>(labelId: string, options: [T, string][], value: T, onPick: (v: T) => void) => {
      const wrap = h("div", { class: "segmented", attrs: { role: "group", "aria-labelledby": labelId } });
      for (const [v, label] of options) {
        wrap.appendChild(
          h("button", {
            type: "button",
            text: label,
            attrs: { "aria-pressed": String(v === value) },
            on: {
              click: () => {
                wrap.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", "false"));
                wrap.children[options.findIndex((o) => o[0] === v)]?.setAttribute("aria-pressed", "true");
                onPick(v);
              },
            },
          }),
        );
      }
      return wrap;
    };

    this.menu = h(
      "div",
      { class: "menu", attrs: { role: "dialog", "aria-label": t("app.settings") } },
      h(
        "div",
        { class: "field" },
        h("span", { class: "field-label", text: t("settings.theme"), attrs: { id: "menu-theme" } }),
        segmented<ThemePref>(
          "menu-theme",
          [
            ["system", t("settings.system")],
            ["light", t("settings.light")],
            ["dark", t("settings.dark")],
          ],
          themePref,
          (v) => {
            writeSetting("theme", v);
            applyTheme(v);
          },
        ),
      ),
      h(
        "div",
        { class: "about" },
        h("strong", { text: t("settings.about", { version: this.version }) }),
        h("span", { text: t("settings.aboutText") }),
        h("span", { text: t("settings.credits") }),
      ),
    );
    this.header.appendChild(this.menu);
    this.menuToggle?.setAttribute("aria-expanded", "true");
    document.addEventListener("keydown", this.menuKeys, true);
    this.menu.querySelector<HTMLElement>("button[aria-pressed='true']")?.focus();
  }

  private closeMenu(returnFocus = false): void {
    if (!this.menu) return;
    this.menu.remove();
    this.menu = null;
    document.removeEventListener("keydown", this.menuKeys, true);
    this.menuToggle?.setAttribute("aria-expanded", "false");
    if (returnFocus) this.menuToggle?.focus();
  }
}

function isReady(id: ToolId): boolean {
  return TOOLS.some((tool) => tool.id === id && tool.ready);
}
