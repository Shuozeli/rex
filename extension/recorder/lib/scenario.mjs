// Loads a scenario file and resolves its scenes. Shared by run.mjs and the hermetic fixture server, so both see
// the same pages.
//
// A scene is one of:
//   { name, label, url, expect }           a live page; `expect` = { recipe, status, minFields, fields }
//   { fixture: "fixtures/<id>/<case>" }    a fixture page: URL from meta.json, expectation from expected.json
//                                          (the exact recipe and version, the status, the top-level field count)
//   { recipeFixtures: "recipes/sites" }    one fixture scene per recipe under that directory: its first
//                                          captured (not synthetic) case, else its first case ("siteFixtures"
//                                          is the older name). A recipe without fixtures is an error, so every
//                                          recipe in the directory is covered.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

function yamlFiles(dir) {
  const out = [];
  for (const e of readdirSync(dir).sort()) {
    const p = join(dir, e);
    if (statSync(p).isDirectory()) out.push(...yamlFiles(p));
    else if (e.endsWith(".yaml")) out.push(p);
  }
  return out;
}

/** The fixture a site recipe is tested with: its first captured case, else its first case. */
export function fixtureFor(repo, recipeId) {
  const dir = join(repo, "fixtures", recipeId);
  if (!existsSync(dir)) return null;
  const cases = readdirSync(dir)
    .sort()
    .filter((c) => existsSync(join(dir, c, "meta.json")) && existsSync(join(dir, c, "expected.json")));
  const captured = cases.find((c) => JSON.parse(readFileSync(join(dir, c, "meta.json"), "utf8")).synthetic !== true);
  const pick = captured ?? cases[0];
  return pick ? relative(repo, join(dir, pick)) : null;
}

function expand(repo, s) {
  const dir = s.recipeFixtures ?? s.siteFixtures;
  if (!dir) return [s];
  return yamlFiles(join(repo, dir)).map((f) => {
    const id = readFileSync(f, "utf8").match(/^id:\s*"?([^"\s]+)"?/m)?.[1];
    if (!id) throw new Error(`${relative(repo, f)}: no id`);
    const fixture = fixtureFor(repo, id);
    if (!fixture) throw new Error(`${id}: no fixture under fixtures/${id}/ for its e2e scene`);
    return { name: id.replaceAll("/", "_"), label: id, fixture };
  });
}

function resolve(repo, s) {
  if (!s.fixture) return s;
  const dir = join(repo, s.fixture);
  const meta = JSON.parse(readFileSync(join(dir, "meta.json"), "utf8"));
  const exp = JSON.parse(readFileSync(join(dir, "expected.json"), "utf8"));
  const fields = Object.keys(exp.record ?? {}).filter((k) => k !== "@type").length;
  return {
    name: s.fixture.split("/").slice(-3).join("-"),
    label: s.fixture.replace(/^fixtures\//, ""),
    url: meta.url,
    expect: { recipe: `${exp.recipe.id}@${exp.recipe.version}`, status: exp.status.replace("EXTRACTION_STATUS_", ""), fields },
    ...s,
  };
}

export function loadScenario(file, repo) {
  const scenario = JSON.parse(readFileSync(file, "utf8"));
  scenario.scenes = scenario.scenes.flatMap((s) => expand(repo, s)).map((s) => resolve(repo, s));
  return scenario;
}

/** The fixture page of every fixture scene: "host/path?query" -> { file, scene }. */
export function fixturePages(scenario, repo) {
  const out = new Map();
  for (const s of scenario.scenes) {
    if (!s.fixture) continue;
    const u = new URL(s.url);
    const dir = join(repo, s.fixture);
    const file = existsSync(join(dir, "page.html")) ? join(dir, "page.html") : join(dir, "page.html.gz");
    const key = `${u.host}${u.pathname}${u.search}`;
    // One URL, one page: two different fixtures at the same URL would test each other's page.
    const prev = out.get(key);
    if (prev && prev.file !== file) throw new Error(`${key}: served by both ${prev.file} and ${file}`);
    out.set(key, { file, scene: s.name });
  }
  return out;
}
