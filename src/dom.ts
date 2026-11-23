// Petits utilitaires DOM. Règle de sécurité : on n'utilise JAMAIS innerHTML.
// Tout texte (noms de fichiers notamment) passe par des nœuds texte.

export type Child = Node | string | number | null | undefined | false;
type Listener = (ev: Event) => void;

export interface Props {
  class?: string;
  text?: string;
  title?: string;
  type?: string;
  disabled?: boolean;
  dataset?: Record<string, string>;
  attrs?: Record<string, string | number | boolean | null | undefined>;
  style?: Record<string, string>;
  on?: Partial<Record<keyof HTMLElementEventMap, Listener>>;
  [key: string]: unknown;
}

const SPECIAL = new Set(["class", "text", "dataset", "attrs", "style", "on"]);

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props?: Props | null,
  ...children: (Child | Child[])[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (props) {
    if (props.class) el.className = props.class;
    if (props.text !== undefined) el.textContent = props.text;
    if (props.dataset) Object.assign(el.dataset, props.dataset);
    if (props.style) for (const [k, v] of Object.entries(props.style)) el.style.setProperty(k, v);
    if (props.attrs) {
      for (const [k, v] of Object.entries(props.attrs)) {
        if (v === false || v === null || v === undefined) continue;
        el.setAttribute(k, v === true ? "" : String(v));
      }
    }
    if (props.on) {
      for (const [ev, fn] of Object.entries(props.on)) if (fn) el.addEventListener(ev, fn as Listener);
    }
    for (const [k, v] of Object.entries(props)) {
      if (SPECIAL.has(k) || v === undefined) continue;
      // Propriétés DOM simples (value, checked, disabled, type, title…).
      (el as unknown as Record<string, unknown>)[k] = v;
    }
  }
  append(el, children);
  return el;
}

export function append(parent: Node, children: (Child | Child[])[]): void {
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    parent.appendChild(typeof c === "string" || typeof c === "number" ? document.createTextNode(String(c)) : c);
  }
}

export function clear(el: Element): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

/** Texte enrichi limité à <em> et <strong> (traductions de l'appli uniquement). */
export function richText(source: string): DocumentFragment {
  const frag = document.createDocumentFragment();
  const re = /<(em|strong)>(.*?)<\/\1>/g;
  let last = 0;
  for (let m = re.exec(source); m; m = re.exec(source)) {
    if (m.index > last) frag.appendChild(document.createTextNode(source.slice(last, m.index)));
    const tag = m[1] === "em" ? "em" : "strong";
    frag.appendChild(h(tag, { text: m[2] ?? "" }));
    last = m.index + m[0].length;
  }
  if (last < source.length) frag.appendChild(document.createTextNode(source.slice(last)));
  return frag;
}

/** Regroupe des fonctions de nettoyage (écouteurs, URL d'images…) d'une vue. */
export class Scope {
  private cleanups: (() => void)[] = [];
  private disposed = false;

  add(fn: () => void): void {
    if (this.disposed) fn();
    else this.cleanups.push(fn);
  }

  /** URL d'objet libérée automatiquement à la fin de la vue. */
  objectUrl(blob: Blob): string {
    const url = URL.createObjectURL(blob);
    this.add(() => URL.revokeObjectURL(url));
    return url;
  }

  get isDisposed(): boolean {
    return this.disposed;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const fn of this.cleanups.splice(0).reverse()) {
      try {
        fn();
      } catch {
        /* nettoyage au mieux */
      }
    }
  }
}
