// Builds the MV3 extension into dist/: bundles TS (and the wasm-bindgen glue in wasm/) with esbuild,
// copies static files, icons, the engine binary (wasm/rex_wasm_bg.wasm, produced by
// `cargo build -p rex-wasm` + `wasm-bindgen --target web --out-dir extension/wasm`) and the recipes
// (builtin/, produced by `rex bundle embed --out extension/builtin`; decisions.md U45), renders
// _locales from src/messages.ts, and renders the manifest. scripts/build-extension.sh does all three steps.
//
// The extension never connects to the internet: the recipes are in the package, the manifest asks for no
// host access, and CSP allows connections to the package only.
//
// Config: $REX_CONFIG, else config.local.json (machine-local, gitignored), else config.example.json.
// REX_E2E=1 adds <all_urls> host access and the rexDebug hooks so automated tests can drive the
// extension without a toolbar click. REX_DIST puts the build somewhere other than dist/; REX_BUILTIN reads
// the recipes from another directory than builtin/.
import { build } from "esbuild";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const dist = process.env.REX_DIST ? resolve(process.cwd(), process.env.REX_DIST) : join(root, "dist");
const e2e = process.env.REX_E2E === "1";
const configPath = process.env.REX_CONFIG
  ? (isAbsolute(process.env.REX_CONFIG) ? process.env.REX_CONFIG : resolve(process.cwd(), process.env.REX_CONFIG))
  : join(root, existsSync(join(root, "config.local.json")) ? "config.local.json" : "config.example.json");
const config = JSON.parse(readFileSync(configPath, "utf8"));
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

const builtin = process.env.REX_BUILTIN ? resolve(process.cwd(), process.env.REX_BUILTIN) : join(root, "builtin");
if (!existsSync(join(root, "wasm", "rex_wasm_bg.wasm"))) {
  throw new Error("wasm/ is missing: build the engine first (scripts/build-extension.sh)");
}
const RECIPE_FILES = ["index.json", "schemas.fds", "recipes.json"];
for (const f of RECIPE_FILES) {
  if (!existsSync(join(builtin, f))) {
    throw new Error(`${builtin}/${f} is missing: run \`rex bundle embed --out extension/builtin\` (scripts/build-extension.sh)`);
  }
}
rmSync(dist, { recursive: true, force: true });
mkdirSync(join(dist, "wasm"), { recursive: true });
cpSync(join(root, "wasm", "rex_wasm_bg.wasm"), join(dist, "wasm", "rex_wasm_bg.wasm"));
mkdirSync(join(dist, "recipes"), { recursive: true });
for (const f of RECIPE_FILES) cpSync(join(builtin, f), join(dist, "recipes", f));
const recipeCount = JSON.parse(readFileSync(join(builtin, "index.json"), "utf8")).entries.length;

// Enum value prefixes of the record schemas ("INSTRUMENT_TYPE_", …): the popup shows enum values without
// them ("INSTRUMENT_TYPE_STOCK" -> "Stock"). Proto style puts the enum's name in SCREAMING_SNAKE before each value.
function enumPrefixes(dir) {
  const out = new Set();
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) for (const x of enumPrefixes(p)) out.add(x);
    else if (e.name.endsWith(".proto")) {
      for (const m of readFileSync(p, "utf8").matchAll(/^\s*enum\s+(\w+)\s*\{/gm)) {
        out.add(`${m[1].replace(/([a-z0-9])([A-Z])/g, "$1_$2").toUpperCase()}_`);
      }
    }
  }
  return [...out].sort();
}
const ENUM_PREFIXES = enumPrefixes(join(root, "..", "proto", "rex", "schema"));

await build({
  entryPoints: {
    background: join(root, "src/background.ts"),
    popup: join(root, "src/popup.ts"),
    options: join(root, "src/options.ts"),
  },
  outdir: dist,
  bundle: true,
  format: "esm",
  target: "chrome137",
  sourcemap: false,
  // Folds __REX_E2E__ and drops the dead test-hook branch from store builds (identifiers stay readable).
  minifySyntax: true,
  define: {
    __REX_CONFIG__: JSON.stringify(config),
    __REX_VERSION__: JSON.stringify(pkg.version),
    __REX_E2E__: JSON.stringify(e2e),
    __REX_ENUM_PREFIXES__: JSON.stringify(ENUM_PREFIXES),
  },
  logLevel: "warning",
});

// _locales from the typed message tables.
const bundled = await build({
  entryPoints: [join(root, "src/messages.ts")],
  bundle: true,
  format: "esm",
  write: false,
  logLevel: "warning",
});
const messages = await import(`data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString("base64")}`);
for (const [locale, table] of [["en", messages.en], ["zh_CN", messages.zh_CN]]) {
  const json = Object.fromEntries(Object.entries(table).map(([k, message]) => [k, { message }]));
  mkdirSync(join(dist, "_locales", locale), { recursive: true });
  writeFileSync(join(dist, "_locales", locale, "messages.json"), JSON.stringify(json, null, 2));
}

for (const f of ["popup.html", "popup.css", "options.html"]) {
  cpSync(join(root, "static", f), join(dist, f));
}
cpSync(join(root, "static", "icons"), join(dist, "icons"), { recursive: true });

// Every data-i18n key used by the pages must exist.
for (const f of ["popup.html", "options.html"]) {
  const html = readFileSync(join(root, "static", f), "utf8");
  for (const m of html.matchAll(/data-i18n(?:-title|-placeholder)?="([^"]+)"/g)) {
    if (!(m[1] in messages.en)) throw new Error(`${f}: unknown message key "${m[1]}"`);
  }
}

const icons = { 16: "icons/icon-16.png", 32: "icons/icon-32.png", 48: "icons/icon-48.png", 128: "icons/icon-128.png" };
const manifest = {
  manifest_version: 3,
  name: "__MSG_extName__",
  version: pkg.version,
  description: "__MSG_extDescription__",
  default_locale: "en",
  minimum_chrome_version: "137",
  icons,
  permissions: ["activeTab", "scripting", "storage", "declarativeContent"],
  // Test builds only: the e2e hooks read tabs without a toolbar click. The store build asks for no host access.
  ...(e2e ? { host_permissions: ["<all_urls>"] } : {}),
  background: { service_worker: "background.js", type: "module" },
  action: { default_popup: "popup.html", default_title: "__MSG_extName__", default_icon: icons },
  options_page: "options.html",
  // No network at all (decisions.md U45): fetch() reaches the package only (the engine and the recipes), and
  // images, media, fonts and frames stay inside it. tests/e2e and the recorder check the requests made.
  content_security_policy: {
    extension_pages:
      "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'; img-src 'self' data:; media-src 'none'; " +
      "font-src 'self'; frame-src 'none'; connect-src 'self'",
  },
  commands: { _execute_action: { suggested_key: { default: "Alt+Shift+J" } } },
};
writeFileSync(join(dist, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`built ${dist.replace(root + "/", "")}/ v${pkg.version} (${recipeCount} recipes; config: ${configPath.replace(root + "/", "")}${e2e ? ", e2e hooks" : ""})`);
