import { api, type SplitMode } from "../../api";
import { h } from "../../dom";
import { formatSize, t, tn } from "../../i18n";
import { icon } from "../../icons";
import { checkbox, choices, entriesBlocker, field, segmented, type Entry, type ToolConfig, type ToolPage } from "../tool";

type Mode = "ranges" | "every" | "extract";

const state = {
  mode: "ranges" as Mode,
  ranges: "",
  mergeRanges: false,
  every: 1,
  extractAll: true,
  selected: new Set<number>(),
  mergeSelected: true,
  entryId: "",
};

/** Couleurs des groupes de pages (un groupe = un fichier produit). */
// Chiffre blanc lisible sur chacune (≥ 5:1, audit Impeccable).
const GROUP_COLORS = ["#b91c1c", "#1d4ed8", "#047857", "#b45309", "#6d28d9", "#be185d", "#0e7490", "#3f6212"];

function currentEntry(page: ToolPage): Entry | undefined {
  const entry = page.ready[0];
  if (entry?.info && state.entryId !== entry.id) {
    // Nouveau document : valeurs par défaut adaptées.
    state.entryId = entry.id;
    state.selected.clear();
    const n = entry.info.pageCount;
    const half = Math.ceil(n / 2);
    state.ranges = n > 1 ? `1-${half}, ${half + 1}-${n}` : "1";
    state.every = Math.min(Math.max(1, state.every), Math.max(1, n));
  }
  return entry;
}

/** Même règles que le moteur Rust (hestia-core/src/ranges.rs). */
function parseRanges(input: string, count: number): { groups: number[][]; error: string | null } {
  const groups: number[][] = [];
  for (const raw of input.split(/[,;]/)) {
    const part = raw.trim();
    if (!part) continue;
    const m = /^(\d*)\s*(-?)\s*(\d*)$/.exec(part);
    if (!m) return { groups, error: t("split.errSyntax", { part }) };
    const [, a = "", dash = "", b = ""] = m;
    let start: number;
    let end: number;
    if (!dash) {
      if (!a) return { groups, error: t("split.errSyntax", { part }) };
      start = end = Number(a);
    } else {
      start = a ? Number(a) : 1;
      end = b ? Number(b) : count;
    }
    if (start < 1) return { groups, error: t("split.errSyntax", { part }) };
    if (end < start) return { groups, error: t("split.errOrder", { part }) };
    if (end > count) return { groups, error: t("split.errMax", { part, n: count }) };
    groups.push(Array.from({ length: end - start + 1 }, (_, i) => start - 1 + i));
  }
  if (groups.length === 0) return { groups, error: t("split.errEmpty") };
  return { groups, error: null };
}

function plan(count: number): { groups: number[][]; error: string | null } {
  switch (state.mode) {
    case "ranges": {
      const parsed = parseRanges(state.ranges, count);
      if (parsed.error) return parsed;
      return { groups: state.mergeRanges ? [parsed.groups.flat()] : parsed.groups, error: null };
    }
    case "every": {
      const size = Math.max(1, Math.floor(state.every) || 1);
      const groups: number[][] = [];
      for (let i = 0; i < count; i += size) groups.push(Array.from({ length: Math.min(size, count - i) }, (_, k) => i + k));
      return { groups, error: null };
    }
    case "extract": {
      if (state.extractAll) return { groups: Array.from({ length: count }, (_, i) => [i]), error: null };
      const selected = [...state.selected].filter((p) => p < count).sort((a, b) => a - b);
      if (selected.length === 0) return { groups: [], error: t("split.selectedCount", { n: 0 }) };
      return { groups: state.mergeSelected ? [selected] : selected.map((p) => [p]), error: null };
    }
  }
}

function splitMode(): SplitMode {
  switch (state.mode) {
    case "ranges":
      return { mode: "ranges", ranges: state.ranges, merge: state.mergeRanges };
    case "every":
      return { mode: "every", size: Math.max(1, Math.floor(state.every) || 1) };
    case "extract":
      return state.extractAll
        ? { mode: "allPages" }
        : { mode: "select", pages: [...state.selected].sort((a, b) => a - b).map((p) => p + 1), merge: state.mergeSelected };
  }
}

