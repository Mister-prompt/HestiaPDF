// Accueil : grille des outils, comme sur iLovePDF, mais 100 % local.

import type { ViewContext } from "../app";
import { h, richText } from "../dom";
import { t, type Key } from "../i18n";
import { icon } from "../icons";
import { TOOLS, descKey, titleKey, type Category } from "../tools";

let lastFilter: Category | "all" = "all";

export function homeView(ctx: ViewContext): HTMLElement {
  const grid = h("div", { class: "tools-grid" });

  const renderGrid = () => {
    grid.replaceChildren(
      ...TOOLS.filter((tool) => lastFilter === "all" || tool.category === lastFilter).map((tool) => {
        const descId = `tool-desc-${tool.id}`;
        return h(
          "article",
          { class: `tool-card${tool.ready ? "" : " is-soon"}` },
          tool.ready ? null : h("span", { class: "badge badge-soon", text: t("home.soon") }),
          h("span", { class: "tool-icon" }, icon(tool.icon, 44)),
          h(
            "h2",
            { class: "tool-title" },
            h("button", {
              class: "tool-link",
              type: "button",
              text: t(titleKey(tool.id)),
              attrs: { "aria-describedby": descId },
              on: { click: () => ctx.navigate({ name: "tool", id: tool.id }) },
            }),
          ),
          h("p", { text: t(descKey(tool.id)), attrs: { id: descId } }),
        );
      }),
    );
  };

  const filters: (Category | "all")[] = ["all", "organize", "optimize", "convert", "edit"];
  const chips = h(
    "div",
    { class: "filters", attrs: { role: "group" } },
    filters.map((f) =>
      h("button", {
        class: "chip",
        type: "button",
        text: t(`home.filter.${f}` as Key),
        attrs: { "aria-pressed": String(f === lastFilter) },
        on: {
          click: (e) => {
            lastFilter = f;
            chips.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", "false"));
            (e.currentTarget as HTMLElement).setAttribute("aria-pressed", "true");
            renderGrid();
          },
        },
      }),
    ),
  );

  renderGrid();

  // Déposer un PDF sur l'accueil l'ouvre ; des images vont vers « JPG en PDF ».
  ctx.onFiles((files) => {
    const images = files.filter((f) => f.kind === "image");
    const pdf = files.find((f) => f.kind === "pdf");
    if (images.length > 0 && !pdf) ctx.navigate({ name: "tool", id: "jpgToPdf", files: images });
    else if (pdf) {
      const pdfs = files.filter((f) => f.kind === "pdf");
      if (pdfs.length > 1) ctx.navigate({ name: "tool", id: "merge", files: pdfs });
      else ctx.navigate({ name: "viewer", file: pdf });
    }
  });

  return h(
    "div",
    { class: "container" },
    h(
      "section",
      { class: "hero" },
      h("h1", null, richText(t("home.title"))),
      h("p", { text: t("home.subtitle") }),
    ),
    chips,
    grid,
    h("div", { class: "privacy-note" }, icon("offline", 22), h("p", null, richText(t("home.privacy")))),
    // Texte sélectionnable plutôt qu'un lien : l'appli n'ouvre jamais le réseau.
    h("p", { class: "open-source-note" }, richText(t("home.openSource", { url: "github.com/Mister-prompt/HestiaPDF" }))),
  );
}
