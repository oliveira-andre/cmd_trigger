// Small DOM helpers: h() builds elements, toast() shows a note at the bottom.
import { icon, type IconName } from "./icons";
import markUrl from "./assets/mark.svg";

export function mark(): HTMLElement {
  return h("img", { class: "mark", src: markUrl, alt: "" });
}

type Child = Node | string | number | null | undefined | false;
type Attrs = Record<string, unknown>;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs?: Attrs | null, ...children: (Child | Child[])[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs ?? {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") el.className = String(value);
    else if (key.startsWith("on") && typeof value === "function") el.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    else if (key === "value" || key === "checked" || key === "disabled" || key === "selected") (el as unknown as Record<string, unknown>)[key] = value;
    else if (key === "dataset") Object.assign(el.dataset, value);
    else el.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(typeof child === "string" || typeof child === "number" ? String(child) : child);
  }
  return el;
}

export function button(label: string | null, opts: { icon?: IconName; kind?: "primary" | "secondary" | "danger" | "ghost"; title?: string; onClick?: (e: MouseEvent) => void; type?: string; small?: boolean } = {}) {
  const kind = opts.kind ?? "secondary";
  return h(
    "button",
    { class: `button button-${kind}${opts.small ? " button-small" : ""}${label ? "" : " button-icon"}`, type: opts.type ?? "button", title: opts.title, "aria-label": opts.title ?? label, onClick: opts.onClick },
    opts.icon ? icon(opts.icon, opts.small ? 14 : 16) : null,
    label ? h("span", null, label) : null,
  );
}

export function toast(message: string, kind: "info" | "error" | "success" = "info") {
  const host = document.getElementById("toasts")!;
  const note = h("div", { class: `toast toast-${kind}`, role: "status" }, icon(kind === "error" ? "alert" : kind === "success" ? "check" : "bulb", 16), h("span", null, message));
  host.append(note);
  setTimeout(() => note.classList.add("leaving"), 3200);
  setTimeout(() => note.remove(), 3600);
}

/// A dark dialog over the page. Resolves when closed.
export function dialog(title: string, body: HTMLElement, opts: { wide?: boolean } = {}) {
  const host = document.getElementById("overlays")!;
  const close = () => {
    backdrop.remove();
    document.removeEventListener("keydown", onKey);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  };
  const panel = h(
    "div",
    { class: `dialog${opts.wide ? " dialog-wide" : ""}`, role: "dialog", "aria-modal": "true", "aria-label": title },
    h("div", { class: "dialog-head" }, h("h2", null, title), button(null, { icon: "close", kind: "ghost", title: "Close", onClick: () => close() })),
    body,
  );
  const backdrop = h("div", { class: "backdrop", onMousedown: (e: MouseEvent) => e.target === backdrop && close() }, panel);
  host.append(backdrop);
  document.addEventListener("keydown", onKey);
  return { close, panel };
}

export function field(label: string, control: HTMLElement, hint?: Child | null): HTMLElement {
  return h("label", { class: "field" }, h("span", { class: "field-label" }, label), control, hint ? h("span", { class: "field-hint" }, hint) : null);
}

export function toggle(checked: boolean, label: string, onChange?: (on: boolean) => void): { root: HTMLElement; input: HTMLInputElement } {
  const input = h("input", { type: "checkbox", class: "toggle-input", checked, onChange: () => onChange?.(input.checked) });
  const root = h("label", { class: "toggle" }, input, h("span", { class: "toggle-track" }, h("span", { class: "toggle-thumb" })), h("span", { class: "toggle-label" }, label));
  return { root, input };
}

export function linkButton(label: string, onClick: () => void): HTMLElement {
  return h("button", { type: "button", class: "link", onClick }, label, icon("external", 12));
}
