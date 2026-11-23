import { api } from "../../api";
import { t, tn } from "../../i18n";
import { entriesBlocker, infoBox, type ToolConfig } from "../tool";

export const mergeConfig: ToolConfig = {
  multiple: true,
  reorder: true,
  pickLabel: "drop.selectPdf",

  sidebar(page) {
    const pages = page.ready.reduce((sum, e) => sum + (e.info?.pageCount ?? 0), 0);
    return {
      body: [infoBox(t("merge.help"))],
      action: t("merge.action"),
      blocker: entriesBlocker(page) ?? (page.ready.length < 2 ? t("merge.needTwo") : null),
      summary: `${tn("common.file", "common.files", page.ready.length)} · ${tn("common.page", "common.pages", pages)}`,
    };
  },

  run(page, jobId, progress) {
    return api.mergePdfs(
      jobId,
      page.ready.map((e) => page.pdfInput(e)),
      t("merge.outputName"),
      progress,
    );
  },

  resultTitle: () => t("merge.done"),
};
