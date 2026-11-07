// The icon picker: every emoji, searchable by name, plus the plain symbol an entry shows when it
// has no emoji. An icon is only text, so the iPhone app and the dashboard draw what's picked here
// as it is. Names and groups come from emojibase in the window's language, loaded on first use.
import { h } from "../dom";
import { icon, type IconName } from "../icons";
import { t } from "../i18n";

const LANGUAGES = {
  en: () => Promise.all([import("emojibase-data/en/data.json"), import("emojibase-data/en/messages.json")]),
  es: () => Promise.all([import("emojibase-data/es/data.json"), import("emojibase-data/es/messages.json")]),
  pt: () => Promise.all([import("emojibase-data/pt/data.json"), import("emojibase-data/pt/messages.json")]),
  de: () => Promise.all([import("emojibase-data/de/data.json"), import("emojibase-data/de/messages.json")]),
  fr: () => Promise.all([import("emojibase-data/fr/data.json"), import("emojibase-data/fr/messages.json")]),
  ja: () => Promise.all([import("emojibase-data/ja/data.json"), import("emojibase-data/ja/messages.json")]),
  it: () => Promise.all([import("emojibase-data/it/data.json"), import("emojibase-data/it/messages.json")]),
};

// The examples' icons first, then what commands are usually about.
const SUGGESTED = (
  "🗣️ 🔒 🔇 🔊 🎚️ ⏯️ 🎵 🌙 ⚡ 😴 ⏻ 🚀 🔁 ⏰ 🧮 🌐 🔄 📦 📝 " +
  "📺 🎬 🎶 🎸 🎹 🎮 🕹️ 📻 📡 🔔 🎙️ 🎧 🎤 📽️ 📸 " +
  "💡 🔌 🖥️ 💻 🖨️ ⌨️ 🖱️ 📱 ☎️ 🔋 🔦 🏠 🚪 🪟 🛋️ 🛏️ 🚿 🧹 " +
  "☀️ 🌤️ ⛅ 🌧️ ⛈️ ❄️ 🌬️ 🌡️ 🔥 💧 🌊 🌿 🌱 ⭐ 🌈 " +
  "🚗 🚕 🚌 🏎️ 🚓 🚑 ✈️ 🛸 🚂 🚢 🏍️ 🚲 🛴 " +
  "🔧 🔩 ⚙️ 🛠️ 🔑 🔐 🔓 📂 📁 📋 📌 ✅ ❌ ⚠️ ▶️ ⏸️ ⏹️ ⏭️ 🗑️ " +
  "👤 👥 👍 👎 👋 🙌 👏 💪 ❤️ 🧠 👀 " +
  "🍎 🍕 🍔 ☕ 🍵 🥤 🌸 🌻 🌴 🐶 🐱 🐦 🐠 🐾 " +
  "🔴 🟠 🟡 🟢 🔵 🟣 ⚫ ⚪ 🔶 🔷 ❓ ❗ 💬 📊 📈 🗓️ ⌚"
).split(" ");

// Words people use here that the emoji names leave out. There's no terminal emoji, so the
// computers stand in for it.
const ALIASES: Record<string, string> = {
  "💻": "terminal console command script",
  "🖥️": "terminal console command script server screen display",
  "⌨️": "terminal console command",
  "📜": "script",
  "🗄️": "server database",
  "📺": "screen display",
  "🗣️": "say voice",
  "🎙️": "voice",
  "🔊": "audio",
  "🖼️": "image",
  "🔄": "restart reboot sync update",
  "🔁": "restart reboot",
  "📦": "update",
  "🔌": "power shutdown usb",
  "📴": "shutdown",
  "⏰": "wake schedule",
  "🗓️": "schedule",
  "▶️": "start",
  "🚀": "start deploy",
  "⏹️": "quit kill",
  "❌": "quit kill",
  "🛑": "quit kill",
  "⚡": "shortcut",
  "⚙️": "settings",
  "🌐": "browser",
  "🔔": "notification",
  "🔍": "find",
  "🐞": "debug",
  "🔑": "ssh",
  "🛡️": "firewall vpn",
  "💾": "backup storage",
  "📋": "paste",
  "⬇️": "download",
  "📥": "download",
  "⬆️": "upload",
  "📤": "upload",
};

// The Shut down examples use this one; it's a Unicode symbol, so emoji lists don't have it.
const POWER = { emoji: "⏻", label: "power", tags: ["shutdown", "off", "on", "standby"], group: 8 };

