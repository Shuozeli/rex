// Builds the MV3 extension into dist/: bundles TS (and the wasm-bindgen glue in wasm/) with esbuild,
// copies static files, icons and the engine binary (wasm/rex_wasm_bg.wasm, produced by
// `cargo build -p rex-wasm` + `wasm-bindgen --target web --out-dir extension/wasm`), renders
// _locales from src/messages.ts, and renders the manifest.
//
// Config: $REX_CONFIG, else config.local.json (machine-local, gitignored), else config.example.json.
// REX_E2E=1 adds <all_urls> host access and the rexDebug hooks so automated tests can drive the
// extension without a toolbar click. REX_DIST puts the build somewhere other than dist/.
import { build } from "esbuild";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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

if (!existsSync(join(root, "wasm", "rex_wasm_bg.wasm"))) {
  throw new Error("wasm/ is missing: build the engine first (scripts/build-extension.sh)");
}
rmSync(dist, { recursive: true, force: true });
mkdirSync(join(dist, "wasm"), { recursive: true });
cpSync(join(root, "wasm", "rex_wasm_bg.wasm"), join(dist, "wasm", "rex_wasm_bg.wasm"));

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

const origins = [...new Set(config.sources.map((s) => {
  const u = new URL(s.baseUrl);
  return `${u.protocol}//${u.hostname}/*`;
}))];
const icons = { 16: "icons/icon-16.png", 32: "icons/icon-32.png", 48: "icons/icon-48.png", 128: "icons/icon-128.png" };
const manifest = {
  manifest_version: 3,
  name: "__MSG_extName__",
  version: pkg.version,
  description: "__MSG_extDescription__",
  default_locale: "en",
  minimum_chrome_version: "137",
  icons,
  permissions: ["activeTab", "scripting", "storage", "alarms", "declarativeContent"],
  host_permissions: e2e ? [...origins, "<all_urls>"] : origins,
  optional_host_permissions: ["https://*/*", "http://*/*"],
  background: { service_worker: "background.js", type: "module" },
  action: { default_popup: "popup.html", default_title: "__MSG_extName__", default_icon: icons },
  options_page: "options.html",
  content_security_policy: { extension_pages: "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'" },
  commands: { _execute_action: { suggested_key: { default: "Alt+Shift+J" } } },
};
writeFileSync(join(dist, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`built ${dist.replace(root + "/", "")}/ v${pkg.version} (config: ${configPath.replace(root + "/", "")}${e2e ? ", e2e hooks" : ""})`);
