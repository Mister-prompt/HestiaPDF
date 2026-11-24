// Outil « Modifier PDF » : éditeur visuel + panneau de propriétés.

import { api, type EditOutcome, type JobOutcome } from "../../api";
import { h } from "../../dom";
import { t, type Key } from "../../i18n";
import { icon, type IconName } from "../../icons";
import { modal } from "../../ui";
import { defaults, Editor, lengthText } from "../editor/editor";
import { FONT_GROUPS, FONTS, fontDef, preloadFonts } from "../editor/fonts";
import type { EditorObject, MeasureUnit } from "../editor/model";
import { entriesBlocker, field, infoBox, segmented, type Entry, type ToolConfig, type ToolPage } from "../tool";

const editors = new WeakMap<ToolPage, { entryId: string; editor: Editor }>();

function editorFor(page: ToolPage, entry: Entry): Editor | null {
  if (!entry.info) return null;
  const known = editors.get(page);
  if (known?.entryId === entry.id) return known.editor;
  const editor = new Editor({
    input: () => page.pdfInput(entry),
    pages: entry.info.pages,
    scope: page.scope,
    pageThumb: (index, width) => page.pageThumb(entry, index, width),
    onSelectionChange: () => page.refreshSidebar(),
    onEdit: () => page.refreshAction(),
  });
  editors.set(page, { entryId: entry.id, editor });
  return editor;
}

function currentEditor(page: ToolPage): Editor | null {
  const entry = page.ready[0];
  return entry ? editorFor(page, entry) : null;
}

const SWATCHES: [string, Key][] = [
  ["#111111", "color.black"],
  ["#ffffff", "color.white"],
  ["#6b7280", "color.gray"],
  ["#b91c1c", "color.carmine"],
  ["#ea580c", "color.orange"],
  ["#facc15", "color.yellow"],
  ["#15803d", "color.green"],
  ["#1d4ed8", "color.blue"],
  ["#7c3aed", "color.violet"],
  ["#db2777", "color.pink"],
];

/**
 * Choix de couleur : pastilles + sélecteur libre (+ « aucune » si permis).
 * Le sélecteur libre applique la couleur en direct (`live` = geste en cours).
 */
function colorPicker(value: string | null, allowNone: boolean, onPick: (c: string | null, live: boolean) => void): HTMLElement {
  const wrap = h("div", { class: "swatches", attrs: { role: "group" } });
  if (allowNone) {
    wrap.appendChild(
      h("button", {
        class: "swatch none",
        type: "button",
        title: t("edit.none"),
        attrs: { "aria-label": t("edit.none"), "aria-pressed": String(value === null) },
        on: { click: () => onPick(null, false) },
      }),
    );
  }
  for (const [c, name] of SWATCHES) {
    wrap.appendChild(
      h("button", {
        class: "swatch",
        type: "button",
        title: t(name),
        style: { background: c },
        attrs: { "aria-label": t(name), "aria-pressed": String(value?.toLowerCase() === c) },
        on: { click: () => onPick(c, false) },
      }),
    );
  }
  const custom = h("input", {
    class: "swatch custom",
    type: "color",
    value: value ?? "#000000",
    title: t("edit.customColor"),
    attrs: { "aria-label": t("edit.customColor") },
    on: {
      // Pendant que l'on choisit : aperçu en direct, sans redessiner le panneau.
      input: (e) => onPick((e.target as HTMLInputElement).value, true),
      change: (e) => onPick((e.target as HTMLInputElement).value, false),
    },
  });
  wrap.appendChild(custom);
  return wrap;
}

let sliderSeq = 0;

/**
 * Curseur + valeur saisissable au clavier (même réglage, deux façons).
 * Une seule étape d'annulation par geste.
 */
