// Turning record values into something people read: dates, numbers, durations, enum names and field labels,
// in English or Chinese. Pure functions (the language is a parameter), so tests run them without Chrome.
// The JSON a user copies is untouched; this is display only.

export type Lang = "en" | "zh";

export function langOf(uiLanguage: string): Lang {
  return uiLanguage.toLowerCase().startsWith("zh") ? "zh" : "en";
}

const intlLocale = (lang: Lang): string => (lang === "zh" ? "zh-CN" : "en-US");

/** camelCase (protojson) -> snake_case (the schema's field name). */
export const snake = (key: string): string => key.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

// ---- field labels -------------------------------------------------------------------------------------

const LABELS: Record<string, [string, string]> = {
  title: ["Title", "标题"],
  name: ["Name", "名称"],
  headline: ["Headline", "标题"],
  subtitle: ["Subtitle", "副标题"],
  summary: ["Summary", "摘要"],
  description: ["Description", "简介"],
  authors: ["Authors", "作者"],
  author: ["Author", "作者"],
  published_at: ["Published", "发布"],
  updated_at: ["Updated", "更新"],
  canonical_url: ["Link", "链接"],
  url: ["Link", "链接"],
  image: ["Image", "图片"],
  images: ["Images", "图片"],
  thumbnail: ["Thumbnail", "封面"],
  body_text: ["Text", "正文"],
  body_html: ["HTML", "正文 HTML"],
  tags: ["Tags", "标签"],
  section: ["Section", "栏目"],
  language: ["Language", "语言"],
  site_name: ["Site", "网站"],
  channel: ["Channel", "频道"],
  duration_seconds: ["Duration", "时长"],
  views: ["Views", "播放"],
  likes: ["Likes", "点赞"],
  comments: ["Comments", "评论"],
  comment_count: ["Comments", "评论"],
  reply_count: ["Replies", "回复"],
  score: ["Score", "分数"],
  rank: ["Rank", "排名"],
  id: ["ID", "ID"],
  sku: ["SKU", "SKU"],
  brand: ["Brand", "品牌"],
  price: ["Price", "价格"],
  currency: ["Currency", "币种"],
  availability: ["Availability", "库存"],
  rating: ["Rating", "评分"],
  review_count: ["Reviews", "评价数"],
  seller: ["Seller", "卖家"],
  symbol: ["Symbol", "代码"],
  exchange: ["Exchange", "交易所"],
  type: ["Type", "类型"],
  market_state: ["Market", "市场状态"],
  change: ["Change", "涨跌"],
  change_percent: ["Change %", "涨跌幅"],
  previous_close: ["Prev. close", "昨收"],
  open: ["Open", "今开"],
  day_low: ["Day low", "最低"],
  day_high: ["Day high", "最高"],
  year_low: ["52-wk low", "52 周低"],
  year_high: ["52-wk high", "52 周高"],
  volume: ["Volume", "成交量"],
  market_cap: ["Market cap", "市值"],
  pe_ratio: ["P/E", "市盈率"],
  eps: ["EPS", "每股收益"],
  as_of: ["As of", "时间"],
  stats: ["More figures", "更多数据"],
  items: ["Items", "条目"],
  results: ["Results", "结果"],
  query: ["Query", "搜索词"],
  engine: ["Engine", "搜索引擎"],
  ingredients: ["Ingredients", "食材"],
  instructions: ["Steps", "步骤"],
  steps: ["Steps", "步骤"],
  prep_time_seconds: ["Prep time", "准备时间"],
  cook_time_seconds: ["Cook time", "烹饪时间"],
  total_time_seconds: ["Total time", "总时间"],
  servings: ["Servings", "份量"],
  yield: ["Yield", "份量"],
  start_date: ["Starts", "开始"],
  end_date: ["Ends", "结束"],
  start_at: ["Starts", "开始"],
  end_at: ["Ends", "结束"],
  location: ["Location", "地点"],
  address: ["Address", "地址"],
  organizer: ["Organizer", "主办方"],
  status: ["Status", "状态"],
  company: ["Company", "公司"],
  salary: ["Salary", "薪资"],
  employment_type: ["Employment", "雇佣类型"],
  release_date: ["Released", "上映"],
  genres: ["Genres", "类型"],
  creators: ["Creators", "主创"],
  isbn: ["ISBN", "ISBN"],
  publisher: ["Publisher", "出版社"],
  question: ["Question", "问题"],
  answers: ["Answers", "回答"],
  replies: ["Replies", "回复"],
  post: ["Post", "帖子"],
  phone: ["Phone", "电话"],
  hours: ["Hours", "营业时间"],
};

