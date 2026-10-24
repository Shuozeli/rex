// Shared types: build-time config, user settings, stored bundle state, engine output, and the message
// protocol between the popup/options pages and the service worker.

export interface PinnedKey {
  keyId: string;
  public: string;
}

export interface SourceConfig {
  id: string;
  baseUrl: string;
  channel: string;
  keys: PinnedKey[];
}

/** Build-time defaults (config.local.json or config.example.json). */
export interface RexConfig {
  sources: SourceConfig[];
  refreshMinutes: number;
  reportRepo: string;
}

declare global {
  const __REX_CONFIG__: RexConfig;
  const __REX_VERSION__: string;
  const __REX_E2E__: boolean;
}

export interface SourceSetting extends SourceConfig {
  enabled: boolean;
}

export type OutputFormat = "envelope" | "record";

/** User settings (FR-EX-9, FR-EX-10), stored in chrome.storage.local; defaults come from RexConfig. */
export interface Settings {
  /** In priority order: the first enabled source whose recipes match the URL is used. */
  sources: SourceSetting[];
  refreshMinutes: number;
  outputFormat: OutputFormat;
  devMode: boolean;
  /** One recipe as protojson (`rex fmt --json`); "" when unset. Unsigned, developer mode only. */
  localRecipe: string;
  localRecipeEnabled: boolean;
}

export interface UrlPattern {
  hosts?: string[];
  include?: string[];
  exclude?: string[];
}

/** Subset of rex.bundle.v1.Index (protojson) the extension reads. */
export interface BundleIndex {
  format: string;
  channel: string;
  sequence: string;
  generatedAt?: string;
  minClientVersion?: string;
  schemas: { sha256: string; bytes: string; messages?: string[] };
  entries: IndexEntry[];
}

export interface IndexEntry {
  ref: { id: string; version: number; sha256: string };
  schema: string;
  extends?: string;
  urls?: UrlPattern;
  base?: boolean;
  requires?: string[];
}

export interface SkippedEntry {
  id: string;
  reason: string;
}

/** What a source keeps in chrome.storage.local (written atomically as one value). */
export interface StoredSource {
  /** The source identity this bundle was verified for; a mismatch means "not synced". */
  baseUrl: string;
  channel: string;
  keysFingerprint: string;
  etag: string | null;
  sequence: string;
  index: BundleIndex;
  fdsSha: string;
  fdsBase64: string;
  /** sha256 -> canonical recipe protojson, for every entry that was not skipped. */
  recipes: Record<string, string>;
  skipped: SkippedEntry[];
  /** Engine features when the entries were selected; a newer engine re-selects (it may run skipped ones). */
  features?: string;
  lastSync: number;
  lastError: string | null;
}

export interface UrlHit {
  id: string;
  exactHost: boolean;
  includeMatched: boolean;
  priority: number;
}

export interface Plan {
  urlHits: UrlHit[];
  confirmed: string[];
  detectedTypes: string[];
  fallback: string | null;
  chosen: string | null;
}

export interface Attempt {
  candidate: number;
  recipe: string;
  source: string;
  raw: string[];
  outcome: string;
}

export interface Trace {
  path: string;
  attempts: Attempt[];
  winner: number | null;
  value: string[];
}

export interface Diagnostic {
  level: "info" | "warn";
  code: string;
  path: string;
  recipe: string;
  detail: string;
}

export interface FieldSource {
  path: string;
  origin: string;
  candidate?: number;
  recipe: string;
}

/** rex.extract.v1.Extraction as protojson. */
export interface Extraction {
  recipe?: { id: string; version: number; sha256?: string };
  schema: string;
  url: string;
  extractorVersion: string;
  fetchTime?: string;
  extractTime?: string;
  status: string;
  missingRequired?: string[];
  expectationsFailed?: string[];
  sources?: FieldSource[];
  bodyReadabilityAgreementBps?: number;
  record: Record<string, unknown> & { "@type": string };
  chain?: { id: string; version: number }[];
}

export interface EngineOutput {
  plan: Plan | null;
  extraction: Extraction | null;
  record: Record<string, unknown> | null;
  diagnostics: Diagnostic[];
  trace: Trace[];
  fuelUsed: number;
}

export interface SourceStatus {
  id: string;
  baseUrl: string;
  channel: string;
  enabled: boolean;
  hasPermission: boolean;
  sequence: string | null;
  generatedAt: string | null;
  recipeCount: number;
  skipped: SkippedEntry[];
  lastSync: number | null;
  lastError: string | null;
}

export interface Status {
  extensionVersion: string;
  engineVersion: string | null;
  sources: SourceStatus[];
  outputFormat: OutputFormat;
  devMode: boolean;
  /** Id of the developer-mode local recipe when it is active and valid. */
  localRecipeId: string | null;
  localRecipeError: string | null;
}

/** "Save as fixture" output; `rex fixture import` turns it into page.html + meta.json. */
export interface FixtureFile {
  format: "rex.fixture.v1";
  recipe: string | null;
  url: string;
  fetchTime: string;
  pageHtml: string;
}

export type Request =
  | { type: "extract"; tabId: number; recipeId?: string }
  | { type: "status" }
  | { type: "refresh" }
  | { type: "getSettings" }
  | { type: "saveSettings"; settings: Settings }
  | { type: "validateRecipe"; text: string }
  | { type: "fixture"; tabId: number; recipeId: string | null };

export type ExtractResponse =
  | {
      ok: true;
      page: { url: string; title: string };
      output: EngineOutput;
      sourceId: string;
      local: boolean;
      status: Status;
    }
  | { ok: false; error: string; status: Status | null };

export type StatusResponse = { ok: true; status: Status } | { ok: false; error: string; status: Status | null };

export type SettingsResponse = { ok: true; settings: Settings; status: Status } | { ok: false; error: string };

export type ValidateResponse = { ok: true; id: string } | { ok: false; error: string };

export type FixtureResponse = { ok: true; fixture: FixtureFile } | { ok: false; error: string };