function slider(
  label: string,
  value: number,
  min: number,
  max: number,
  step: number,
  unit: string,
  onInput: (v: number) => void,
  onEnd: () => void,
): HTMLElement {
  const labelId = `slider-${++sliderSeq}`;
  const decimals = step < 1 ? 1 : 0;
  const clamp = (v: number) => Math.min(max, Math.max(min, Math.round(v / step) * step));
  const range = h("input", {
    class: "slider",
    type: "range",
    attrs: { "aria-labelledby": labelId },
    min: String(min),
    max: String(max),
    step: String(step),
    value: String(value),
  });
  const num = h("input", {
    class: "num-input",
    type: "number",
    min: String(min),
    max: String(max),
    step: String(step),
    value: value.toFixed(decimals),
    attrs: { "aria-labelledby": labelId, inputmode: decimals ? "decimal" : "numeric" },
  });
  range.addEventListener("input", () => {
    const v = Number(range.value);
    num.value = v.toFixed(decimals);
    onInput(v);
  });
  range.addEventListener("change", () => onEnd());
  num.addEventListener("input", () => {
    const v = Number(num.value);
    if (num.value === "" || !Number.isFinite(v) || v < min || v > max) return;
    range.value = String(v);
    onInput(v);
  });
  num.addEventListener("change", () => {
    const v = Number(num.value);
    const fixed = clamp(Number.isFinite(v) ? v : value);
    num.value = fixed.toFixed(decimals);
    range.value = String(fixed);
    onInput(fixed);
    onEnd();
  });
  num.addEventListener("focus", () => num.select());
  return h(
    "div",
    { class: "field" },
    h(
      "div",
      { class: "slider-head" },
      h("span", { class: "field-label", id: labelId, text: label }),
      h("label", { class: "num-field" }, num, h("span", { class: "num-unit", text: unit, attrs: { "aria-hidden": "true" } })),
    ),
    range,
  );
}

type Kind = "text" | "shape" | "line" | "ink" | "highlight" | "image" | "measure" | null;

function kindOf(editor: Editor): Kind {
  const sel = editor.selected;
  if (sel) {
    if (sel.type === "measure") return "measure";
    if (sel.type === "rect" || sel.type === "ellipse") return "shape";
    if (sel.type === "ink") return sel.width >= 10 && sel.opacity < 0.7 ? "highlight" : "ink";
    return sel.type === "line" ? "line" : sel.type;
  }
  switch (editor.tool) {
    case "text":
      return "text";
    case "rect":
    case "ellipse":
      return "shape";
    case "line":
    case "arrow":
      return "line";
    case "ink":
      return "ink";
    case "highlight":
      return "highlight";
    case "measure":
      return "measure";
    default:
      return null;
  }
}

/** Réglages de l'outil Mesure : longueur, unité, ajout au PDF. */
function measurePanel(editor: Editor, apply: (setDefault: () => void, mutate: (o: EditorObject) => void, endOfGesture?: boolean, refresh?: boolean) => void, endGesture: () => void): HTMLElement[] {
  const d = defaults.measure;
  const sel = editor.selected?.type === "measure" ? editor.selected : null;
  const body: HTMLElement[] = [];
  if (sel) {
    const other = d.unit === "cm" ? "in" : "cm";
    body.push(
      h(
        "div",
        { class: "measure-readout", attrs: { "aria-live": "polite" } },
        h("span", { class: "measure-value selectable", text: lengthText(sel) }),
        h("span", { class: "measure-other selectable", text: lengthText(sel, other) }),
        h("span", { class: "field-help", text: t("edit.measure.paper") }),
      ),
    );
  } else {
    body.push(infoBox(t("edit.measure.help")));
  }
  body.push(
    field(
      t("edit.measure.unit"),
      segmented<MeasureUnit>(
        [
          { value: "cm", label: t("edit.measure.cm") },
          { value: "in", label: t("edit.measure.in") },
        ],
        d.unit,
        (u) => editor.setMeasureUnit(u),
        t("edit.measure.unit"),
      ),
    ),
  );
  const keep = sel ? sel.keep : d.keep;
  body.push(
    h(
      "label",
      { class: "check-row" },
      h("input", {
        type: "checkbox",
        checked: keep,
        on: {
          change: (e) => {
            const v = (e.target as HTMLInputElement).checked;
            apply(() => (d.keep = v), (o) => o.type === "measure" && (o.keep = v));
          },
        },
      }),
      h("span", { text: t("edit.measure.keep") }),
    ),
    h("div", { class: "field-help", text: t(keep ? "edit.measure.keptHelp" : "edit.measure.shownHelp") }),
  );
  const cur = sel ?? d;
  body.push(
    field(t("edit.color"), colorPicker(cur.color, false, (c, live) => c && apply(() => (d.color = c), (o) => o.type === "measure" && (o.color = c), !live, !live))),
    slider(t("edit.thickness"), cur.width, 0.5, 6, 0.5, "pt", (v) => apply(() => (d.width = v), (o) => o.type === "measure" && (o.width = v), false, false), endGesture),
  );
  return body;
}

