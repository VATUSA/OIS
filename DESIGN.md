# DESIGN.md — the OIS interface language

How to design and build UI for OIS so the whole product reads as **one system** — across the web app
today and the Tauri desktop app later — instead of a jumble of one-off screens.

This is the **concrete, app-specific** design language. Its underlying *principles* come from the
Apple-inspired design skill at `.claude/skills/claude-apple-design-system/` (read it for the "why"
behind a rule). Where the two ever disagree, **this file wins for OIS** — it is the adapted, shipping
system; the skill is the foundation.

> **Living visual reference:** an annotated teardown of every element below (shell, sidebar, cards,
> table, tokens) is published as a Claude artifact — ask the design owner for the link. When in doubt
> about how something should *look*, that reference is the source of truth alongside this file.

---

## The look in one paragraph

A quiet, dark **operator console**. One rounded panel holds a collapsible sidebar and the main area,
joined at the top so content reads as a child of the navigation. Depth comes from stacked dark
surfaces and 1px hairlines — never shadows or gradients. Type is a strict 400/600/700 ladder; numbers,
IDs and money are monospace. Colour is near-monochrome with **one** accent (a VATUSA-aligned pastel
blue) and a small set of **semantic** status colours. It's dense where data lives and airy in the
headers. It should feel calm, precise, and obviously the same product on every screen.

---

## The 9 non-negotiables

1. **One accent, ever.** Every interactive/brand signal — active nav, primary button, focus ring,
   links, selected tab — is the accent blue `#6ea8fe`. There is no second brand colour. Semantic
   colours (success/danger/warning) exist only for **status**, never for decoration.
2. **No gradients, anywhere.** Flat fills only — solid surfaces, a solid accent, flat translucent
   sparkline fills. Gradients read cheap; depth comes from surface tints, not blends.
