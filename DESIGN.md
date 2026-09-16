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
3. **No chrome shadows.** Cards, buttons, chips, popovers get **no** drop shadow. Elevation is a
   surface-colour step (ground → panel → card) plus a hairline. The *only* allowed shadow is the one
   soft drop under the whole app shell so it floats on its ground.
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
   token. A hardcoded hex or px in a component is a bug; it's what makes a system drift.

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

The single highest-leverage primitive. One full-height rounded frame on `--ground` (a thin gutter to
the window, the system's only shadow beneath it) holds:

- **Sidebar** (`--panel`, collapsible): a chrome row (back · forward · recent pages / collapse), the
  identity (avatar + name + mono CID), a pill **⌘K** search, then **every section the user can use**
  — Home, Advisories, Operations, Planning, Historical, Admin, and a **User** group (Profile, Settings,
  API keys, Sign out) — and Docs / theme / Zulu clock at the foot. No dropdown menus for navigation. It is the only navigation: there is no top bar.
- **Main**: a breadcrumb row (muted parents, bright current crumb with its icon), then the page's
  **content panel** — inset and rounded on all four corners, with no divider between it and the
  sidebar. Inside: the page header (700 title + count chip + subtitle, with a segmented view switch)
  and the content. Don't repeat the page's name in section headings.

Every signed-in page renders inside this frame. The only page outside it is the signed-out homepage,
which keeps the public landing and the site footer (the footer appears nowhere else).

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
- **Buttons** — primary = solid `--brand` **pill** with **dark ink** (`--primary-foreground`; white
  fails AA on a pastel accent). Secondary = `--panel-2` pill with a hairline. Press = scale 0.97,
  150–220ms ease-out, no bounce.
- **Icons** — one-weight line icons (~1.7px stroke, round caps), `currentColor` so they inherit the
  row's ink/accent. Never give an icon its own colour.

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

- [ ] No hex, `rgb()`, or Tailwind palette class (`emerald-500`, `zinc-…`) outside `globals.css`.
- [ ] No gradient; no `shadow-*` except the one under the app shell.
- [ ] No `font-medium` (500). Weights are 400 / 600 / 700.
- [ ] Radii from the scale (`rounded-xs…xl`, `rounded-full` for pills); no arbitrary radius.
- [ ] Separators are 1px `border-line` / `border-line-soft`.
- [ ] One accent: interactive state uses `primary`/`brand`; status colours only mean status.
- [ ] IDs, counts, times and money render `font-mono` (tabular).
- [ ] The page renders inside the shell; its title comes from route meta, not a hand-rolled `<h1>`.
- [ ] Tables use `DataTable`; charts use the `@ois/ui` chart components; overlays use `Dialog`/`Sheet`.
- [ ] Links and nav items the user can't use are not rendered.
- [ ] Legible in both dark and light.

---

## Theming

Dark-first is canonical. Because every component reads from tokens, a light theme (if we add one) is a
**token-swap only** — redefine the surface/ink tokens under `:root[data-theme="light"]`, keep accent
and semantics working on the new ground, change **no component code**. Build against the tokens now and
light mode stays cheap later.

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