function propertiesPanel(page: ToolPage, editor: Editor): HTMLElement[] {
  const sel = editor.selected;
  const kind = kindOf(editor);
  const body: HTMLElement[] = [];

  /** Applique un réglage à l'objet sélectionné et aux valeurs par défaut. */
  const apply = (setDefault: () => void, mutate: (o: EditorObject) => void, endOfGesture = true, refresh = true) => {
    setDefault();
    if (sel) editor.updateSelected(mutate, endOfGesture);
    if (refresh) page.refreshSidebar();
  };
  const endGesture = () => editor.endGesture();

  if (!kind) {
    body.push(
      infoBox(t("edit.hint")),
      h(
        "ul",
        { class: "tips" },
        h("li", { text: t("edit.tipText") }),
        h("li", { text: t("edit.tipMove") }),
        h("li", { text: t("edit.tipKeys") }),
      ),
    );
  }

  if (kind === "text") {
    const d = defaults.text;
    const cur = sel?.type === "text" ? sel : d;
    const def = fontDef(cur.font);
    // Chaque police s'affiche dans la liste avec son propre dessin.
    preloadFonts();
    const fontSelect = h(
      "select",
      {
        class: "select font-select",
        style: { "font-family": def.css },
        on: {
          change: (e) => {
            const v = (e.target as HTMLSelectElement).value;
            apply(() => (d.font = v), (o) => o.type === "text" && (o.font = v));
          },
        },
      },
      FONT_GROUPS.map((group) =>
        h(
          "optgroup",
          { attrs: { label: t(`edit.fontGroup.${group}` as Key) } },
          FONTS.filter((f) => f.group === group).map((f) =>
            h("option", { value: f.id, text: f.label, selected: cur.font === f.id, style: { "font-family": f.css } }),
          ),
        ),
      ),
    );
    const sizeInput = h("input", {
      class: "input",
      type: "number",
      min: "4",
      max: "300",
      step: "1",
      value: String(cur.size),
      style: { width: "84px" },
      attrs: { "aria-label": t("edit.size") },
      on: {
        input: (e) => {
          const v = Number((e.target as HTMLInputElement).value);
          if (!Number.isFinite(v) || v < 4 || v > 300) return;
          apply(() => (d.size = v), (o) => o.type === "text" && (o.size = v), false, false);
        },
        change: (e) => {
          // Valeur hors limites : ramenée dans [4, 300].
          const input = e.target as HTMLInputElement;
          const v = Math.min(300, Math.max(4, Math.round((Number(input.value) || cur.size) * 2) / 2));
          input.value = String(v);
          apply(() => (d.size = v), (o) => o.type === "text" && (o.size = v), true, false);
        },
        focus: (e) => (e.target as HTMLInputElement).select(),
      },
    });
    /** Gras / italique : grisés si la police ne les propose pas (jamais simulés). */
    const toggle = (label: Key, glyph: string, on: boolean, available: boolean, set: (v: boolean) => void) =>
      h("button", {
        class: "style-toggle",
        type: "button",
        text: glyph,
        title: available ? t(label) : t("edit.styleUnavailable", { font: def.label }),
        disabled: !available,
        attrs: { "aria-pressed": String(on && available), "aria-label": t(label) },
        on: { click: () => set(!on) },
      });
    body.push(
      field(t("edit.font"), fontSelect),
      field(
        t("edit.size"),
        h(
          "div",
          { class: "inline-row" },
          sizeInput,
          h("span", { class: "field-help", text: "pt" }),
          h("span", { class: "header-spacer" }),
          toggle("edit.bold", "B", cur.bold, def.bold, (v) => apply(() => (d.bold = v), (o) => o.type === "text" && (o.bold = v))),
          toggle("edit.italic", "I", cur.italic, def.italic, (v) => apply(() => (d.italic = v), (o) => o.type === "text" && (o.italic = v))),
        ),
      ),
      field(t("edit.color"), colorPicker(cur.color, false, (c, live) => c && apply(() => (d.color = c), (o) => o.type === "text" && (o.color = c), !live, !live))),
    );
    if (sel?.type === "text") {
      body.push(
        h(
          "button",
          { class: "btn", type: "button", on: { click: () => editor.editText(sel, false) } },
          icon("pen", 18),
          t("edit.editText"),
        ),
      );
    }
  }

  if (kind === "image" && sel?.type === "image") {
    const tool = (name: IconName, label: Key, fn: () => void) =>
      h("button", { class: "btn image-tool", type: "button", title: t(label), attrs: { "aria-label": t(label) }, on: { click: fn } }, icon(name, 20));
    body.push(
      field(
        t("edit.imageTransform"),
        h(
          "div",
          { class: "image-tools", attrs: { role: "group", "aria-label": t("edit.imageTransform") } },
          tool("rotateLeft", "edit.rotateLeft", () => editor.transformImage("left")),
          tool("rotateRight", "edit.rotateRight", () => editor.transformImage("right")),
          tool("flipH", "edit.flipH", () => editor.transformImage("flipH")),
          tool("flipV", "edit.flipV", () => editor.transformImage("flipV")),
        ),
      ),
      h(
        "label",
        { class: "check-row" },
        h("input", {
          type: "checkbox",
          checked: sel.grayscale,
          on: { change: (e) => apply(() => undefined, (o) => o.type === "image" && (o.grayscale = (e.target as HTMLInputElement).checked)) },
        }),
        icon("grayscale", 18),
        h("span", { text: t("edit.grayscale") }),
      ),
    );
  }

  if (kind === "shape") {
    const d = defaults.shape;
    const cur = sel && (sel.type === "rect" || sel.type === "ellipse") ? sel : d;
    body.push(
      field(t("edit.stroke"), colorPicker(cur.stroke, true, (c, live) => apply(() => (d.stroke = c), (o) => (o.type === "rect" || o.type === "ellipse") && (o.stroke = c), !live, !live))),
      field(t("edit.fill"), colorPicker(cur.fill, true, (c, live) => apply(() => (d.fill = c), (o) => (o.type === "rect" || o.type === "ellipse") && (o.fill = c), !live, !live))),
      slider(t("edit.thickness"), cur.strokeWidth, 0.5, 20, 0.5, "pt", (v) => apply(() => (d.strokeWidth = v), (o) => (o.type === "rect" || o.type === "ellipse") && (o.strokeWidth = v), false, false), endGesture),
    );
  }

  if (kind === "line") {
    const d = defaults.line;
    const cur = sel?.type === "line" ? sel : d;
    body.push(
      field(t("edit.color"), colorPicker(cur.color, false, (c, live) => c && apply(() => (d.color = c), (o) => o.type === "line" && (o.color = c), !live, !live))),
      slider(t("edit.thickness"), cur.width, 0.5, 20, 0.5, "pt", (v) => apply(() => (d.width = v), (o) => o.type === "line" && (o.width = v), false, false), endGesture),
    );
    if (sel?.type === "line") {
      const box = h("input", {
        type: "checkbox",
        checked: sel.arrow,
        on: { change: (e) => apply(() => undefined, (o) => o.type === "line" && (o.arrow = (e.target as HTMLInputElement).checked)) },
      });
      body.push(h("label", { class: "check-row" }, box, h("span", { text: t("edit.arrow") })));
    }
  }

  if (kind === "measure") body.push(...measurePanel(editor, apply, endGesture));

  if (kind === "ink" || kind === "highlight") {
    const d = kind === "ink" ? defaults.ink : defaults.highlight;
    const cur = sel?.type === "ink" ? sel : d;
    body.push(
      field(t("edit.color"), colorPicker(cur.color, false, (c, live) => c && apply(() => (d.color = c), (o) => o.type === "ink" && (o.color = c), !live, !live))),
      slider(t("edit.thickness"), cur.width, 1, 40, 1, "pt", (v) => apply(() => (d.width = v), (o) => o.type === "ink" && (o.width = v), false, false), endGesture),
    );
  }

  if (kind && kind !== "measure") {
    const target = sel ?? (kind === "text" ? defaults.text : kind === "shape" ? defaults.shape : kind === "line" ? defaults.line : kind === "ink" ? defaults.ink : kind === "highlight" ? defaults.highlight : null);
    if (target) {
      body.push(
        slider(
          t("edit.opacity"),
          Math.round(target.opacity * 100),
          1,
          100,
          1,
          "%",
          (v) => {
            const value = v / 100;
            if (!sel) target.opacity = value;
            apply(() => undefined, (o) => (o.opacity = value), false, false);
          },
          endGesture,
        ),
      );
    }
  }

  if (sel) {
    const action = (name: IconName, label: Key, fn: () => void, danger = false) =>
      h("button", { class: `btn${danger ? " btn-danger" : ""}`, type: "button", title: t(label), attrs: { "aria-label": t(label) }, on: { click: fn } }, icon(name, 18));
    body.push(
      h(
        "div",
        { class: "object-actions" },
        action("duplicate", "edit.duplicate", () => editor.duplicateSelected()),
        action("layerUp", "edit.front", () => editor.reorderSelected(true)),
        action("layerDown", "edit.back", () => editor.reorderSelected(false)),
        h("span", { class: "header-spacer" }),
        action("trash", "edit.delete", () => editor.deleteSelected(), true),
      ),
    );
  }

  const bad = editor.unsupportedChars();
  if (bad.length) body.push(infoBox(t("edit.unsupported", { chars: bad.slice(0, 8).join(" ") }), "warning"));
  body.push(h("div", { class: "field-help", text: t("edit.flattenInfo") }));
  return body;
}

