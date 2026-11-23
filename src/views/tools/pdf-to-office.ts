// PDF → Word, PowerPoint, Excel, avec le moteur intégré (sans Office).

import { api, type OfficeFormat, type PdfToOfficeOutcome } from "../../api";
import { t, type Key } from "../../i18n";
import { entriesBlocker, infoBox, type ToolConfig } from "../tool";

export function pdfToOfficeConfig(format: OfficeFormat): ToolConfig {
  return {
    multiple: true,
    pickLabel: "drop.selectPdf",

    sidebar(page) {
      return {
        body: [
          infoBox(t(`toOffice.keeps.${format}` as Key)),
          infoBox(t("toOffice.limits")),
          infoBox(t("toOffice.noOffice")),
        ],
        action: t(`toOffice.action.${format}` as Key),
        blocker: entriesBlocker(page),
      };
    },

    run(page, jobId, progress) {
      return api.pdfToOffice(
        jobId,
        page.ready.map((e) => page.pdfInput(e)),
        format,
        progress,
      );
    },

    resultTitle: () => t(`toOffice.done.${format}` as Key),

    resultDetails(outcome) {
      return (outcome as PdfToOfficeOutcome).noText ? infoBox(t("toOffice.noText"), "warning") : null;
    },
  };
}
