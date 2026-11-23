import { api, type CompressionLevel, type JobOutcome } from "../../api";
import { h } from "../../dom";
import { formatSize, t } from "../../i18n";
import { icon } from "../../icons";
import { choices, entriesBlocker, field, infoBox, type ToolConfig } from "../tool";

// Le niveau choisi est mémorisé pendant la session.
let level: CompressionLevel = "recommended";

function gainPercent(before: number, after: number): number {
  return before > 0 ? Math.max(0, Math.round((1 - after / before) * 100)) : 0;
}

export const compressConfig: ToolConfig = {
  multiple: true,
  pickLabel: "drop.selectPdf",

  sidebar(page) {
    return {
      body: [
        field(
          t("compress.level"),
          choices<CompressionLevel>(
            [
              { value: "extreme", title: t("compress.extreme"), desc: t("compress.extremeDesc") },
              {
                value: "recommended",
                title: t("compress.recommended"),
                desc: t("compress.recommendedDesc"),
                tag: t("compress.recommendedTag"),
              },
              { value: "low", title: t("compress.low"), desc: t("compress.lowDesc") },
            ],
            level,
            (v) => {
              level = v;
              page.refreshSidebar();
            },
          ),
        ),
        page.ready.length > 1 ? infoBox(t("compress.multiInfo")) : null,
      ].filter((x): x is HTMLElement => x !== null),
      action: t("compress.action"),
      blocker: entriesBlocker(page),
    };
  },

  run(page, jobId, progress) {
    return api.compressPdfs(
      jobId,
      page.ready.map((e) => page.pdfInput(e)),
      level,
      t("compress.suffix"),
      progress,
    );
  },

  resultTitle: (o) => (o.items.length > 1 ? t("compress.done") : t("compress.doneOne")),

  resultDetails(outcome: JobOutcome) {
    const ok = outcome.items.filter((i) => !i.error && i.originalSize !== null && i.compressedSize !== null);
    if (ok.length === 0) return null;
    if (outcome.items.length === 1) {
      const item = ok[0]!;
      const before = item.originalSize ?? 0;
      const after = item.compressedSize ?? 0;
      if (item.alreadyOptimal) {
        return h("div", null, h("div", { class: "info-box" }, icon("info", 18), h("span", null, h("strong", { text: t("compress.already") }), " — ", t("compress.alreadyText"))));
      }
      return h(
        "div",
        { style: { display: "flex", "flex-direction": "column", "align-items": "center", gap: "8px" } },
        h("div", { class: "big-gain", text: `−${gainPercent(before, after)} %` }),
        h("div", { class: "size-line" }, formatSize(before), icon("arrowRight", 18), formatSize(after)),
      );
    }
    return h(
      "div",
      { class: "result-list" },
      ok.map((item) =>
        h(
          "div",
          { class: "result-row" },
          h("span", { class: "name", text: item.name }),
          h("span", { text: `${formatSize(item.originalSize ?? 0)} → ${formatSize(item.compressedSize ?? 0)}` }),
          h("span", {
            class: "gain",
            text: item.alreadyOptimal
              ? t("compress.already")
              : t("compress.saved", { pct: gainPercent(item.originalSize ?? 0, item.compressedSize ?? 0) }),
          }),
        ),
      ),
    );
  },
};
