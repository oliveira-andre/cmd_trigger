// The text editor for commands.json (the original's CodeMirror window), with JSON linting.
// Saving refuses text that isn't a command list, so a typo can't wipe the commands.
import { json, jsonParseLinter } from "@codemirror/lang-json";
import { linter, lintGutter } from "@codemirror/lint";
import { basicSetup, EditorView } from "codemirror";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { keymap } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { api, on, type AppInfo } from "../api";
import type { Cleanup } from "../main";
import { button, h, toast } from "../dom";
import { t } from "../i18n";

export async function mountText(root: HTMLElement, info: AppInfo): Promise<Cleanup> {
  let saved = await api.readCommandsText();
  const state = h("span", { class: "status-text" });
  const host = h("div", { class: "code-host" });

  const save = async () => {
    const text = view.state.doc.toString();
    try {
      const n = await api.writeCommandsText(text);
      saved = text;
      toast(t("Saved {n} commands.", { n }), "success");
      dirty();
    } catch (e) {
      toast(t("Not saved: {error}", { error: String(e) }), "error");
    }
    return true;
  };

  const dirty = () => {
    const changed = view.state.doc.toString() !== saved;
    state.textContent = changed ? t("Unsaved changes") : "";
  };

  const theme = EditorView.theme(
    {
      "&": { height: "100%", backgroundColor: "var(--canvas)", color: "var(--code-text)" },
      ".cm-content": { fontFamily: "var(--mono)", fontSize: "13px", caretColor: "var(--primary)" },
      ".cm-gutters": { backgroundColor: "var(--canvas)", color: "var(--muted)", border: "none", borderRight: "1px solid var(--hairline)" },
      ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "rgba(0, 217, 146, 0.05)" },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": { backgroundColor: "rgba(0, 217, 146, 0.2) !important" },
      ".cm-cursor": { borderLeftColor: "var(--primary)" },
      ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy #f6465d" },
    },
    { dark: true },
  );
  // DESIGN.md code colours: near-white keys, the green for strings, soft green for literals,
  // muted punctuation.
  const highlight = HighlightStyle.define([
    { tag: tags.propertyName, color: "#f5f6f7", fontWeight: "550" },
    { tag: tags.string, color: "#00d992" },
    { tag: [tags.number, tags.bool, tags.null], color: "#2fd6a1" },
    { tag: [tags.brace, tags.squareBracket, tags.punctuation, tags.separator], color: "#8b949e" },
  ]);

  const view = new EditorView({
    doc: saved,
    parent: host,
    extensions: [
      basicSetup,
      json(),
      linter(jsonParseLinter()),
      lintGutter(),
      theme,
      syntaxHighlighting(highlight),
      keymap.of([{ key: "Mod-s", preventDefault: true, run: () => (save(), true) }]),
      EditorView.updateListener.of((u) => u.docChanged && dirty()),
    ],
  });

  root.append(
    h(
      "div",
      { class: "page" },
      h(
        "header",
        { class: "page-head" },
        button(null, { icon: "chevron", kind: "ghost", title: t("Back"), onClick: () => (location.hash = "commands") }),
        h("div", { class: "page-title" }, h("h1", null, "commands.json"), h("span", { class: "count mono" }, `${info.dataDir}`)),
        h("div", { class: "head-fill" }),
        state,
        button(t("Save"), { icon: "save", kind: "primary", onClick: () => save() }),
      ),
      h("div", { class: "code-frame" }, host),
    ),
  );
  view.focus();

  // Reload when something else changed the file and there are no local edits to lose.
  const unlisten = await on("catalog-changed", async () => {
    if (view.state.doc.toString() !== saved) return;
    const text = await api.readCommandsText();
    if (text === saved) return;
    saved = text;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    dirty();
  });
  return () => unlisten();
}
