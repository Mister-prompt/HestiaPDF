// Outils prévus dans une prochaine version.

import type { ViewContext } from "../app";
import { h } from "../dom";
import { t } from "../i18n";
import { icon } from "../icons";
import { descKey, titleKey, toolById, type ToolId } from "../tools";

export function soonView(ctx: ViewContext, id: ToolId): HTMLElement {
  const tool = toolById(id);
  return h(
    "div",
    { class: "tool-view" },
    h(
      "div",
      { class: "tool-head" },
      h(
        "button",
        { class: "btn btn-ghost back", on: { click: () => ctx.navigate({ name: "home" }) } },
        icon("arrowLeft", 18),
        t("common.back"),
      ),
    ),
    h(
      "div",
      { class: "center-stage" },
      h(
        "div",
        { class: "status-card" },
        h("span", { class: "big-tool-icon" }, icon(tool.icon, 64)),
        h("h2", { text: t(titleKey(id)) }),
        h("p", { text: t(descKey(id)) }),
        h("div", { class: "info-box" }, icon("sparkles", 18), h("span", null, h("strong", { text: t("soon.title") }), " — ", t("soon.text"))),
        h(
          "button",
          { class: "btn btn-primary", on: { click: () => ctx.navigate({ name: "home" }) } },
          t("common.back"),
        ),
      ),
    ),
  );
}
