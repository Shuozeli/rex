<!-- agent-updated: 2026-10-13T06:00:00Z -->

# Operations

Running the recipe host: publishing, keys, garbage collection, and drift handling. The design is in
[decisions.md](decisions.md) U16, U26 and U30.

## Hosting layout

```
<bucket>/<prefix>/v1/<channel>/
  index.json          SignedIndex; Cache-Control: public, max-age=300
  r/<sha256>.json     recipes (immutable; max-age=31536000, immutable)
  s/<sha256>.fds      schema set (immutable)
  gc-state.json       when each unreferenced object was first seen (no-store)
```

Channels: `stable` holds `recipes/generic` and `recipes/sites`; `dev` holds all of `recipes/`, including
`recipes/drafts/`.

## One-time setup (Cloudflare R2)

1. Create an R2 bucket, for example `rex-recipes`, and give it public read on a custom domain. The
   extension source becomes `https://<domain>/<prefix>/v1/stable/`.
2. Create an R2 API token with object read and write on that bucket. The S3 endpoint is
   `https://<account-id>.r2.cloudflarestorage.com`, and the region is `auto`.
3. Generate the production signing key **offline**, outside the repo:

   ```bash
   rex bundle keygen --key-id rex-2026a --out ~/secure/rex-2026a.json
   ```

   It prints the public key. Put the key id and public key into the extension's build config
   (`config.local.json`, or the release config) as the pinned key.