const FACES: Record<string, string> = {
  "smileys-emotion": "😀",
  "people-body": "👋",
  "animals-nature": "🐶",
  "food-drink": "🍎",
  "travel-places": "🚗",
  activities: "⚽",
  objects: "💡",
  symbols: "🔣",
  flags: "🏁",
};

// `place` is the position in SUGGESTED, or its length for the rest.
type Entry = { emoji: string; label: string; group?: number; find: string; place: number };
type Group = { name: string; face: string; entries: Entry[] };

/// Lowercase, without accents, so "icone" finds "ícone".
export function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

/// `find` is padded with spaces so a search can tell whole words from parts of words.
function entry(e: { emoji: string; label: string; tags?: string[]; group?: number }): Entry {
  return { emoji: e.emoji, label: e.label, group: e.group, find: ` ${fold([e.label, ...(e.tags ?? []), ALIASES[e.emoji] ?? ""].join(" "))} `, place: SUGGESTED.length };
}

/// Whether the system font has this emoji: a colour glyph ignores the fill colour, and the empty
/// box of a missing one takes it.
function drawn(emoji: string): boolean {
  const context = document.createElement("canvas").getContext("2d", { willReadFrequently: true })!;
  context.font = "24px sans-serif";
  context.textBaseline = "top";
  const paint = (fill: string) => {
    context.clearRect(0, 0, 32, 32);
    context.fillStyle = fill;
    context.fillText(emoji, 0, 0);
    return context.getImageData(0, 0, 32, 32).data;
  };
  const black = paint("#000");
  const white = paint("#fff");
  return black.some((v) => v !== 0) && black.every((v, i) => v === white[i]);
}

/// The first emoji release this computer can't draw, from one single-glyph sample per release.
/// When it draws none, it has no colour font to test against, so nothing is held back.
function firstMissingRelease(all: { emoji: string; version: number }[]): number {
  const samples = new Map<number, string>();
  for (const e of all) {
    const single = [...e.emoji.replace(/️/g, "")].length === 1;
    if (e.version >= 11 && single && !samples.has(e.version)) samples.set(e.version, e.emoji);
  }
  const releases = [...samples].sort((a, b) => a[0] - b[0]);
  const missing = releases.find(([, sample]) => !drawn(sample));
  return !missing || missing === releases[0] ? Infinity : missing[0];
}

async function load(language: string): Promise<Group[]> {
  const files = LANGUAGES[language as keyof typeof LANGUAGES] ?? LANGUAGES.en;
  const [{ default: data }, { default: messages }] = await files();
  const cutoff = firstMissingRelease(data);
  const shown = data.filter((e) => e.group !== undefined && e.group !== 2 && e.version < cutoff);
  const entries = [...shown, POWER].map(entry);
  const byEmoji = new Map(entries.map((e) => [e.emoji, e]));
  const suggested = SUGGESTED.flatMap((e) => byEmoji.get(e) ?? []);
  suggested.forEach((e, i) => (e.place = i));
  return [
    { name: t("Suggested"), face: "⭐", entries: suggested },
    ...messages.groups
      .filter((g) => FACES[g.key])
      .map((g) => ({ name: g.message, face: FACES[g.key], entries: entries.filter((e) => e.group === g.order) })),
  ];
}

// By language: switching it redraws the window without reloading this module.
const loaded = new Map<string, Promise<Group[]>>();

function groups(): Promise<Group[]> {
  const language = document.documentElement.lang;
  if (!loaded.has(language)) loaded.set(language, load(language));
  return loaded.get(language)!;
}

const bare = (emoji: string) => emoji.replace(/️/g, "");

/// Exact words first, then the starts of words, then anything else; suggested icons lead each.
function rank(e: Entry, word: string): number {
  const fit = e.find.includes(` ${word} `) ? 0 : e.find.includes(` ${word}`) ? 1 : 2;
  return fit * (SUGGESTED.length + 1) + e.place;
}

