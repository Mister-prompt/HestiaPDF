import { api, type ImageExport } from "../../api";
import { t } from "../../i18n";
import { choices, entriesBlocker, field, infoBox, segmented, type ToolConfig } from "../tool";

let mode: "pages" | "extract" = "pages";
let quality: "normal" | "high" = "normal";

export const pdfToJpgConfig: ToolConfig = {
  multiple: true,
  pickLabel: "drop.selectPdf",

  sidebar(page) {
    const body = [
      choices(
        [
          { value: "pages", title: t("pdfToJpg.pages"), desc: t("pdfToJpg.pagesDesc") },
          { value: "extract", title: t("pdfToJpg.extract"), desc: t("pdfToJpg.extractDesc") },
        ],
        mode,
        (v) => {
          mode = v;
          page.refreshSidebar();
        },
        t("pdfToJpg.modeLabel"),
      ),
    ];
    if (mode === "pages") {
      body.push(
        field(
          t("pdfToJpg.quality"),
          segmented(
            [
              { value: "normal", label: `${t("pdfToJpg.normal")} · ${t("pdfToJpg.normalHint")}` },
              { value: "high", label: `${t("pdfToJpg.high")} · ${t("pdfToJpg.highHint")}` },
            ],
            quality,
            (v) => {
              quality = v;
              page.refreshSidebar();
            },
          ),
        ),
      );
    }
    body.push(infoBox(t("pdfToJpg.folderInfo")));
    return { body, action: t("pdfToJpg.action"), blocker: entriesBlocker(page) };
  },

  run(page, jobId, progress) {
    const exportMode: ImageExport = mode === "pages" ? { mode: "pages", quality } : { mode: "extract" };
    return api.pdfToJpg(
      jobId,
      page.ready.map((e) => page.pdfInput(e)),
      exportMode,
      progress,
    );
  },

  resultTitle: () => t("pdfToJpg.done"),
};
