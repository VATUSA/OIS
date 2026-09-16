# @ois/ui

The shared component set for the OIS web app (and later the desktop app). Every component reads the
Console Design Language tokens in `src/styles/globals.css` — see `/DESIGN.md` for the rules. If a
screen needs something this package doesn't have, add it here rather than hand-styling it in a page.

## What to use when

| Need | Use | Not |
| --- | --- | --- |
| A page title, count, subtitle, view switch | Route `staticData` meta (the shell renders `PageHeader`) | A hand-rolled `<h1>` |
| A status, lifecycle or domain state | `StatusPill tone=…` (tones from `web/src/lib/status.ts` `toneOf`) | A Badge with ad-hoc colour classes |
| A small either/or choice or a view switch | `SegmentedControl` | Button `default`/`secondary` toggles |
| Switching between sections of one page | `Tabs` | Underline tabs or bordered button groups |
| Narrowing a list | `FilterBar` + `FilterChip` (+ `AddFilter`) | A row of raw `<select>`s |
| A picker from a short fixed list | `Select` (styled native) | A raw `<select>` |
| A headline number | `MetricCard` (+ `Sparkline` in `sparkline`) | A local `Stat`/`Tile` |
| Loading / error / empty | `QueryState` around the data, or `EmptyState` alone | A "Loading…" `<p>` |
| Arbitrary content in an overlay | `Modal` (`placement` center / top / right drawer) | A `fixed inset-0` div |
| A yes/no or one-field question | `useConfirm()` / `usePrompt()` | `window.confirm` |
| A side column that becomes a bottom sheet on phones | `Sheet` | A custom drawer |
| Search-and-jump over grouped results | `CommandPalette` (controlled; caller ranks results) | A custom overlay list |
| Any tabular data | `DataTable` | A raw `<table>` |
| A chart | `Sparkline` · `TimeSeries` · `Bars` · `StackedBars` · `Donut` | Div bars or hand-drawn SVG |
| A colour in JS (deck.gl, chart attrs) | `useTokens` / `useTokenRgba` | A hex constant |
| A per-device UI preference | `useLocalStorage` | Raw `localStorage` calls |
| Phone-width branching | `useIsMobile` | A local `matchMedia` |

Primitives (`Button`, `Card`, `Input`, `Switch`, `DropdownMenu*`, `Tooltip*`, `Avatar*`, `Badge`) keep
their shadcn shapes, restyled to the tokens.

## Tests

`pnpm --filter @ois/ui test` (vitest) — logic only (token parsing, table paging).