/// `fallback` is what the entry shows without an emoji, and it can be picked like the rest.
export function emojiPicker(initial: string, onPick: (emoji: string) => void, fallback: IconName = "terminal"): HTMLElement {
  let current = initial;
  let everything: Entry[] = [];
  let sections: HTMLElement[] = [];
  const fallbackFind = ` ${fold(`${fallback} ${t("No icon")}`)} `;

  const cell = (e: Entry) =>
    h("button", { type: "button", class: "emoji-cell", title: e.label, "aria-label": e.label, tabindex: -1, dataset: { emoji: e.emoji } }, e.emoji);
  const fallbackCell = () =>
    h("button", { type: "button", class: "emoji-cell", title: t("No icon"), "aria-label": t("No icon"), tabindex: -1, dataset: { emoji: "" } }, icon(fallback, 18));

  const preview = h("span", { class: "emoji-current" });
  const tabs = h("div", { class: "emoji-tabs" });
  const browse = h("div");
  const results = h("div", { class: "emoji-cells", hidden: true });

  const cells = () => [...(results.hidden ? browse : results).querySelectorAll<HTMLElement>(".emoji-cell")];

  const show = () => {
    preview.replaceChildren(current || icon(fallback, 22));
    for (const c of cells()) c.classList.toggle("active", bare(c.dataset.emoji ?? "") === bare(current));
  };

  const choose = (emoji: string) => {
    current = emoji;
    onPick(emoji);
    show();
  };

  const filter = () => {
    const words = fold(input.value).split(/\s+/).filter(Boolean);
    const searching = words.length > 0;
    browse.hidden = tabs.hidden = searching;
    results.hidden = !searching;
    if (searching) {
      const hits = everything
        .filter((e) => words.every((w) => e.find.includes(w)))
        .sort((a, b) => rank(a, words[0]) - rank(b, words[0]));
      const plain = words.every((w) => fallbackFind.includes(w)) ? [fallbackCell()] : [];
      const found = [...plain, ...hits.map(cell)];
      results.replaceChildren(...(found.length ? found : [h("p", { class: "muted small" }, t("Nothing matches “{q}”.", { q: input.value }))]));
    }
    const first = cells()[0];
    if (first) first.tabIndex = 0;
    show();
  };

  const input = h("input", {
    class: "input search-input",
    placeholder: t("Search icons"),
    "aria-label": t("Search icons"),
    autocomplete: "off",
    spellcheck: "false",
    onInput: filter,
    onKeydown: (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        // Enter would otherwise save the whole form.
        e.preventDefault();
        if (!results.hidden) cells()[0]?.click();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        cells()[0]?.focus();
      } else if (e.key === "Escape" && input.value) {
        // The first Escape clears the search; the next one closes the dialog as usual.
        e.stopPropagation();
        input.value = "";
        filter();
      }
    },
  });

  // Keep the tab of the group in view lit.
  const follow = () => {
    let at = 0;
    sections.forEach((section, i) => {
      if (section.offsetTop <= scroller.scrollTop + 1) at = i;
    });
    [...tabs.children].forEach((tab, i) => tab.classList.toggle("active", i === at));
  };

  const scroller = h(
    "div",
    {
      class: "emoji-scroll",
      onScroll: follow,
      onClick: (e: MouseEvent) => {
        const hit = (e.target as Element).closest<HTMLElement>(".emoji-cell");
        if (hit) choose(hit.dataset.emoji ?? "");
      },
      // The cells are one Tab stop, so Tab can still get past them to Save.
      onKeydown: (e: KeyboardEvent) => {
        const perRow = Math.max(1, Math.floor((scroller.clientWidth + 4) / 38));
        const step = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -perRow, ArrowDown: perRow } as Record<string, number>)[e.key];
        if (!step) return;
        e.preventDefault();
        const list = cells();
        const next = list[list.indexOf(document.activeElement as HTMLElement) + step];
        if (next) next.focus();
        else if (step < 0) input.focus();
      },
    },
    browse,
    results,
  );

  groups().then((list) => {
    everything = list.slice(1).flatMap((g) => g.entries);
    sections = list.map((g, i) =>
      h(
        "section",
        null,
        h("h4", { class: "emoji-group" }, g.name),
        h("div", { class: "emoji-cells" }, i === 0 ? fallbackCell() : null, ...g.entries.map(cell)),
      ),
    );
    browse.replaceChildren(...sections);
    tabs.replaceChildren(
      ...list.map((g, i) =>
        h("button", { type: "button", class: "emoji-tab", title: g.name, "aria-label": g.name, onClick: () => { scroller.scrollTop = sections[i].offsetTop; } }, g.face),
      ),
    );
    filter();
    follow();
  });

  show();
  return h(
    "div",
    { class: "emoji-picker" },
    h(
      "div",
      { class: "emoji-head" },
      preview,
      h("div", { class: "search" }, icon("search", 16), input),
      h("button", { type: "button", class: "link", onClick: () => choose("") }, t("No icon")),
    ),
    tabs,
    scroller,
  );
}
