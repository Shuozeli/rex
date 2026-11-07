// The popup's two views of a record, and the JSON panel under them.
//
// Reader: the record as a card: a title, the type and site, the key facts in two columns with readable dates
// and numbers, long text folded, lists as one line per item. Where values came from is left to the table and
// the trace.
// Table: one row per field with its schema name, the exact value and a dot for its origin.
//
// Both mark each top-level field with `data-field` (tests count them), and neither changes the JSON a user
// copies, which the panel at the bottom shows as it is.

import {
  asNumber,
  enumLabel,
  fieldLabel,
  formatScalar,
  isTimestamp,
  kindOf,
  type Lang,
  relativeTime,
  shortUrl,
  snake,
  typeLabel,
} from "./format";
import { t } from "./i18n";
import type { Extraction, FieldSource } from "./types";

export type View = "reader" | "table";

const PREFIXES = __REX_ENUM_PREFIXES__;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function link(href: string, text: string, cls = ""): HTMLAnchorElement {
  const a = el("a", cls, text);
  a.href = href;
  a.target = "_blank";
  a.rel = "noreferrer";
  return a;
}

/** Marks an element as showing the top-level field `key`. */
function mark<T extends HTMLElement>(e: T, key: string): T {
  e.classList.add("field");
  e.dataset.field = key;
  return e;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const LONG = 280;
const TITLE_KEYS = ["title", "name", "headline", "query"];
const LEDE_KEYS = ["summary", "description", "subtitle"];

/** The text that names an object or an item. */
function nameOf(v: unknown): string {
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (isObj(v)) {
    for (const k of ["title", "name", "text", "headline", "url", "id"]) if (typeof v[k] === "string") return v[k] as string;
  }
  return JSON.stringify(v);
}

/** Long text, folded to a few lines with a Show more / Show less toggle. */
function folded(text: string, cls: string): HTMLElement {
  const box = el("div", `fold ${cls}`);
  const body = el("div", "fold-body", text);
  box.appendChild(body);
  if (text.length > 220) {
    box.classList.add("closed");
    const b = el("button", "more", t("showMore"));
    b.addEventListener("click", () => {
      const closed = box.classList.toggle("closed");
      b.textContent = closed ? t("showMore") : t("showLess");
    });
    box.appendChild(b);
  }
  return box;
}

function scalarNode(key: string, value: unknown, lang: Lang, style: "read" | "exact"): HTMLElement {
  if (typeof value === "string" && /^https?:\/\//.test(value)) return link(value, style === "read" ? shortUrl(value) : value);
  const f = formatScalar(key, value, PREFIXES, lang, style);
  const span = el("span", f.tone ? `tone-${f.tone}` : "", f.text);
  if (style === "read" && isTimestamp(value)) span.title = value;
  return span;
}

/** A list item's second line: its other fields, readable ("Rasbora · Score 653 · Comments 211 · 8 hours ago"). */
function itemMeta(item: Record<string, unknown>, lang: Lang, now: number, style: "read" | "exact"): string {
  // Who first, then the numbers, then when: "Rasbora · Score 710 · Comments 224 · 6 hours ago".
  const names: string[] = [];
  const numbers: string[] = [];
  const times: string[] = [];
  for (const [k, v] of Object.entries(item)) {
    if (["title", "name", "text", "url", "id", "@type", "rank", "position"].includes(k) || v === null || v === "") continue;
    if (isObj(v)) names.push(nameOf(v));
    else if (Array.isArray(v)) names.push(v.map(nameOf).join(", "));
    else if (isTimestamp(v)) times.push(style === "read" ? relativeTime(v, now, lang) : formatScalar(k, v, PREFIXES, lang, "exact").text);
    else if (typeof v === "string" && /^https?:\/\//.test(v)) continue;
    else if (asNumber(v) !== null && isQuantity(k)) numbers.push(`${fieldLabel(k, lang)} ${formatScalar(k, v, PREFIXES, lang, style).text}`);
    else names.push(enumLabel(String(v), PREFIXES, lang) ?? String(v));
  }
  return [...names, ...numbers, ...times].join(" · ");
}

/** A numeric field that is a quantity, not an identifier such as a symbol or an id. */
const isQuantity = (k: string): boolean => kindOf(k, "1", PREFIXES) !== "text";

/** People and similar lists: objects with only a name (and maybe a url), shown inline as names. */
const isNameList = (v: unknown[]): boolean =>
  v.length > 0 && v.length <= 8 && v.every((x) => isObj(x) && Object.keys(x).every((k) => ["name", "url", "@type"].includes(k)) && typeof x["name"] === "string");

function nameList(v: unknown[]): HTMLElement {
  const span = el("span");
  v.forEach((x, i) => {
    const o = x as Record<string, unknown>;
    if (i > 0) span.appendChild(document.createTextNode(", "));
    span.appendChild(typeof o["url"] === "string" ? link(o["url"] as string, o["name"] as string) : document.createTextNode(o["name"] as string));
  });
  return span;
}

/** Arrays of objects as one entry per item: a linked title and a meta line. */
function itemList(items: unknown[], lang: Lang, now: number, style: "read" | "exact"): HTMLElement {
  const list = el("ol", "items");
  for (const item of items.slice(0, 50)) {
    const li = el("li");
    const o = isObj(item) ? item : {};
    const title = nameOf(item);
    const head = el("div", "it-title");
    if (typeof o["url"] === "string" && title !== o["url"]) head.appendChild(link(o["url"] as string, title));
    else head.textContent = title;
    li.appendChild(head);
    const meta = isObj(item) ? itemMeta(o, lang, now, style) : "";
    if (meta) li.appendChild(el("div", "it-meta", meta));
    list.appendChild(li);
  }
  if (items.length > 50) list.appendChild(el("li", "it-more", t("moreItems", String(items.length - 50))));
  return list;
}

/** `[{name, value}]` arrays (a quote's extra figures) as label/value pairs. */
const isPairs = (v: unknown[]): boolean => v.length > 0 && v.every((x) => isObj(x) && "value" in x && ("name" in x || "label" in x));

function pairs(v: unknown[], lang: Lang, style: "read" | "exact"): HTMLElement {
  // Two columns for short figures; one when a name or value is long (product specs).
  const wide = v.some((x) => {
    const o = x as Record<string, unknown>;
    return String(o["value"]).length > 24 || String(o["name"] ?? o["label"]).length > 24;
  });
  const g = el("div", wide ? "pairs one" : "pairs");
  for (const x of v.slice(0, 60)) {
    const o = x as Record<string, unknown>;
    const row = el("div");
    row.append(el("span", "pk", String(o["name"] ?? o["label"])), scalarNode("value", o["value"], lang, style));
    g.appendChild(row);
  }
  return g;
}

function valueNode(key: string, value: unknown, lang: Lang, now: number, style: "read" | "exact"): HTMLElement {
  const k = snake(key);
  if (Array.isArray(value)) {
    if (value.every((x) => typeof x === "string" || typeof x === "number")) {
      const strs = value.map(String);
      // Links as links, sentences as a bulleted list, short words (tags) as chips.
      if (strs.every((x) => /^https?:\/\//.test(x))) {
        const ul = el("ul", "links");
        for (const u of strs.slice(0, 30)) {
          const li = el("li");
          li.appendChild(link(u, style === "read" ? shortUrl(u) : u));
          ul.appendChild(li);
        }
        return ul;
      }
      if (strs.some((x) => x.length > 40)) {
        const ul = el("ul", "bullets");
        for (const x of strs.slice(0, 50)) ul.appendChild(el("li", "", x));
        return ul;
      }
      const chips = el("div", "chips");
      for (const x of strs) chips.appendChild(el("span", "chip", x));
      return chips;
    }
    if (isPairs(value)) return pairs(value, lang, style);
    if (isNameList(value)) return nameList(value);
    return itemList(value, lang, now, style);
  }
  if (isObj(value)) {
    const name = nameOf(value);
    return typeof value["url"] === "string" && name !== value["url"] ? link(value["url"] as string, name) : el("span", "", name);
  }
  if (k === "body_html" && typeof value === "string") return el("span", "muted", t("htmlChars", value.length.toLocaleString()));
  if (typeof value === "string" && value.length > LONG) return folded(value, "long");
  return scalarNode(key, value, lang, style);
}

// ---- reader -------------------------------------------------------------------------------------------

function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return "";
  }
}

function renderReader(box: HTMLElement, ext: Extraction, rec: Record<string, unknown>, lang: Lang): void {
  const now = Date.now();
  const keys = Object.keys(rec).filter((k) => k !== "@type");
  const done = new Set<string>();
  const take = (k: string): boolean => keys.includes(k) && !done.has(k) && rec[k] !== null && rec[k] !== "" && (done.add(k), true);
  const kind = ext.schema.split(".").pop() ?? "";

  const hero = el("section", "hero");
  hero.appendChild(el("div", "site", [hostOf(ext.url), typeLabel(ext.schema, lang)].filter(Boolean).join(" · ")));
  const titleKey = TITLE_KEYS.find((k) => keys.includes(k) && typeof rec[k] === "string");
  const h = el("h1", "title");
  if (titleKey && take(titleKey)) h.appendChild(mark(el("span", "", rec[titleKey] as string), titleKey));
  if (kind === "Quote") {
    if (take("symbol")) h.appendChild(mark(el("span", "symbol", String(rec["symbol"])), "symbol"));
    if (take("marketState")) h.appendChild(mark(el("span", "pill", formatScalar("marketState", rec["marketState"], PREFIXES, lang, "read").text), "marketState"));
  }
  if (h.childNodes.length > 0) hero.appendChild(h);

  // The headline number of a quote or a product: price, currency, change.
  if (keys.includes("price")) {
    const p = el("div", "price");
    if (take("price")) p.appendChild(mark(el("span", "big", formatScalar("price", rec["price"], PREFIXES, lang, "read").text), "price"));
    if (take("currency")) p.appendChild(mark(el("span", "cur", String(rec["currency"])), "currency"));
    for (const k of ["change", "changePercent"]) {
      if (!take(k)) continue;
      const f = formatScalar(k, rec[k], PREFIXES, lang, "read");
      p.appendChild(mark(el("span", `chg${f.tone ? ` tone-${f.tone}` : ""}`, k === "changePercent" ? `(${f.text})` : f.text), k));
    }
    if (kind !== "Quote" && take("availability")) {
      p.appendChild(mark(el("span", "pill", formatScalar("availability", rec["availability"], PREFIXES, lang, "read").text), "availability"));
    }
    hero.appendChild(p);
  }
  const ledeKey = LEDE_KEYS.find((k) => keys.includes(k) && typeof rec[k] === "string" && !done.has(k));
  if (ledeKey && take(ledeKey)) hero.appendChild(mark(folded(rec[ledeKey] as string, "lede"), ledeKey));
  box.appendChild(hero);

  // Facts: scalars and small values in two columns; a quote's figures in a grid, its links below it.
  const facts = el("dl", kind === "Quote" ? "facts grid" : "facts");
  const after = el("dl", "facts");
  const blocks: HTMLElement[] = [];
  for (const k of keys) {
    if (done.has(k) || rec[k] === null || rec[k] === "") continue;
    const v = rec[k];
    const sk = snake(k);
    const big =
      (typeof v === "string" && sk !== "body_html" && (v.length > LONG || sk === "body_text")) ||
      (Array.isArray(v) && v.length > 0 && !isNameList(v) && (v.some(isObj) || v.some((x) => typeof x === "string" && x.length > 40)));
    if (big) continue;
    done.add(k);
    const row = mark(el("div", "fact"), k);
    row.append(el("dt", "", fieldLabel(k, lang)));
    const dd = el("dd");
    dd.appendChild(valueNode(k, v, lang, now, "read"));
    row.appendChild(dd);
    const wide = typeof v === "string" && (/^https?:\/\//.test(v) || sk === "body_html");
    (kind === "Quote" && (wide || Array.isArray(v)) ? after : facts).appendChild(row);
  }
  if (facts.childNodes.length > 0) box.appendChild(facts);
  if (after.childNodes.length > 0) box.appendChild(after);

  // Long text and lists, each under its own heading.
  for (const k of keys) {
    if (done.has(k) || rec[k] === null || rec[k] === "") continue;
    done.add(k);
    const v = rec[k];
    const sec = mark(el("section", "block"), k);
    const head = el("div", "block-head", fieldLabel(k, lang));
    if (Array.isArray(v)) head.appendChild(el("span", "count", t("itemsN", String(v.length))));
    else if (typeof v === "string") head.appendChild(el("span", "count", t("charsN", v.length.toLocaleString())));
    sec.appendChild(head);
    sec.appendChild(valueNode(k, v, lang, now, "read"));
    blocks.push(sec);
  }
  for (const b of blocks) box.appendChild(b);
}

// ---- table --------------------------------------------------------------------------------------------

const ORIGIN_KEYS: Record<string, Parameters<typeof t>[0]> = {
  SITE_SELECTOR: "originSite",
  JSON_LD: "originJsonLd",
  META: "originMeta",
  OPEN_GRAPH: "originOpenGraph",
  MICRODATA: "originMicrodata",
  URL: "originUrl",
  CONST: "originConst",
  READABILITY: "originReadability",
};

function originOf(sources: FieldSource[], key: string): string | null {
  const k = snake(key);
  const s = sources.find((x) => x.path === k) ?? sources.find((x) => x.path.startsWith(`${k}[`) || x.path.startsWith(`${k}.`));
  return s ? s.origin.replace("FIELD_ORIGIN_", "") : null;
}

function renderTable(box: HTMLElement, ext: Extraction, rec: Record<string, unknown>, lang: Lang): void {
  const now = Date.now();
  const sources = ext.sources ?? [];
  const table = el("table", "fields");
  const used = new Set<string>();
  for (const [k, v] of Object.entries(rec)) {
    if (k === "@type") continue;
    const tr = mark(el("tr"), k);
    const name = el("td", "k", snake(k));
    if (Array.isArray(v)) name.appendChild(el("span", "count", ` [${v.length}]`));
    const val = el("td", "v");
    val.appendChild(valueNode(k, v, lang, now, "exact"));
    const dot = el("td", "s");
    const origin = originOf(sources, k);
    if (origin) {
      used.add(origin);
      const d = el("i", `dot o-${origin}`);
      d.title = t(ORIGIN_KEYS[origin] ?? "originOther");
      dot.appendChild(d);
    }
    tr.append(name, val, dot);
    table.appendChild(tr);
  }
  box.appendChild(table);
  if (used.size > 0) {
    const legend = el("div", "legend");
    for (const o of used) {
      const item = el("span");
      item.append(el("i", `dot o-${o}`), document.createTextNode(t(ORIGIN_KEYS[o] ?? "originOther")));
      legend.appendChild(item);
    }
    box.appendChild(legend);
  }
}

export function renderView(box: HTMLElement, view: View, ext: Extraction, rec: Record<string, unknown>, lang: Lang): void {
  box.replaceChildren();
  box.className = `record ${view}`;
  if (view === "table") renderTable(box, ext, rec, lang);
  else renderReader(box, ext, rec, lang);
}

// ---- JSON ---------------------------------------------------------------------------------------------

const TOKEN = /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;

/** Pretty JSON with keys, strings, numbers and literals in their own colors (text nodes only, no HTML). */
export function renderJson(pre: HTMLElement, text: string): void {
  pre.replaceChildren();
  let at = 0;
  for (const m of text.matchAll(TOKEN)) {
    const i = m.index ?? 0;
    if (i > at) pre.appendChild(document.createTextNode(text.slice(at, i)));
    const cls = m[1] !== undefined ? (m[2] !== undefined ? "j-key" : "j-str") : m[3] !== undefined ? "j-lit" : "j-num";
    pre.appendChild(el("span", cls, m[1] !== undefined ? m[1] : m[0]));
    if (m[2] !== undefined) pre.appendChild(document.createTextNode(m[2]));
    at = i + m[0].length;
  }
  if (at < text.length) pre.appendChild(document.createTextNode(text.slice(at)));
}
