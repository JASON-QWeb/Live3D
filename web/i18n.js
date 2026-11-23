// Console interface language. Chinese is the source text everywhere: in the
// page, in these scripts and in messages from the Vorela backend. EN maps
// each Chinese string or template to English; "{0}", "{1}" mark values.
// Anything missing from EN shows in Chinese.

const KEY = "vorela.ui-lang";
let stored = null;
try {
  stored = localStorage.getItem(KEY);
} catch {
  // Storage can be blocked; the console then stays in Chinese.
}
export const lang = stored === "en" ? "en" : "zh";

/** Saves the choice and reloads, so every label is drawn in the new language. */
export function setLang(next) {
  try {
    localStorage.setItem(KEY, next);
  } catch {
    return false;
  }
  location.reload();
  return true;
}

const dictionary = await fetch(new URL("./i18n-en.json", import.meta.url));
const EN = await dictionary.json();

function fill(template, args) {
  return template.replace(/\{(\d+)\}/g, (m, i) => (i < args.length ? String(args[i]) : m));
}

/** A string written in this console's scripts, in the interface language. */
export function t(zh, ...args) {
  return fill(lang === "en" ? (EN[zh] ?? zh) : zh, args);
}

// Templates with values, longest literal text first so specific ones win.
let patterns = null;
function compile() {
  const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  patterns = Object.keys(EN)
    .filter((k) => /\{\d+\}/.test(k) && /[一-鿿]/.test(k.replace(/\{\d+\}/g, "")))
    .map((k) => {
      const order = [];
      const source = k
        .split(/(\{\d+\})/)
        .map((part) => {
          const m = /^\{(\d+)\}$/.exec(part);
          if (!m) return escape(part);
          order.push(Number(m[1]));
          return "([\\s\\S]*?)";
        })
        .join("");
      return { re: new RegExp("^" + source + "$"), order, en: EN[k], weight: k.replace(/\{\d+\}/g, "").length };
    })
    .sort((a, b) => b.weight - a.weight);
}

/**
 * A message from the Vorela backend, in the interface language. Values
 * inside a template, such as another message or a platform name, are
 * translated too; viewer text that matches nothing stays as it is.
 */
export function tr(text, depth = 0) {
  if (lang !== "en" || !text || depth > 3) return text;
  if (EN[text] !== undefined) return EN[text];
  if (!/[一-鿿]/.test(text)) return text;
  if (!patterns) compile();
  for (const p of patterns) {
    const m = p.re.exec(text);
    if (!m) continue;
    const args = [];
    p.order.forEach((index, i) => (args[index] = tr(m[i + 1], depth + 1)));
    return fill(p.en, args);
  }
  return text;
}

const ATTRS = ["placeholder", "title", "aria-label", "alt"];

/** Translates the page's fixed text once, before the scripts draw anything. */
function translatePage() {
  document.documentElement.lang = "en";
  document.title = t(document.title);
  const meta = document.querySelector('meta[name="description"]');
  if (meta) meta.content = t(meta.content);
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const raw = node.nodeValue;
    const key = raw.replace(/\s+/g, " ").trim();
    if (!key || EN[key] === undefined) continue;
    const lead = raw.match(/^\s*/)[0] ? " " : "";
    const tail = raw.match(/\s*$/)[0] ? " " : "";
    node.nodeValue = lead + EN[key] + tail;
  }
  for (const element of document.body.querySelectorAll(ATTRS.map((a) => `[${a}]`).join(","))) {
    for (const a of ATTRS) {
      const v = element.getAttribute(a);
      if (v && EN[v] !== undefined) element.setAttribute(a, EN[v]);
    }
  }
}
if (lang === "en") translatePage();

/** Chinese keys with no English, for checking coverage. */
export function missing(keys) {
  return keys.filter((k) => EN[k] === undefined);
}