export const editConfig: ToolConfig = {
  multiple: false,
  fullBleed: true,
  pickLabel: "drop.selectPdfOne",

  main(page) {
    const entry = page.entries[0];
    if (!entry) return h("div");
    const editor = entry.status === "ready" ? editorFor(page, entry) : null;
    if (editor) return editor.root;
    const status =
      entry.status === "locked"
        ? h("div", { class: "lock" }, icon("lock", 30), t("password.required"), h("button", { class: "btn", on: { click: () => void page.unlock(entry) } }, icon("unlock", 16), t("password.unlock")))
        : entry.status === "error"
          ? h("div", { class: "lock" }, icon("alert", 30))
          : h("div", { class: "spinner" });
    return h("div", { class: "center-stage" }, h("div", { class: "file-card" }, h("div", { class: "thumb" }, status)));
  },

  sidebar(page) {
    const editor = currentEditor(page);
    if (!editor) return { body: [], action: t("edit.action"), blocker: entriesBlocker(page) ?? t("common.loading") };
    const { objects, pages } = editor.count();
    return {
      body: propertiesPanel(page, editor),
      action: t("edit.action"),
      blocker: entriesBlocker(page) ?? (objects === 0 ? t("edit.empty") : null),
      summary: t("edit.summary", { n: objects, p: pages }),
    };
  },

  hasUnsavedWork(page) {
    const editor = currentEditor(page);
    return !!editor && editor.dirty && editor.count().objects > 0;
  },

  async run(page, jobId, progress): Promise<JobOutcome> {
    const entry = page.ready[0];
    const editor = currentEditor(page);
    if (!entry || !editor) throw { code: "no_input", file: null, detail: "" };
    editor.commitText();
    const outcome = await api.editPdf(jobId, page.pdfInput(entry), editor.toEditDocument(), t("edit.suffix"), progress);
    if (!outcome.cancelled) editor.dirty = false;
    return outcome;
  },

  resultTitle: () => t("edit.done"),

  resultDetails(outcome) {
    const o = outcome as EditOutcome;
    const notes: HTMLElement[] = [];
    if (o.replacedChars > 0) notes.push(infoBox(t("edit.replaced", { n: o.replacedChars }), "warning"));
    if (o.rebuilt) notes.push(infoBox(t("edit.rebuilt"), "warning"));
    return notes.length ? h("div", { style: { display: "flex", "flex-direction": "column", gap: "8px", width: "100%" } }, ...notes) : null;
  },
};

/** Fenêtre de confirmation avant de quitter l'éditeur avec des modifications. */
export async function confirmLeave(): Promise<boolean> {
  const choice = await modal({
    title: t("edit.leaveTitle"),
    titleIcon: "alert",
    body: [h("p", { text: t("edit.leaveText") })],
    actions: [
      { label: t("edit.leaveQuit"), value: "quit" },
      { label: t("edit.leaveStay"), value: "stay", primary: true },
    ],
  });
  return choice === "quit";
}
