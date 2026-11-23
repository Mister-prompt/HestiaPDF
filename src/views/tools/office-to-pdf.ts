// Word, PowerPoint, Excel → PDF, avec la suite bureautique installée.

import { api, type FileKind, type OfficeInfo } from "../../api";
import { t, type Key } from "../../i18n";
import { entriesBlocker, infoBox, type ToolConfig } from "../tool";

type OfficeKind = Extract<FileKind, "word" | "powerpoint" | "excel">;

const APP: Record<OfficeKind, string> = {
  word: "Microsoft Word",
  powerpoint: "Microsoft PowerPoint",
  excel: "Microsoft Excel",
};

const PICK: Record<OfficeKind, Key> = {
  word: "drop.selectWord",
  powerpoint: "drop.selectPowerpoint",
  excel: "drop.selectExcel",
};

// Recherche faite une fois par session (Office installé ? imprimante ?).
let info: OfficeInfo | null = null;
let checking: Promise<void> | null = null;

export function officeToPdfConfig(kind: OfficeKind): ToolConfig {
  return {
    multiple: true,
    pickLabel: PICK[kind],

    sidebar(page) {
      if (!info && !checking) {
        checking = api
          .officeStatus()
          .then((found) => {
            info = found;
          })
          .catch(() => {
            info = { word: false, powerpoint: false, excel: false, libreOffice: false, offlinePrinter: null };
          })
          .finally(() => {
            if (!page.scope.isDisposed) page.refreshSidebar();
          });
      }
      const body: HTMLElement[] = [];
      let blocker = entriesBlocker(page);
      if (!info) {
        body.push(infoBox(t("office.checking")));
        blocker ??= t("office.checking");
      } else if (info[kind]) {
        body.push(infoBox(t("office.withMicrosoft", { app: APP[kind] })));
        // PowerPoint n'a pas besoin de l'imprimante pour exporter.
        if (info.offlinePrinter && kind !== "powerpoint") {
          body.push(infoBox(t("office.printerOffline", { printer: info.offlinePrinter }), "warning"));
        }
      } else if (info.libreOffice) {
        body.push(infoBox(t("office.withLibre", { app: APP[kind] })));
      } else {
        body.push(infoBox(t("office.missing"), "warning"));
        blocker ??= t("office.missingShort");
      }
      body.push(infoBox(t("office.hidden")), infoBox(t("office.local")));
      return { body, action: t("office.action"), blocker };
    },

    run(page, jobId, progress) {
      return api.officeToPdf(
        jobId,
        page.ready.map((e) => e.file.path),
        progress,
      );
    },

    resultTitle: () => t("office.done"),
  };
}
