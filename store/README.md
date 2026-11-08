# Chrome Web Store: publishing rex

Everything needed to publish "rex: Page to JSON" in the Chrome Web Store: the package, the listing text in
English and Chinese, the images, and the answers for the privacy tab, ordered as the
[developer dashboard](https://chrome.google.com/webstore/devconsole) asks for them. Decisions: decisions.md U45
(no network) and U46 (this listing).

Requirements are from Google's documentation, checked 2026-11-07:
[images](https://developer.chrome.com/docs/webstore/images),
[store listing](https://developer.chrome.com/docs/webstore/cws-dashboard-listing),
[privacy tab](https://developer.chrome.com/docs/webstore/cws-dashboard-privacy),
[user data FAQ](https://developer.chrome.com/docs/webstore/program-policies/user-data-faq).

## Checklist

| # | Step | Material |
|---|---|---|
| 1 | Register a developer account (one-time registration fee; accept the developer agreement) | You |
| 2 | Build the package: `scripts/store-package.sh` | `target/store/rex-extension-<version>.zip` |
| 3 | Dashboard → **New item** → upload the ZIP | The ZIP |
| 4 | **Store listing** tab: description, category, language, images, URLs | [Store listing](#store-listing) |
| 5 | **Privacy** tab: single purpose, permissions, remote code, data use, privacy policy URL | [Privacy](#privacy) |
| 6 | **Distribution** tab: free, public, all regions | [Distribution](#distribution) |
| 7 | **Submit for review** | |

The repository must be public before step 5: the privacy policy URL points to `PRIVACY.md` on GitHub.

## Package

```bash
scripts/store-package.sh      # -> target/store/rex-extension-0.3.0.zip (and its sha256)
```

- **Config:** it builds with the committed `extension/config.example.json`, never a machine's `config.local.json`.
- **Checks:** the build fails if the manifest asks for host access, if CSP allows connections outside the
  package, if the test hooks are present, or if the recipes are missing.
- **Versions:** every upload needs a higher `version` than the last one. Bump `extension/package.json`.

What the manifest declares:

| Field | Value |
|---|---|
| Name (≤ 75 chars) | `rex: Page to JSON` / `rex：网页转 JSON` (from `_locales`) |
| Description (≤ 132 chars; the store's summary) | en: "Turn the page you are reading into one typed JSON record, with declarative recipes built into the extension. Works offline." (123) · zh_CN: "用扩展内置的声明式配方，把正在阅读的网页变成一条有类型的 JSON 记录。不联网。" (41) |
| Permissions | `activeTab`, `scripting`, `storage`, `declarativeContent`; no host permissions |
| CSP | `connect-src 'self'`; `wasm-unsafe-eval` for the packaged WebAssembly engine |

## Store listing

**Default language:** English. Add **Chinese (Simplified)** as a second language; its name and summary come from
`_locales/zh_CN`, and the description and screenshots below go into its tab.

**Category:** Developer Tools.

**Detailed description:** [`listing/en.md`](listing/en.md) and [`listing/zh_CN.md`](listing/zh_CN.md). Paste the text
under "Description" into the field.

**Graphic assets** (all in `store/assets/`):

| Field | File | Spec |
|---|---|---|
| Store icon | `icon-128.png` | 128×128 PNG, 96×96 artwork with 16 px transparent padding |
| Screenshots (en) | `en/screenshot-1.png` … `en/screenshot-5.png` | 1280×800 PNG without alpha, full bleed, up to 5 |
| Screenshots (zh_CN) | `zh_CN/screenshot-1.png` … `zh_CN/screenshot-5.png` | the same, Chinese UI |
| Small promo tile | `promo-small-440x280.png` | 440×280, no text (tiles are not localized) |
| Marquee promo tile (optional) | `promo-marquee-1400x560.png` | 1400×560, no text |
| YouTube video (optional) | none | |

The screenshots, in order:

| # | en | zh_CN |
|---|---|---|
| 1 | Wikipedia article, Reader view | Chinese Wikipedia article, Reader view |
| 2 | Hacker News front page (a list) | Ruan Yifeng's weekly (a blog post) |
| 3 | GitHub Trending, Table view | GitHub Trending, Table view |
| 4 | Yahoo Finance quote (AAPL) | Hacker News front page (a list) |
| 5 | Wikipedia article, the JSON panel | Chinese Wikipedia article, the JSON panel |

**Additional fields:**

| Field | Value |
|---|---|
| Homepage URL | `https://github.com/Shuozeli/rex` |
| Support URL | `https://github.com/Shuozeli/rex/issues` |
| Official URL | leave empty (it needs a site verified in Search Console) |
| Mature content | off |

## Privacy

**Single purpose:**

> rex turns the web page the user is viewing into one structured JSON record (for example an article's title,
> authors, dates and text, or a product's price), when the user clicks its toolbar icon, so the user can copy or
> download it.

**Permission justifications:**

| Permission | Justification |
|---|---|
| `activeTab` | When the user clicks the rex icon, rex reads that one tab to extract its record. It has no other access to any website. |
| `scripting` | Runs a small packaged function in the clicked tab that returns the page's HTML to the extension, where the record is extracted. No code is fetched or injected from elsewhere. |
| `storage` | Saves the user's settings (copy format, developer mode, a recipe pasted in developer mode) and the last popup view, on the user's computer. |
| `declarativeContent` | Marks the toolbar icon with a green dot on sites that have a site recipe, using rules generated from the built-in recipes. It reads nothing from the page. |

**Host permissions:** none requested.

**Remote code:** *No, I am not using remote code.* The WebAssembly engine and the recipes are files inside the
package. CSP allows `wasm-unsafe-eval` only to instantiate that packaged module.

**Data usage:** tick **Website content**, and no other category. The Chrome Web Store counts reading a page as
handling user data even when nothing leaves the device (user data FAQ). Then tick all three certifications:

- I do not sell or transfer user data to third parties, outside of the approved use cases.
- I do not use or transfer user data for purposes that are unrelated to my item's single purpose.
- I do not use or transfer user data to determine creditworthiness or for lending purposes.

**Privacy policy URL:** `https://github.com/Shuozeli/rex/blob/main/PRIVACY.md` (English and Chinese). The
dashboard's account page has a privacy policy field as well; use the same URL there.

## Distribution

| Field | Value |
|---|---|
| Payment | Free |
| Visibility | Public |
| Regions | All regions |

## Updating

1. Bump `version` in `extension/package.json`. Recipes ship with the extension (U45), so a recipe fix also needs
   a new version.
2. Run `scripts/store-package.sh`.
3. Upload the ZIP in the dashboard's **Package** tab and submit it for review.

Regenerating the images:

- **Screenshots:** `extension/recorder/store.mjs`, in the recorder desktop at 1280×800; the commands are in its
  header and in docs/operations.md.
- **Icon and promo tiles:** `uv run --with pillow tools/icons/make_icons.py`.
