import { api, type ImagesToPdfOptions } from "../../api";
import { h } from "../../dom";
import { t } from "../../i18n";
import { checkbox, entriesBlocker, field, segmented, type ToolConfig } from "../tool";

const options: ImagesToPdfOptions = { pageSize: "a4", orientation: "auto", margin: "none" };
let mergeAll = true;

export const jpgToPdfConfig: ToolConfig = {
  multiple: true,
  reorder: true,
  rotate: true,
  pickLabel: "drop.selectImages",

  sidebar(page) {
    const select = h(
      "select",
      {
        class: "select",
        on: {
          change: (e) => {
            options.pageSize = (e.target as HTMLSelectElement).value as ImagesToPdfOptions["pageSize"];
            page.refreshSidebar();
          },
        },
      },
      (["a4", "letter", "fit"] as const).map((v) =>
        h("option", { value: v, selected: options.pageSize === v, text: t(`jpgToPdf.${v}`) }),
      ),
    );
    const body: HTMLElement[] = [field(t("jpgToPdf.pageSize"), select)];
    if (options.pageSize !== "fit") {
      body.push(
        field(
          t("jpgToPdf.orientation"),
          segmented(
            [
              { value: "auto", label: t("jpgToPdf.auto") },
              { value: "portrait", label: t("jpgToPdf.portrait") },
              { value: "landscape", label: t("jpgToPdf.landscape") },
            ],
            options.orientation,
            (v) => {
              options.orientation = v;
              page.refreshSidebar();
            },
          ),
        ),
      );
    }
    body.push(
      field(
        t("jpgToPdf.margin"),
        segmented(
          [
            { value: "none", label: t("jpgToPdf.marginNone") },
            { value: "small", label: t("jpgToPdf.marginSmall") },
            { value: "large", label: t("jpgToPdf.marginLarge") },
          ],
          options.margin,
          (v) => {
            options.margin = v;
            page.refreshSidebar();
          },
        ),
      ),
    );
    if (page.ready.length > 1) {
      body.push(
        checkbox(t("jpgToPdf.merge"), mergeAll, (v) => {
          mergeAll = v;
          page.refreshSidebar();
        }),
      );
    }
    return { body, action: t("jpgToPdf.action"), blocker: entriesBlocker(page) };
  },

  run(page, jobId, progress) {
    return api.jpgToPdf(
      jobId,
      page.ready.map((e) => ({ path: e.file.path, rotation: e.rotation })),
      { ...options },
      mergeAll,
      t("jpgToPdf.outputName"),
      progress,
    );
  },

  resultTitle: () => t("jpgToPdf.done"),
};
