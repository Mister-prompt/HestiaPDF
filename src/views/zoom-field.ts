// Pourcentage de zoom affiché entre les boutons − et +, modifiable au clavier :
// on clique, on tape « 150 », Entrée. Flèches haut / bas : ± 10 %.

import { h } from "../dom";
import { t } from "../i18n";

export interface ZoomField {
  el: HTMLElement;
  /** Affiche le zoom courant (1 = 100 %), sauf pendant la saisie. */
  set(zoom: number): void;
}

export function zoomField(min: number, max: number, onCommit: (zoom: number) => void): ZoomField {
  let current = 1;
  const input = h("input", {
    class: "zoom-input",
    type: "text",
    attrs: {
      inputmode: "numeric",
      autocomplete: "off",
      spellcheck: "false",
      maxlength: "5",
      "aria-label": t("viewer.zoomValue"),
    },
  });
  const el = h("label", { class: "zoom-field", title: t("viewer.zoomValue") }, input, h("span", { class: "zoom-unit", text: "%", attrs: { "aria-hidden": "true" } }));

  const clamp = (z: number) => Math.min(max, Math.max(min, z));
  const show = () => (input.value = String(Math.round(current * 100)));
  const commit = () => {
    const typed = Number(input.value.replace(",", ".").replace(/[^\d.]/g, ""));
    if (input.value.trim() !== "" && Number.isFinite(typed) && typed > 0) onCommit(clamp(typed / 100));
    show();
  };

  input.addEventListener("focus", () => input.select());
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
      input.select();
    } else if (e.key === "Escape") {
      e.preventDefault();
      show();
      input.blur();
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      const step = e.key === "ArrowUp" ? 1 : -1;
      onCommit(clamp((Math.round(current * 10) + step) / 10));
      show();
      input.select();
    }
  });
  input.addEventListener("change", commit);
  input.addEventListener("blur", show);

  return {
    el,
    set(zoom: number) {
      current = zoom;
      if (document.activeElement !== input) show();
    },
  };
}
