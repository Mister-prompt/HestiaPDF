// Composants transverses : notifications, fenêtres, messages d'erreur.

import type { CommandError } from "./api";
import { h } from "./dom";
import { t, type Key } from "./i18n";
import { icon } from "./icons";

// --- Notifications ----------------------------------------------------------

let toastHost: HTMLElement | null = null;

export function toast(message: string, kind: "info" | "error" = "info", ms = 4200): void {
  if (!toastHost) {
    toastHost = h("div", { class: "toasts", attrs: { role: "status", "aria-live": "polite" } });
    document.body.appendChild(toastHost);
  }
  const el = h("div", { class: `toast ${kind}` }, icon(kind === "error" ? "alert" : "info", 18), message);
  toastHost.appendChild(el);
  window.setTimeout(() => el.remove(), ms);
}

// --- Erreurs ----------------------------------------------------------------

const KNOWN = new Set([
  "password_required",
  "wrong_password",
  "invalid_pdf",
  "unsupported_image",
  "invalid_range",
  "no_input",
  "cancelled",
  "would_overwrite_input",
  "engine_unavailable",
  "office_missing",
  "office_protected",
  "office_timeout",
  "office_failed",
  "forbidden",
  "io",
  "internal",
]);

/** Message lisible pour une erreur renvoyée par Rust. */
export function errorText(err: CommandError): string {
  const key = (KNOWN.has(err.code) ? `error.${err.code}` : "error.unknown") as Key;
  return err.file ? `${err.file} : ${t(key)}` : t(key);
}

// --- Fenêtres modales -------------------------------------------------------

interface ModalOptions {
  title: string;
  titleIcon?: Parameters<typeof icon>[0];
  body: (Node | string)[];
  actions: { label: string; primary?: boolean; value: string }[];
  /** Champ à focaliser à l'ouverture. */
  focus?: HTMLElement;
  /** Appelée avant de fermer avec une valeur ; renvoie false pour garder ouvert. */
  onSubmit?: (value: string) => boolean | Promise<boolean>;
}

/** Affiche une fenêtre ; renvoie la valeur du bouton choisi (null = fermée). */
export function modal(opts: ModalOptions): Promise<string | null> {
  return new Promise((resolve) => {
    const previous = document.activeElement as HTMLElement | null;
    const close = (value: string | null) => {
      document.removeEventListener("keydown", onKey, true);
      backdrop.remove();
      previous?.focus?.();
      resolve(value);
    };
    const choose = async (value: string) => {
      if (opts.onSubmit && value !== "cancel" && !(await opts.onSubmit(value))) return;
      close(value);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close(null);
      } else if (e.key === "Tab") {
        trapFocus(e, form);
      }
    };
    const primary = opts.actions.find((a) => a.primary);
    const form = h(
      "form",
      {
        class: "modal",
        attrs: { role: "dialog", "aria-modal": "true" },
        on: {
          submit: (e) => {
            e.preventDefault();
            if (primary) void choose(primary.value);
          },
        },
      },
      h("h3", null, opts.titleIcon ? icon(opts.titleIcon, 22) : null, opts.title),
      ...opts.body,
      h(
        "div",
        { class: "modal-actions" },
        opts.actions.map((a) =>
          h("button", {
            class: `btn ${a.primary ? "btn-primary" : ""}`,
            type: a.primary ? "submit" : "button",
            text: a.label,
            on: a.primary ? {} : { click: () => void choose(a.value) },
          }),
        ),
      ),
    );
    const backdrop = h("div", {
      class: "modal-backdrop",
      on: { mousedown: (e) => e.target === backdrop && close(null) },
    }, form);
    document.body.appendChild(backdrop);
    document.addEventListener("keydown", onKey, true);
    (opts.focus ?? form.querySelector<HTMLElement>("button"))?.focus();
  });
}

/** Garde le focus clavier à l'intérieur de `container`. */
export function trapFocus(e: KeyboardEvent, container: HTMLElement): void {
  const focusable = [
    ...container.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea, [tabindex]:not([tabindex='-1'])"),
  ].filter((el) => el.offsetParent !== null);
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (!first || !last) return;
  const active = document.activeElement;
  if (e.shiftKey && (active === first || !container.contains(active))) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && (active === last || !container.contains(active))) {
    e.preventDefault();
    first.focus();
  }
}

/**
 * Demande un mot de passe. `check` vérifie le mot de passe (côté Rust) et
 * renvoie true s'il est bon ; la fenêtre reste ouverte tant qu'il est faux.
 */
export async function askPassword(fileName: string, check: (pw: string) => Promise<boolean>): Promise<string | null> {
  const input = h("input", {
    class: "input",
    type: "password",
    attrs: { autocomplete: "off", spellcheck: "false", "aria-label": t("password.label") },
  });
  const help = h("div", { class: "field-help error", attrs: { "aria-live": "polite" } });
  let accepted: string | null = null;
  const result = await modal({
    title: t("password.title"),
    titleIcon: "lock",
    body: [
      h("p", { text: t("password.text", { name: fileName }) }),
      h("label", { class: "field" }, h("span", { class: "field-label", text: t("password.label") }), input, help),
    ],
    actions: [
      { label: t("common.cancel"), value: "cancel" },
      { label: t("password.unlock"), value: "ok", primary: true },
    ],
    focus: input,
    onSubmit: async () => {
      const pw = input.value;
      if (!pw) return false;
      if (await check(pw)) {
        accepted = pw;
        return true;
      }
      help.textContent = t("password.wrong");
      input.select();
      return false;
    },
  });
  return result === "ok" ? accepted : null;
}

/** Libellé de bouton avec icône. */
export function iconLabel(name: Parameters<typeof icon>[0], label: string, size = 18): (Node | string)[] {
  return [icon(name, size), label];
}
