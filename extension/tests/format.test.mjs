// Unit tests for src/format.ts (display formatting in the popup). Bundled with esbuild, run with node:test:
//   node --test tests/format.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";
import { build } from "esbuild";

const out = await build({ entryPoints: [new URL("../src/format.ts", import.meta.url).pathname], bundle: true, format: "esm", write: false, logLevel: "warning" });
const f = await import(`data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString("base64")}`);
const PREFIXES = ["AVAILABILITY_", "INSTRUMENT_TYPE_", "MARKET_STATE_"];

test("numbers keep their decimals and gain grouping", () => {
  // Arrange
  const values = ["1436.13", "1450000000000", 7];

  // Act
  const got = values.map((v) => f.grouped(v, "en"));

  // Assert
  assert.deepEqual(got, ["1,436.13", "1,450,000,000,000", "7"]);
});

test("large counts are shortened, small ones stay exact", () => {
  // Arrange
  const values = [31001400, 9999];

  // Act
  const en = values.map((v) => f.compact(v, "en"));
  const zh = values.map((v) => f.compact(v, "zh"));

  // Assert
  assert.deepEqual(en, ["31M", "9,999"]);
  assert.deepEqual(zh, ["3100万", "9,999"]);
});

test("seconds read as a clock", () => {
  // Arrange
  const values = [217, 3725, 0];

  // Act
  const got = values.map(f.duration);

  // Assert
  assert.deepEqual(got, ["3:37", "1:02:05", "0:00"]);
});

test("enum values lose their type prefix and read as words", () => {
  // Arrange
  const values = ["INSTRUMENT_TYPE_STOCK", "AVAILABILITY_IN_STOCK", "MARKET_STATE_OPEN"];

  // Act
  const en = values.map((v) => f.enumLabel(v, PREFIXES, "en"));
  const zh = values.map((v) => f.enumLabel(v, PREFIXES, "zh"));

  // Assert
  assert.deepEqual(en, ["Stock", "In stock", "Open"]);
  assert.deepEqual(zh, ["股票", "有货", "交易中"]);
});

test("strings that are not enum values are left alone", () => {
  // Arrange
  const values = ["SSE", "CNY", "UNKNOWN_PREFIX_VALUE", "Hello world"];

  // Act
  const got = values.map((v) => f.enumLabel(v, PREFIXES, "en"));

  // Assert
  assert.deepEqual(got, [null, null, null, null]);
});

test("a price change carries its sign and tone", () => {
  // Arrange
  const cases = [["change", "-10.31"], ["changePercent", "-0.71"], ["change", "2.5"]];

  // Act
  const got = cases.map(([k, v]) => f.formatScalar(k, v, PREFIXES, "en", "read"));

  // Assert
  assert.deepEqual(got, [
    { text: "−10.31", tone: "down" },
    { text: "−0.71%", tone: "down" },
    { text: "+2.5", tone: "up" },
  ]);
});

test("timestamps are readable in the reader and exact in the table", () => {
  // Arrange
  const iso = "2005-09-17T18:57:30Z";

  // Act
  const exact = f.formatScalar("publishedAt", iso, PREFIXES, "en", "exact").text;
  const midnight = f.readableTime("2026-11-01T00:00:00Z", "en");

  // Assert
  assert.equal(exact, "2005-09-17 18:57 UTC");
  assert.equal(midnight, "Nov 1, 2026");
});

test("field kinds come from the name and the value", () => {
  // Arrange
  const cases = [
    ["durationSeconds", "217"],
    ["views", "3000000"],
    ["marketCap", "1450000000000"],
    ["price", "128.45"],
    ["canonicalUrl", "https://example.com/"],
    ["type", "INSTRUMENT_TYPE_STOCK"],
    ["title", "Web scraping"],
  ];

  // Act
  const got = cases.map(([k, v]) => f.kindOf(k, v, PREFIXES));

  // Assert
  assert.deepEqual(got, ["duration", "count", "count", "number", "url", "enum", "text"]);
});

test("unknown fields get a sentence-case label, known ones a translation", () => {
  // Arrange
  const keys = ["previousClose", "someNewField"];

  // Act
  const en = keys.map((k) => f.fieldLabel(k, "en"));
  const zh = keys.map((k) => f.fieldLabel(k, "zh"));

  // Assert
  assert.deepEqual(en, ["Prev. close", "Some new field"]);
  assert.deepEqual(zh, ["昨收", "Some new field"]);
});

test("identifiers that look like numbers are not grouped", () => {
  // Arrange
  const cases = [["symbol", "600519"], ["isbn", "9780743273565"], ["releaseYear", "2024"]];

  // Act
  const got = cases.map(([k, v]) => f.formatScalar(k, v, PREFIXES, "en", "exact").text);

  // Assert
  assert.deepEqual(got, ["600519", "9780743273565", "2024"]);
});

test("a video length reads as a clock, other durations as words", () => {
  // Arrange
  const cases = [["durationSeconds", "217"], ["prepTimeSeconds", "1200"], ["totalTimeSeconds", "3900"]];

  // Act
  const en = cases.map(([k, v]) => f.formatScalar(k, v, PREFIXES, "en", "read").text);
  const zh = cases.map(([k, v]) => f.formatScalar(k, v, PREFIXES, "zh", "read").text);

  // Assert
  assert.deepEqual(en, ["3:37", "20 min", "1 hr 5 min"]);
  assert.deepEqual(zh, ["3:37", "20 分钟", "1 小时 5 分钟"]);
});