4. In the GitHub repo, set:
   - **variables:** `REX_PUBLISH_ENABLED=true`, `REX_BUCKET`, `REX_PREFIX`, `REX_PUBLIC_URL`,
     `REX_KEY_ID`, `REX_PUBLIC_KEY`;
   - **secrets:** `REX_SIGNING_KEY` (the key file's JSON), `REX_S3_ENDPOINT`, `REX_S3_ACCESS_KEY_ID`,
     `REX_S3_SECRET_ACCESS_KEY`.
5. Push to `main`, or run **Publish recipes** manually. The workflow gates, publishes `stable` and `dev`,
   and verifies both through the public URL.

## Publishing by hand

```bash
export REX_S3_ENDPOINT=https://<account>.r2.cloudflarestorage.com
export REX_S3_ACCESS_KEY_ID=… REX_S3_SECRET_ACCESS_KEY=… REX_S3_REGION=auto
rex bundle publish --channel stable --recipes recipes/generic --recipes recipes/sites \
  --key ~/secure/rex-2026a.json --to s3://rex-recipes/rex-bundle
rex bundle verify --url https://<domain>/rex-bundle/v1/stable/ --key-id rex-2026a --public <hex>
```

`--dry-run` lists what would be uploaded. If nothing changed, publish says so and does not bump the
sequence (`--force` re-signs anyway). A directory works as a target too (`--to ~/.local/share/rex/bundle`),
which is how the tailnet dev bundle is served through serve-lib.

## Garbage collection

```bash
rex bundle gc --channel stable --key ~/secure/rex-2026a.json --to s3://rex-recipes/rex-bundle --retention-days 30
```

- gc verifies the live index before doing anything.
- Unreferenced objects are first **marked** (`gc-state.json`) and deleted only once the retention period
  has passed, so clients on an older index keep working.
- The publish workflow runs gc weekly. Never run gc and publish for the same channel at the same time
  outside CI; CI serializes them in one concurrency group.

## Key rotation

1. Generate the new key (`rex-2026b`).
2. Ship an extension release that pins **both** keys (`keys: [rex-2026a, rex-2026b]`), and wait until it
   has rolled out.
3. Switch publishing to the new key. The live index is still signed by the old key, so pass it as trusted
   for the first publish: set the repo variable `REX_TRUST_PREVIOUS=rex-2026a:<hex>` (or pass
   `--trust rex-2026a:<hex>` by hand). The sequence continues, so clients see no rollback.
4. Remove `REX_TRUST_PREVIOUS`. In a later extension release, drop the old key.

Clients keep a rollback floor per (base URL, channel), apart from the stored bundle: changing the pinned
keys forgets the stored bundle (it is bound to the keys that verified it) but never lowers the floor, so the
sequence must keep growing across a rotation.

**If a key is compromised**, publish a new bundle with a fresh key and a sequence above anything the
attacker could have signed, and ship an extension release that pins only the fresh key. Until that release
is installed, clients still accept anything the compromised key signs — the extension cannot tell an
attacker's index from yours. Once it is installed, the stored bundle (verified by the dropped key) is
forgotten and the next sync fetches the fresh one; until it succeeds the extension has no bundle. A
compromised key can also raise the floor out of reach (a huge sequence); then move the release to a new
channel name or base URL, which starts its own floor.

## Testing a host locally (MinIO)

```bash
docker run -d --name rex-minio -p $TAILSCALE_IP:9900:9000 \
  -e MINIO_ROOT_USER=<user> -e MINIO_ROOT_PASSWORD=<secret> -v ~/.local/share/rex/minio-data:/data \
  minio/minio:RELEASE.2025-04-22T22-12-26Z server /data
docker run --rm --network host -e MC_HOST_rex=http://<user>:<secret>@$TAILSCALE_IP:9900 \
  minio/mc:RELEASE.2025-04-16T18-13-26Z mb rex/rex-recipes
docker run --rm --network host -e MC_HOST_rex=http://<user>:<secret>@$TAILSCALE_IP:9900 \
  minio/mc:RELEASE.2025-04-16T18-13-26Z anonymous set download rex/rex-recipes
REX_S3_ENDPOINT=http://$TAILSCALE_HOST:9900 REX_S3_REGION=us-east-1 REX_S3_ACCESS_KEY_ID=<user> \
  REX_S3_SECRET_ACCESS_KEY=<secret> rex bundle publish --key <dev key> --to s3://rex-recipes/rex-bundle
```

`extension/tests/e2e-sync.mjs` can also sync from such a host:
`REX_E2E_MINIO=http://$TAILSCALE_HOST:9900/rex-recipes/rex-bundle/v1/dev/ REX_E2E_MINIO_KEY=<id>:<hex>`.

## Live-page test in a real Chrome (ARM Windows)

`extension/tests/live-remote.mjs` drives a separately running Chrome with the extension loaded. It opens the
real pages from `extension/tests/live-cases.json` over that machine's residential network, extracts through
the extension, and compares the matched recipe, the status and the field set with each case's fixture
(decisions.md U38). The browser runs on the ARM Windows host (`armwinroot-yuacx`, SSH `cyuan@100.77.171.126`):

- **Chrome:** Chrome for Testing in `C:\Users\cyuan\rex-live\chrome-win64`. Branded Chrome ignores
  `--load-extension` since M137.
- **Profile:** its own, in `rex-live\profile`, separate from the 15 Chrome instances that host runs for other
  jobs. Never use their `taskkill /IM chrome.exe` procedure.
- **Extension:** the e2e build (`pnpm build:e2e`, which syncs the serve-lib dev bundle) unpacked into
  `rex-live\ext`.
- **Launch:** the scheduled task `rex-live-chrome` runs `rex-live\start-chrome.cmd`. A process started from
  an SSH session dies when the session closes.
- **CDP:** on `127.0.0.1:9530` on that host, reached through an SSH tunnel.

```bash
# update the extension, restart that Chrome only, open the tunnel, run
pnpm -C extension build:e2e && tar -czf /tmp/rex-ext.tgz -C extension/dist .
sshpass -p <pw> scp /tmp/rex-ext.tgz cyuan@100.77.171.126:rex-live/rex-ext.tgz
sshpass -p <pw> ssh cyuan@100.77.171.126 "powershell -Command \"Remove-Item -Recurse -Force C:\Users\cyuan\rex-live\ext; New-Item -ItemType Directory C:\Users\cyuan\rex-live\ext; tar -xzf C:\Users\cyuan\rex-live\rex-ext.tgz -C C:\Users\cyuan\rex-live\ext\""
sshpass -p <pw> ssh cyuan@100.77.171.126 "powershell -ExecutionPolicy Bypass -File C:\Users\cyuan\rex-live\restart.ps1"
sshpass -p <pw> ssh -N -L 127.0.0.1:9530:127.0.0.1:9530 cyuan@100.77.171.126 &
node extension/tests/live-remote.mjs --ext-id nhclffgaalnjhlfdmkmbokplmikibheo   # --only <substring>
```

- **Extension id:** it follows from the install path (`rex-live\ext`). The runner wakes the idle MV3 service
  worker by opening the extension's options page.
- **Results:** a summary, plus each live extraction in `extension/test-results/live/`. `--save-html` keeps
  the page HTML as well.
- **How it drives the browser:** raw CDP to the extension's service worker only. The worker opens each page
  with `chrome.tabs.create`, waits for it to load plus 5 s, extracts with the extension's own code path,
  times the extraction, and closes the tab.
  - No debugger attaches to the pages. Playwright did, stalled on pages that stream updates (Yahoo
    Finance), and left the browser refusing later connections.
- **Restarting:** `restart.ps1` restarts that Chrome only, stopping the processes whose command line names
  `rex-live`. Use it after updating the extension.

## Recording tutorials and e2e runs (Ubuntu desktop in Docker)

`extension/recorder/` records the extension on real pages as a video and checks the results (decisions.md
U40). The desktop runs in a container with Xvfb, openbox and **Google Chrome stable**, the browser users
run, not Chrome for Testing.

- **Install, as an organization deploys it:** branded Chrome ignores `--load-extension`, so at start the
  container:
  - packs `extension/dist` into a CRX with Chrome's own packer, signed with `~/.config/rex/ext-e2e-key.pem`
    (not in the repo);
  - serves the CRX and an `updates.xml` on 127.0.0.1:8099;
  - writes the `ExtensionSettings` policy: `force_installed` from that update URL, `toolbar_pin:
    force_pinned`.
  The extension id follows from the key (`gejionoepeoggkhlglolgefomjfobppm`).
- **DevTools on the extension:** `DeveloperToolsAvailability: 1` is required; the default refuses DevTools
  on policy-installed extensions, and CDP says "Not allowed".
  - Other policies turn off sign-in, sync, promos, translate and the password manager.
- **Either build:** the runner works with the store build (`pnpm build`, what users get) or the e2e build
  (`pnpm build:e2e`).
  - **Store build:** driven through the UI only. The runner waits for the extension's own first sync and
    reads the result from the popup (status badge, `Recipe <id>@<v>`, one row per field). It has no
    `<all_urls>`, so the real toolbar click is what grants `activeTab`; that path is now tested.
  - **e2e build:** the runner also checks that the extraction behind the popup matches what the popup shows.
- **Real input:** each scene is the user's flow, done with `xdotool`:
  1. type the URL into the address bar;
  2. scroll the page;
  3. move the pointer to the rex icon (found on a screenshot by template matching) and click it;
  4. scroll the popup.
- **CDP observes only:** it reads the load state, whether a bundle is stored, and the popup.
- **Capture:** `ffmpeg x11grab` records the whole screen at 50 fps, one frame every 20 ms, including the
  toolbar, the popup and the pointer. Captions come from the logged events (ASS, burned in). An intro card,
  the scenes and a results card are joined into one MP4.
- **Checks:** each scene has an `expect` (recipe id prefix, status, optional minimum field count).
  `results.json` lists the outcome, and the exit code is 1 if a scene fails.
- **Fresh Chrome per scene:** Chrome restarts (SIGTERM, same profile) between scenes.
- **After rebuilding the extension,** recreate the container (`--force-recreate`). The CRX is packed at start,
  and the version in the manifest does not change between builds.

```bash
export TAILSCALE_IP=$(tailscale ip -4)
pnpm -C extension build                          # or build:e2e; the container packs extension/dist
docker compose -f extension/recorder/docker/compose.yaml up -d --build --force-recreate
docker compose -f extension/recorder/docker/compose.yaml exec desktop \
  node /repo/extension/recorder/run.mjs /repo/extension/recorder/scenarios/tutorial.json --out /out/tutorial
# --only wikipedia,hn   --no-video (checks only)
# output: extension/recorder/docker/out/tutorial/{rex-tutorial.mp4,results.json,<scene>.raw.mp4,...}
```

### Hermetic docker e2e (CI-safe)

`scripts/docker-e2e.sh` runs the same flow with no network: real input, Google Chrome, and the store build
installed from a CRX. Every scene must match its fixture's `expected.json`: the exact recipe and version,
the status, and the number of top-level fields.

1. It makes a throwaway Ed25519 bundle key and a config whose only source is
   `https://bundles.rex.test/v1/dev/`.
2. It builds the store extension with that config into `extension/recorder/docker/out/hermetic/ext`
   (`REX_DIST`; `extension/dist` is untouched).
3. It publishes `recipes/` with `rex bundle publish`.
4. It runs the `e2e` compose service:
   - `network_mode: none`, so the container has loopback only;
   - the policy-installed CRX;
   - `--host-resolver-rules="MAP * 127.0.0.1"`;
   - `fixture-server.mjs`, which serves each fixture page at its real URL plus the bundle, over TLS from
     a CA made at start and trusted in the profile's NSS store.

The scenarios (`extension/recorder/scenarios/`, resolved by `lib/scenario.mjs`):

- **`fixtures.json`:** the 7 tutorial scenes, with the full flow and a video.
- **`sites.json`:** one scene per site recipe. `{"siteFixtures": "recipes/sites"}` expands to the first
  captured (not synthetic) fixture of each recipe, so a new site recipe is covered without editing the
  scenario. It uses the quick flow (type the URL, click the icon, read the popup, close it; about 7.6 s a
  scene) and is checks only. The raw clips of passing scenes are deleted; failing ones are kept.
- **One page per URL:** two different fixtures at one URL are an error, since they would test each other's
  page.

```bash
scripts/docker-e2e.sh                 # fixtures.json + sites.json (59 scenes, about 13 min)
REX_E2E_SCENARIOS=sites.json scripts/docker-e2e.sh --only zhihu.com_question
scripts/docker-e2e.sh --no-video      # no video for any scenario
REX_DOCKER_E2E=1 scripts/ci-local.sh  # as the last ci-local step
# output: extension/recorder/docker/out/hermetic/run/<scenario name>/{results.json,...}
#         (directly in run/ when only one scenario runs)
```

- **Signing key:** the CRX key is `REX_EXT_KEY`, else `~/.config/rex/ext-e2e-key.pem`, else a throwaway key
  made by the script, so a CI runner needs nothing set up.
- **Fixtures repo in CI:** `Shuozeli/rex-fixtures` is private. `ci.yml` and `publish.yml` check it out with
  the secret `REX_FIXTURES_SSH_KEY`, the private half of a read-only deploy key on that repo ("rex CI
  (read-only)"). To rotate it: make a new ed25519 key, `gh repo deploy-key add` it to rex-fixtures,
  `gh secret set REX_FIXTURES_SSH_KEY -R Shuozeli/rex`, then delete the old deploy key.
- **CI:** the `docker-e2e` job in `ci.yml` runs after `rust` and `wasm`, using their `rex` and wasm artifacts.
  It is a matrix over the scenarios (`fixtures`, `sites`), which run in parallel.
  - It builds the image with buildx and a GitHub Actions layer cache. `CHROME_REFRESH` is the ISO week, so
    the cached Chrome layer is refreshed weekly.
  - It then runs `scripts/docker-e2e.sh` with `REX_E2E_SKIP_BUILD=1`.
  - Artifacts: `docker-e2e-<scenario>` (results, events, and for fixtures the video, about 8 MB), plus
    `docker-e2e-<scenario>-raw-clips` when it fails.
  - Run dir permissions: the container is uid 1000 and the runner is not, so the script opens `run/` to all
    and makes the throwaway key world-readable.
- **Command mode:** with arguments, the container runs them once Chrome answers and exits with their status.
- **Logs:** Chrome's log goes to `/tmp/chrome.log` in the container. In hermetic runs it is full of expected
  TLS errors for Google's own hosts.
- **Infobar:** Chrome shows an "unsupported command-line flag" bar for `--host-resolver-rules`. It is the
  only extra flag, and only in this mode.

- **Watch live:** noVNC at `http://shuoze25-yuacx.tail8f3b66.ts.net:6081/vnc.html`.
- **CDP:** `http://shuoze25-yuacx.tail8f3b66.ts.net:9541`. A small relay rewrites the Host header, which Chrome
  only accepts as an IP or localhost. Both bind to the Tailscale IP only.
- **Network:** host networking, so the extension reaches the serve-lib bundle by MagicDNS. Pages see this
  host's IP, and Amazon shows it a "Continue shopping" interstitial.
- **Proxy:** `REX_RECORDER_CHROME_ARGS="--proxy-server=socks5://<host>:<port>"` passes extra Chrome flags.

## Drift

- **Liveness** (`.github/workflows/liveness.yml`, daily) runs `rex liveness` over `liveness.yaml` and keeps
  one open issue labelled `liveness`. To triage:
  1. run `rex liveness` locally;
  2. capture the page (`rex fixture capture`), then `rex explain` it;
  3. fix the selectors, bump the recipe `version`, and add the capture as a redacted fixture.
- **User reports** arrive as "Broken recipe: …" issues from the popup. The page URL is included only when
  the user opted in.