/** A field's display label: known fields are translated, others become "Sentence case". */
export function fieldLabel(key: string, lang: Lang): string {
  const k = snake(key);
  const known = LABELS[k];
  if (known) return lang === "zh" ? known[1] : known[0];
  const words = k.replace(/_/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

// ---- record types -------------------------------------------------------------------------------------

const TYPES: Record<string, [string, string]> = {
  Article: ["Article", "文章"],
  Feed: ["List", "列表"],
  Video: ["Video", "视频"],
  VideoList: ["Video list", "视频列表"],
  Product: ["Product", "商品"],
  ProductList: ["Product list", "商品列表"],
  SearchResults: ["Search results", "搜索结果"],
  Quote: ["Quote", "行情"],
  Thread: ["Discussion", "讨论"],
  Post: ["Post", "帖子"],
  Profile: ["Profile", "个人主页"],
  Recipe: ["Recipe", "菜谱"],
  HowTo: ["How-to", "教程"],
  FaqPage: ["FAQ", "常见问题"],
  Event: ["Event", "活动"],
  Business: ["Place", "商家"],
  JobPosting: ["Job", "职位"],
  App: ["App", "应用"],
  Title: ["Film / TV", "影视"],
  Book: ["Book", "图书"],
};

/** "rex.schema.article.v1.Article" -> "Article" / "文章". */
export function typeLabel(schema: string, lang: Lang): string {
  const name = schema.split(".").pop() ?? schema;
  const known = TYPES[name];
  return known ? (lang === "zh" ? known[1] : known[0]) : name;
}

// ---- enums --------------------------------------------------------------------------------------------

const ENUM_ZH: Record<string, string> = {
  STOCK: "股票", ETF: "ETF", FUND: "基金", INDEX: "指数", CRYPTO: "加密货币", CURRENCY: "外汇", BOND: "债券",
  FUTURE: "期货", OPTION: "期权", OPEN: "交易中", CLOSED: "已收盘", PRE: "盘前", POST: "盘后", HALTED: "停牌",
  IN_STOCK: "有货", OUT_OF_STOCK: "缺货", PRE_ORDER: "预售", LIMITED_AVAILABILITY: "库存紧张", SOLD_OUT: "售罄",
  BACK_ORDER: "缺货待补", DISCONTINUED: "已停产", ONLINE_ONLY: "仅限线上", IN_STORE_ONLY: "仅限门店",
  NEW: "全新", USED: "二手", REFURBISHED: "翻新", DAMAGED: "有损",
  SCHEDULED: "如期举行", CANCELLED: "已取消", POSTPONED: "已延期", RESCHEDULED: "已改期", MOVED_ONLINE: "改为线上",
  OFFLINE: "线下", ONLINE: "线上", MIXED: "线上线下",
  ORGANIC: "自然结果", AD: "广告", VIDEO: "视频", NEWS: "新闻", IMAGE: "图片",
  MOVIE: "电影", TV_SERIES: "剧集", TV_EPISODE: "单集", FULL_TIME: "全职", PART_TIME: "兼职", CONTRACTOR: "合同工",
  TEMPORARY: "临时", INTERN: "实习",
};

/**
 * An enum value without its type prefix, as words: "INSTRUMENT_TYPE_STOCK" -> "Stock" / "股票". `prefixes` are
 * the schema enums' value prefixes ("INSTRUMENT_TYPE_", …), read from the protos at build time. Returns null
 * for strings that are not enum values.
 */
export function enumLabel(value: string, prefixes: readonly string[], lang: Lang): string | null {
  if (!/^[A-Z][A-Z0-9]*(_[A-Z0-9]+)+$/.test(value)) return null;
  const prefix = prefixes.filter((p) => value.startsWith(p)).sort((a, b) => b.length - a.length)[0];
  if (!prefix) return null;
  const rest = value.slice(prefix.length);
  if (lang === "zh" && ENUM_ZH[rest]) return ENUM_ZH[rest];
  const words = rest.toLowerCase().replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

// ---- numbers, durations, dates ------------------------------------------------------------------------

const NUMERIC = /^-?\d+(\.\d+)?$/;

/** A number or a numeric string (protojson writes int64 and decimals as strings). */
export function asNumber(v: unknown): number | null {
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (typeof v === "string" && NUMERIC.test(v)) return Number(v);
  return null;
}

/** Digits grouped, decimals kept as written: "1436.13" -> "1,436.13", "1450000000000" -> "1,450,000,000,000". */
export function grouped(v: string | number, lang: Lang): string {
  const s = String(v);
  const decimals = s.includes(".") ? s.split(".")[1]!.length : 0;
  return new Intl.NumberFormat(intlLocale(lang), { minimumFractionDigits: decimals, maximumFractionDigits: decimals }).format(Number(s));
}

/** Large counts made short: 31001400 -> "31M" / "3100万"; below 10,000 they stay grouped. */
export function compact(v: number, lang: Lang): string {
  if (Math.abs(v) < 10_000) return grouped(v, lang);
  return new Intl.NumberFormat(intlLocale(lang), { notation: "compact", maximumSignificantDigits: 3 }).format(v);
}

/** Seconds as a clock: 217 -> "3:37", 3725 -> "1:02:05". */
export function duration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** Seconds in words, for times that are not a media length: 1200 -> "20 min" / "20 分钟", 3900 -> "1 hr 5 min". */
export function humanDuration(seconds: number, lang: Lang): string {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const rest = s % 60;
  const [H, M, S] = lang === "zh" ? [" 小时", " 分钟", " 秒"] : [" hr", " min", " s"];
  const parts = [h ? `${h}${H}` : "", m ? `${m}${M}` : "", !h && !m ? `${rest}${S}` : ""].filter(Boolean);
  return parts.join(" ");
}

const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/;

export const isTimestamp = (v: unknown): v is string => typeof v === "string" && ISO.test(v);

/** A timestamp for reading: "17 Sep 2005, 18:57" / "2005年9月17日 18:57" (just the date at midnight UTC). */
export function readableTime(iso: string, lang: Lang): string {
  const d = new Date(iso);
  const midnight = /T00:00(:00(\.0+)?)?(Z|\+00:00)$/.test(iso);
  return new Intl.DateTimeFormat(intlLocale(lang), {
    dateStyle: "medium",
    ...(midnight ? {} : { timeStyle: "short" }),
    ...(midnight ? { timeZone: "UTC" } : {}),
  }).format(d);
}

/** A timestamp for the table: "2005-09-17 18:57 UTC". */
export function exactTime(iso: string): string {
  const d = new Date(iso);
  return `${d.toISOString().slice(0, 10)} ${d.toISOString().slice(11, 16)} UTC`;
}

/** "8 hours ago" / "8小时前" within a week of `now`, else the readable date. */
export function relativeTime(iso: string, now: number, lang: Lang): string {
  const diff = (new Date(iso).getTime() - now) / 1000;
  const rtf = new Intl.RelativeTimeFormat(intlLocale(lang), { numeric: "auto" });
  const abs = Math.abs(diff);
  if (abs < 60) return rtf.format(Math.round(diff), "second");
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86_400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 7 * 86_400) return rtf.format(Math.round(diff / 86_400), "day");
  return readableTime(iso, lang);
}

/** "https://en.wikipedia.org/wiki/Web_scraping" -> "en.wikipedia.org/wiki/Web_scraping". */
export function shortUrl(url: string, max = 60): string {
  const s = url.replace(/^https?:\/\/(www\.)?/, "").replace(/\/$/, "");
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

// ---- what kind of value a field holds ------------------------------------------------------------------

const COUNTS = /(^|_)(views|likes|dislikes|volume|count|score|comments|replies|followers|subscribers|shares|market_cap|stars|forks|votes|answers|plays|reviews)$/;
const SIGNED = /^(change|change_percent|.*_change|.*_change_percent)$/;
/** Identifiers that look like numbers but are not quantities: never grouped ("600519", not "600,519"). */
const IDENT = /(^|_)(id|ids|symbol|ticker|code|sku|isbn|gtin|upc|ean|asin|zip|postal_code|phone|year|number)$/;

export type Kind = "duration" | "count" | "signed" | "percent" | "number" | "time" | "url" | "enum" | "text" | "bool";

/** How to show a scalar field, from its name and value. */
export function kindOf(key: string, value: unknown, prefixes: readonly string[]): Kind {
  const k = snake(key);
  if (typeof value === "boolean") return "bool";
  if (isTimestamp(value)) return "time";
  if (typeof value === "string" && /^https?:\/\//.test(value)) return "url";
  if (typeof value === "string" && enumLabel(value, prefixes, "en") !== null) return "enum";
  if (asNumber(value) !== null) {
    if (IDENT.test(k)) return "text";
    if (k.endsWith("_seconds")) return "duration";
    if (SIGNED.test(k)) return "signed";
    if (k.endsWith("_percent") || k.endsWith("_pct")) return "percent";
    if (COUNTS.test(k)) return "count";
    return "number";
  }
  return "text";
}

/**
 * A scalar for display. `style` "read" is the reader view (short counts, readable dates), "exact" the table
 * (every digit, ISO-like dates). Returns the text and an optional tone for signed numbers.
 */
export function formatScalar(
  key: string,
  value: unknown,
  prefixes: readonly string[],
  lang: Lang,
  style: "read" | "exact",
): { text: string; tone?: "up" | "down" } {
  const kind = kindOf(key, value, prefixes);
  const n = asNumber(value);
  switch (kind) {
    case "bool":
      return { text: value ? (lang === "zh" ? "是" : "Yes") : lang === "zh" ? "否" : "No" };
    case "time":
      return { text: style === "read" ? readableTime(value as string, lang) : exactTime(value as string) };
    case "url":
      return { text: style === "read" ? shortUrl(value as string) : (value as string) };
    case "enum":
      return { text: enumLabel(value as string, prefixes, lang) ?? String(value) };
    case "duration":
      // A video's length reads as a clock (3:37); a recipe's prep time as words (20 min), not "20:00".
      return { text: snake(key) === "duration_seconds" ? duration(n!) : humanDuration(n!, lang) };
    case "count":
      return { text: style === "read" ? compact(n!, lang) : grouped(String(value), lang) };
    case "percent":
      return { text: `${grouped(String(value), lang)}%` };
    case "signed": {
      const pct = snake(key).endsWith("percent");
      const body = `${n! > 0 ? "+" : ""}${grouped(String(value), lang)}${pct ? "%" : ""}`;
      return { text: body.replace(/^-/, "−"), tone: n! > 0 ? "up" : n! < 0 ? "down" : undefined };
    }
    case "number":
      return { text: grouped(String(value), lang) };
    default:
      return { text: String(value) };
  }
}