3. **No shadows.** Cards, buttons, chips, popovers get **no** drop shadow, and neither does the shell
   — the one soft drop under the frame went with the frame itself (#402). Elevation is a
   surface-colour step (ground → panel → card) plus a hairline, and nothing else.
4. **Hairlines, not borders.** Every separator is 1px in `--line` (`#26262d`). No 2px+ borders except a
   focused input or a selected row.
5. **Continuous, generous corners.** Rounded everything, from a scale (below). Use CSS
   `border-radius` with large radii on the shell and content panel; never invent in-between values.
6. **Weight ladder = 400 / 600 / 700. 500 is banned.** Body 400, labels/emphasis/active 600, titles
   700. A stray medium weight muddies the cadence — audit for it.
7. **Monospace for data.** IDs, money, counts, dates-as-data, and design tokens are JetBrains Mono
   with `font-variant-numeric: tabular-nums` so columns line up. UI prose is Inter.
8. **Air in headers, density in tables.** Page headers and margins breathe; data tables are
   deliberately dense. Both are correct — don't pad a 500-row table like a marketing page.
9. **Tokens only — never inline a value.** Every colour, radius, space, and font size comes from a
   token. A hardcoded hex or px in a component is a bug; it's what makes a system drift. The
   exceptions are a page that physically cannot load the stylesheet — see § "Standalone pages outside
   the app", which lists every one and the conditions they must meet — and a colour a **user** chose
   and saved, which is data rather than chrome (§ "User-chosen domain colour").

---

## Tokens

These are implemented as CSS custom properties in **`packages/ui/src/styles/globals.css`** — the
single source of truth, shared by the web app and (later) the Tauri desktop app. Dark lives under
`.dark` (canonical), light under `:root`. They're wired into the shadcn/ui semantic vars and exposed as
Tailwind utilities: `bg-panel`, `bg-card`, `text-ink-2`, `border-line`, `text-brand`, `bg-success`,
`bg-warning-soft`, `rounded-lg`, etc. **Retune a token here — never restyle a component's colours.**

### Surfaces & ink (dark-first — the canonical palette)

| Token | Hex | Use |
| --- | --- | --- |
| `--ground` | `#08080a` | Page/app background, the shell base behind panels |
| `--panel` | `#0f0f12` | Sidebar and main panel |
| `--panel-2` | `#131317` | Slightly raised — top bars, active nav tile, segmented control |
| `--card` | `#16161b` | Cards, table header row, raised surfaces |
| `--chip` | `#1c1c22` | Chip/pill fills, code inlays |
| `--line` | `#26262d` | Hairline separators, borders |
| `--line-soft` | `#1c1c22` | Softer inner separators (table rows) |
| `--ink` | `#f3f3f5` | Primary text |
| `--ink-2` | `#a1a1aa` | Muted labels, descriptions |
| `--ink-3` | `#6b6b74` | Faint text, placeholder, inactive icon |

### Accent (one) & semantic status

| Token | Hex | Use |
| --- | --- | --- |
| `--brand` | `#6ea8fe` | The single accent — pastel blue, VATUSA-aligned. Active state, primary button, focus, links. (shadcn `--primary`.) |
| `--brand-ink` | `#a9cbff` | Brand as *text* on a dark ground |
| `--brand-soft` | `rgba(110,168,254,.16)` | Brand tint fills — soft chips, hover/selected wash. (shadcn `--accent`.) |
| `--success` / `--success-soft` | `#43d089` / `rgba(67,208,137,.13)` | Success / Active / positive trend |
| `--danger` / `--danger-soft` | `#fb6b6b` / `rgba(251,107,107,.13)` | Danger / Inactive / negative trend |
| `--warning` / `--warning-soft` | `#efc14d` / `rgba(239,193,77,.13)` | Warning / watch / flat trend |

VATUSA is red/white/blue: **blue** is the accent, **white** is the ink, **red** is the danger
semantic. Keep the accent and the status colours separate — e.g. a "Pending" state should use a
neutral or the accent *deliberately*, not accidentally collide with brand.

### Domain tokens (aviation meaning)

Some colours carry operational meaning that three status hues can't hold. They are tokens too —
defined for both themes in `globals.css`, exposed as utilities (`bg-flight-airborne`,
`text-cat-ifr`, `fill-series-3`, …), and read from JS with `useTokens` / `useTokenRgba`
(`@ois/ui`) where `var()` can't reach (deck.gl, chart attributes). Never re-declare them as hex.

| Group | Tokens | Use |
| --- | --- | --- |
| Flight state | `--flight-airborne` · `--flight-ground` · `--flight-proposed` · `--flight-arrived` | Airport, AADC, FCA, ladder |
| Flight category | `--cat-vfr` · `--cat-mvfr` · `--cat-ifr` · `--cat-lifr` | Weather category chips |
| Load level | `--level-ok` · `--level-watch` · `--level-over` (alias success/warning/danger) | Runway bins, GDP, delays |
| Series | `--series-1` … `--series-8` | Categorical chart series (carriers, fixes, gates, user charts) |
| Map | `--map-aircraft` · `--map-highlight` · `--map-route` · `--map-boundary` · `--map-label` · `--map-label-bg` · `--map-waypoint-bg` · `--map-apron` · `--map-taxiway` · `--map-runway` | deck.gl / MapLibre layers |
| ATC position | `--atc-del` · `--atc-gnd` · `--atc-twr` · `--atc-app` · `--atc-ctr` · `--atc-atis` | Controller badges and areas |

Domain tokens are for data, never for chrome: a button is never `--cat-ifr`.

### Radius, spacing, type

```
--r-xs: 6px    chips, inputs, small tiles
--r-sm: 8px    nav items, segmented control
--r-md: 12px   cards
--r-lg: 16px   panels
--r-xl: 20px   the outer shell
--pill: 999px  buttons, filter chips, status pills, search

spacing: 4-base → 4 · 8 · 12 · 16 · 20 · 24 · 32 · 40

type (Inter): 28/700/-.02em title · 20/700 section · 15/400 body · 14/600 label · 12/600 caption
data (JetBrains Mono): 12–13, tabular-nums
```

The **content region below the top bar** carries a rounded top-left corner — where the breadcrumb
(horizontal) divider curves into the sidebar (vertical) divider, near the page title. It's a signature
*inner* curve; the outer shell/sidebar join stays flush. Build it by putting the vertical divider on the
content's left edge and rounding only that inner corner — not by rounding the whole main panel.

---

## The shell (build this first)

The single highest-leverage primitive. Full height, **flush to the viewport on all four sides** — the
outer edge of the app is the window's bounding box, with no gutter, no CSS rounding of its own and no
shadow (#402). On the desktop app the native title bar is hidden and the window's buttons sit at the
top-left of the sidebar's chrome row, so a gutter here would read as a second bar beneath it.

**The window's own corners are rounded, and that is the OS's doing rather than the shell's** (#419):
macOS rounds a decorated window itself, Windows 11 is asked to through DWM, and the compositor clips
the webview — so nothing in the page carries a `border-radius` for it. "Flush, no outer rounding" is a
rule about *this stylesheet*; it does not mean the app is a hard-cornered rectangle on screen. It holds:

- **Sidebar** (`--panel`, collapsible): a chrome row (back · forward · recent pages / collapse), the
  identity (avatar + name + mono CID), a pill **⌘K** search, then **every section the user can use**
  — Home, Advisories, Operations, Planning, Historical, Admin, and a **User** group (Profile, Settings,
  API keys, Sign out) — and Docs / theme / Zulu clock at the foot. No dropdown menus for navigation. It is the only navigation: there is no top bar.
- **Main**: a breadcrumb row (muted parents, bright current crumb with its icon), then the page's
  **content panel** — inset and rounded on all four corners, with no divider between it and the
  sidebar. Inside: the page header (700 title + count chip + subtitle, with a segmented view switch)
  and the content. Don't repeat the page's name in section headings.

Every signed-in page renders inside this shell. The only pages outside it are the signed-out
homepage — which keeps the public landing and the site footer (the footer appears nowhere else) —
and the standalone pages below.

### Standalone pages outside the app

A page served by something other than the web app cannot reach `packages/ui`'s stylesheet, the
shell, or the self-hosted fonts. Such a page is allowed to inline a copy of the tokens it needs, as
a **documented exception to non-negotiable #9**, under all of these conditions:

- It is genuinely unable to load the stylesheet — not merely inconvenient. The test is whether the
  page would still have to work with **no network at all**.
- The inlined values carry a comment naming `packages/ui/src/styles/globals.css` as the source of
  truth, so the next reader knows the copy can go stale and that the stylesheet wins.
- Everything else in the system still applies: one accent, no gradients, no shadows (elevation is a
  surface step plus a hairline), 1px `--line` hairlines, the 400/600/700 ladder with 500 banned,
  `--r-lg` on panels, and a solid `--brand` pill with dark ink for a primary button.
- It is listed here.

**The list:**

- **The desktop sign-in callback tab** (`desktop/src-tauri/src/auth.rs`, `callback_page`). Served by
  the Tauri app's own loopback listener on `127.0.0.1:8765` as a single self-contained string, in
  four states (signed in, waiting, timed out, superseded). Deliberately **dark-only**, with no
  `prefers-color-scheme` branch: the app window it hands back to is unconditionally dark
  (`tauri.conf.json`'s `backgroundColor: "#08080a"`, which is `--ground`), so honouring a light OS
  preference would flash a light page on the way into a dark app. Its colours are held to
  `globals.css` by `the_inlined_tokens_match_the_stylesheet`, which reads the stylesheet itself — the
  only thing standing between the copy and silent drift. `--r-lg` comes from the token table above,
  which `globals.css` does not define, and the font stacks are system-fallback approximations.

### User-chosen domain colour

An FCA, a route and a facility-map rule each carry a colour a person picked so that theirs reads apart
on a shared map. That colour is **data**, not chrome, so it may be any `#rrggbb` — on these conditions
(#698):

- **Offered from tokens first.** The picker is `ColorSwatches` (`@ois/ui`), fed the named palette in
  `web/src/lib/palette.ts` (`--series-*` and `--ink-3`, each with its name). A custom pick is the
  fallback, not the default.
- **Validated on the server**, on every write path: lowercase `#rrggbb` only.
- **Never invisible.** At least 3:1 contrast against the dark ground (`#08080a`). Every token swatch
  clears it in both themes; black and near-black don't. `ColorSwatches` refuses a darker pick, and the
  server refuses it too.
- **One parser.** The map draws it through `hexToRgb`, and a list chip uses `swatchCss`, which is the
  same parse, so a value can't look right in a list and grey on the map.
- **A `style` colour is allowed only for such data** (a chip, a swatch), never for chrome.
- **It is stored as a hex, so it drifts with theme.** A swatch picked in light mode stores the light
  hex and shows that in dark mode too. That is known and accepted; storing a token name would fix it,
  at the cost of an API change.

## Components (one each, tokens only)

- **Nav item** — icon + label + right-aligned mono count. Active = `--panel-2` tile, icon shifts to
  `--brand-ink`. Nested children indent behind a 1px vertical guide.
- **Status pill** — `--pill`, a semantic soft-tint fill with same-hue text. Optional leading dot for
  people-status (Active/Inactive). Never for actions.
- **Filter chip** — pill, leading icon + chevron, `--panel-2` fill, hairline. Plus a ghost "+ Add
  filter". Pills signal "interactive".
- **Metric card** — `--card`, title + `--max` expand affordance, a big 700 tabular number, a semantic
  trend row, and a sparkline with a **flat** translucent area fill + emphasized endpoint dot in the
  same semantic hue.
- **Data table** — `DataTable` in `@ois/ui`: an opt-in select column, icon-led headers on
  `--card`, `--line-soft` row separators, tabular figures, state as a pill, identity as avatar +
  name(600) + email link. Every table shows a default row cap with a "Show all" expand, then
  paginates — except live operational lists (departures, GDP flights, IDST, runway arrivals),
  which page from the first row (`rowCap={Infinity}`) so no flight is hidden behind an expand.
- **Sector grid** — `SectorGrid` in `@ois/ui`: one row per sector, one column per 15-minute bin on a
  rolling Zulu time axis (#724). **Not a `DataTable`, deliberately:** a `DataTable`'s columns are an
  entity's attributes (select boxes, pills, avatars); these are up to 24 load-carrying cells over time,
  which `DataTable`'s header, row cap and paging would only get in the way of. Its rules:
  sector and limit columns are sticky while the time axis scrolls inside the grid (never the page);
  each cell's figure is **`--ink`**, mono tabular, on a `--level-ok/watch/over` tint with a solid bar
  in the same token — the colour is a non-text signal (≥ 3:1 on each theme's ground, tested), because
  the light theme's green is too faint for small coloured text; hover carries both figures, combined
  peak and airborne alone; the footer labels each bin's start in `HHMM` Zulu; the limit cell is
  editable only where the viewer may edit, and is plain text with no affordance otherwise; a combined
  row lists the sectors it carries under its id, in `--ink-3`, truncated inside the sticky column.
- **Buttons** — primary = solid `--brand` **pill** with **dark ink** (`--primary-foreground`; white
  fails AA on a pastel accent). Secondary = `--panel-2` pill with a hairline. Press = scale 0.97,
  150–220ms ease-out, no bounce.
- **Icons** — one-weight line icons (~1.7px stroke, round caps), `currentColor` so they inherit the
  row's ink/accent. Never give an icon its own colour.
  **One named exception (#680):** the sidebar's *What's new* Sparkles icon turns gold (`--warning`)
  and twinkles (`animate-sparkle`, flat, no gradient; still for reduced motion) on hover. It is
  not precedent — no other icon takes its own colour, and gold is not a second accent anywhere else.
- **Screenshots** — framed like a card: `--card` mat, 1px `--line` hairline, `--r-md`, no shadow and
  no gradient scrim. Thumbnails share one box, `aspect-shot` (16:10), filled top-anchored
  (`object-cover object-top`) because a UI shot carries its signal at the top. Every thumbnail opens
  full-size in a nested `Modal`; at three columns nothing in an OIS screenshot is legible. They're
  bundled, imported from `web/src/assets/changelog/` so they're served from our own origin (the
  desktop CSP allows no remote image, #429). Budget: 8 per changelog entry, only the newest 3
  entries carry any, 300 KB per file and 1.5 MB in total (both enforced by tests, #665).

---

## Charts

One approach: `@tanstack/charts`, wrapped by the chart components in `@ois/ui` (`Sparkline`,
`TimeSeries`, `Bars`, `StackedBars`, `Donut`) that read their colours from tokens.

- **Flat fills only.** Areas are a flat translucent fill of the series hue; never a gradient.
- **Semantic colour.** Status-bearing data uses status or domain tokens (a load level is
  `--level-*`, a flight state `--flight-*`); categorical series use `--series-1…8` in order.
- **Faint structure.** Grid lines `--line-soft`, axes and tick labels `--ink-3` in mono 11–12px.
- **Emphasize the endpoint.** Sparklines end in a dot in the line's hue; thresholds (an AAR cap) are
  a dashed 1px rule in `--warning`.
- **Both themes.** Every colour comes through tokens, so a chart re-reads on theme switch.

---

## Review checklist (web)

Report each miss as `file:line — rule → fix`.

- [ ] No hex, `rgb()`, or Tailwind palette class (`emerald-500`, `zinc-…`) outside `globals.css` — a saved user colour excepted (§ "User-chosen domain colour").
- [ ] No gradient and no `shadow-*` at all — including on the shell.
- [ ] No `font-medium` (500). Weights are 400 / 600 / 700.
- [ ] Radii from the scale (`rounded-xs…xl`, `rounded-full` for pills); no arbitrary radius.
- [ ] Separators are 1px `border-line` / `border-line-soft`.
- [ ] One accent: interactive state uses `primary`/`brand`; status colours only mean status.
- [ ] IDs, counts, times and money render `font-mono` (tabular).
- [ ] The page renders inside the shell; its title comes from route meta, not a hand-rolled `<h1>`.
- [ ] Tables use `DataTable` (a sector-by-time matrix uses `SectorGrid`); charts use the `@ois/ui` chart components; overlays use `Dialog`/`Sheet`.
- [ ] Links and nav items the user can't use are not rendered.
- [ ] Legible in both dark and light.
- [ ] A page listed under § "Standalone pages outside the app" is exempt from the hex, shell and
      dark/light rows above — check it against the conditions there instead, not against this list.

---

## Theming

Dark-first is canonical. Because every component reads from tokens, a light theme (if we add one) is a
**token-swap only** — redefine the surface/ink tokens under `:root[data-theme="light"]`, keep accent
and semantics working on the new ground, change **no component code**. Build against the tokens now and
light mode stays cheap later.

The one thing a token swap can't reach is a **screenshot**: it's pixels, so a dark-UI shot will sit
on a light ground. The card mat and hairline frame soften that. Light/dark variants of each shot
would double their weight and are deliberately not done (#665).

## Do / Don't

- **Do** reach for a surface step or a weight change before any chrome. **Don't** add a shadow or a
  second colour to create emphasis.
- **Do** keep one accent. **Don't** let status colours leak into non-status UI.
- **Do** use hairlines and continuous corners. **Don't** use heavy borders or gradients.
- **Do** put IDs/money/counts in mono with tabular-nums. **Don't** use a 500 weight.
- **Do** build the shell + shared components once and render everything through them. **Don't** hand-
  style a one-off screen.

---

*Foundation & rationale: `.claude/skills/claude-apple-design-system/`. When this file doesn't cover a
case, defer to that skill's `checklist.md` / `components.md`, then to the visual reference artifact.*