export const splitConfig: ToolConfig = {
  multiple: false,
  pickLabel: "drop.selectPdfOne",

  main(page) {
    const entry = page.entries[0];
    if (!entry) return h("div");
    const ready = currentEntry(page);
    const count = ready?.info?.pageCount ?? 0;
    const { groups } = ready ? plan(count) : { groups: [] };
    const groupOf = new Map<number, number>();
    groups.forEach((g, gi) => g.forEach((p) => groupOf.has(p) || groupOf.set(p, gi)));
    const selecting = state.mode === "extract" && !state.extractAll;

    const toolbar = h(
      "div",
      { class: "files-toolbar" },
      icon("paper", 18),
      h("strong", { class: "file-name", text: entry.file.name, style: { "max-width": "50%" } }),
      h("span", { text: ready?.info ? `${tn("common.page", "common.pages", count)} · ${formatSize(entry.file.size)}` : "" }),
      h("span", { class: "spacer" }),
      h("button", { class: "btn", on: { click: () => void page.pick() } }, icon("add", 18), t("drop.selectPdfOne")),
    );

    if (!ready) {
      // Fichier en cours de lecture, verrouillé ou illisible : carte simple.
      const status =
        entry.status === "locked"
          ? h("div", { class: "lock" }, icon("lock", 30), t("password.required"), h("button", { class: "btn", on: { click: () => void page.unlock(entry) } }, icon("unlock", 16), t("password.unlock")))
          : entry.status === "error"
            ? h("div", { class: "lock" }, icon("alert", 30))
            : h("div", { class: "spinner" });
      return h("div", null, toolbar, h("div", { class: "files-grid" }, h("div", { class: "file-card" }, h("div", { class: "thumb" }, status))));
    }

    const grid = h("div", { class: "pages-grid" });
    for (let i = 0; i < count; i++) {
      const gi = groupOf.get(i);
      const included = gi !== undefined;
      const tile = h(
        "button",
        {
          class: `page-tile${selecting ? " is-selectable" : ""}${selecting && state.selected.has(i) ? " is-selected" : ""}`,
          type: "button",
          style: !selecting && !included ? { opacity: "0.35" } : {},
          attrs: { "aria-pressed": selecting ? String(state.selected.has(i)) : null, tabindex: selecting ? 0 : -1 },
        },
        page.pageThumb(ready, i, 220),
        h("span", { text: String(i + 1) }),
        !selecting && included && groups.length > 1 && state.mode !== "extract"
          ? h("span", { class: "group-tag", text: String(gi + 1), style: { background: GROUP_COLORS[gi % GROUP_COLORS.length]! } })
          : null,
        selecting && state.selected.has(i) ? h("span", { class: "check" }, icon("check", 14)) : null,
      );
      if (selecting) {
        tile.addEventListener("click", () => {
          if (state.selected.has(i)) state.selected.delete(i);
          else state.selected.add(i);
          const on = state.selected.has(i);
          tile.classList.toggle("is-selected", on);
          tile.setAttribute("aria-pressed", String(on));
          tile.querySelector(".check")?.remove();
          if (on) tile.appendChild(h("span", { class: "check" }, icon("check", 14)));
          page.refreshAction();
          const counter = document.querySelector("[data-split-count]");
          if (counter) counter.textContent = t("split.selectedCount", { n: state.selected.size });
        });
      }
      grid.appendChild(tile);
    }
    return h("div", null, toolbar, grid);
  },

  sidebar(page) {
    const entry = currentEntry(page);
    const count = entry?.info?.pageCount ?? 0;
    const result = entry ? plan(count) : { groups: [], error: null };
    const body: HTMLElement[] = [
      segmented<Mode>(
        [
          { value: "ranges", label: t("split.modeRanges") },
          { value: "every", label: t("split.modeEvery") },
          { value: "extract", label: t("split.modeExtract") },
        ],
        state.mode,
        (v) => {
          state.mode = v;
          page.refreshFiles();
        },
        t("split.modeLabel"),
      ),
    ];

    if (state.mode === "ranges") {
      const help = h("div", {
        class: `field-help${result.error ? " error" : ""}`,
        text: result.error ?? t("split.rangesHelp"),
        attrs: { id: "split-ranges-help", "aria-live": "polite" },
      });
      const input = h("input", {
        class: `input${result.error ? " invalid" : ""}`,
        value: state.ranges,
        placeholder: t("split.rangesPlaceholder"),
        attrs: { spellcheck: "false", inputmode: "numeric", "aria-describedby": "split-ranges-help", "aria-invalid": String(result.error !== null) },
        on: {
          input: (e) => {
            state.ranges = (e.target as HTMLInputElement).value;
            const r = plan(count);
            input.classList.toggle("invalid", r.error !== null);
            input.setAttribute("aria-invalid", String(r.error !== null));
            help.classList.toggle("error", r.error !== null);
            help.textContent = r.error ?? t("split.rangesHelp");
            page.refreshMain();
            page.refreshAction();
          },
        },
      });
      body.push(
        field(t("split.rangesLabel"), input, help),
        checkbox(t("split.mergeRanges"), state.mergeRanges, (v) => {
          state.mergeRanges = v;
          page.refreshMain();
          page.refreshAction();
        }),
      );
    } else if (state.mode === "every") {
      const input = h("input", {
        class: "input",
        type: "number",
        min: "1",
        max: String(Math.max(1, count)),
        value: String(state.every),
        style: { width: "90px" },
        on: {
          input: (e) => {
            const v = Number((e.target as HTMLInputElement).value);
            if (Number.isFinite(v) && v >= 1) {
              state.every = Math.min(Math.floor(v), Math.max(1, count));
              page.refreshMain();
              page.refreshAction();
            }
          },
        },
      });
      body.push(
        field(
          t("split.everyLabel"),
          h("div", { style: { display: "flex", "align-items": "center", gap: "10px" } }, input, h("span", { text: t("split.everyUnit") })),
        ),
      );
    } else {
      body.push(
        choices<"all" | "select">(
          [
            { value: "all", title: t("split.extractAll"), desc: t("split.extractAllDesc") },
            { value: "select", title: t("split.extractSelect"), desc: t("split.extractSelectDesc") },
          ],
          state.extractAll ? "all" : "select",
          (v) => {
            state.extractAll = v === "all";
            page.refreshFiles();
          },
          t("split.modeExtract"),
        ),
      );
      if (!state.extractAll) {
        body.push(
          h(
            "div",
            { style: { display: "flex", "align-items": "center", gap: "8px" } },
            h("span", { class: "field-help", style: { flex: "1" }, text: t("split.selectedCount", { n: state.selected.size }), dataset: { splitCount: "" } }),
            h("button", {
              class: "btn",
              text: t("split.selectAll"),
              on: {
                click: () => {
                  for (let i = 0; i < count; i++) state.selected.add(i);
                  page.refreshFiles();
                },
              },
            }),
            h("button", {
              class: "btn",
              text: t("split.selectNone"),
              on: {
                click: () => {
                  state.selected.clear();
                  page.refreshFiles();
                },
              },
            }),
          ),
          checkbox(t("split.mergeSelected"), state.mergeSelected, (v) => {
            state.mergeSelected = v;
            page.refreshAction();
          }),
        );
      }
    }

    const n = result.groups.length;
    return {
      body,
      action: t("split.action"),
      blocker: entriesBlocker(page) ?? result.error,
      summary: n === 1 ? t("split.willCreateOne") : t("split.willCreate", { n }),
    };
  },

  run(page, jobId, progress) {
    const entry = page.ready[0];
    if (!entry) return Promise.reject({ code: "no_input", file: null, detail: "" });
    return api.splitPdf(jobId, page.pdfInput(entry), splitMode(), progress);
  },

  resultTitle: () => t("split.done"),
};
