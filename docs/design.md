# Design system

This page describes Ranch's design system as it is implemented. Every value here comes from `src/styles/globals.css` and the components in `src/components/`; when this page and the code disagree, the code is right and this page needs a fix.

## Contents

- [Principles](#principles)
- [Colour tokens](#colour-tokens)
- [Typography](#typography)
- [Spacing, sizes and radii](#spacing-sizes-and-radii)
- [Components](#components)
- [Rules every screen follows](#rules-every-screen-follows)
- [Logo](#logo)

## Principles

**Native first.** Ranch behaves like a desktop tool, not a web dashboard in a window. Every action sits in the native menu bar with its shortcut, ⌘K reaches everything, right-click menus are native, a job can open in its own window or tab, and confirmations are sheets attached to the top of the window. On macOS the window has no title bar: the sidebar holds the traffic lights. In a browser (Ranch server, Ranch web) the same command registry appears under a "Ranch" button in the sidebar header.

**Dark technical look, plus a light theme.** Dark graphite surfaces with one warm accent; a light theme with the same structure. The theme follows the system by default and can be set in View › Appearance. Both themes are tested for contrast.

**Readable data.** The system font for the interface, JetBrains Mono for anything a developer reads as data: counts, ids, keys, timestamps, payloads. Numbers use tabular figures so live counters do not jitter. Nothing renders under 11 px.

**Lists are rows, not cards.** Tables of rows with a sticky header carry most screens. A `Panel` (surface, border, radius) only groups content that belongs together, such as a chart or a short ranked list.

**Graphical where it helps.** Small hand-drawn SVG charts sit next to the numbers they explain: sparklines, heat strips, stacked bars, a trace of each job's attempts. Missing data is said ("metrics off"), never drawn as zeros.

**Orange is the human, violet is the agent.** The accent marks what the user does. The AI colour, with the ✦ glyph, marks what an agent did or asks for. The two never share a control. With AI features off, nothing violet and no ✦ appears anywhere.

## Colour tokens

Colours are CSS custom properties: light values in `:root`, dark values in `.dark` (put on `<html>` by `next-themes`). Use them through Tailwind's arbitrary-property syntax, for example `bg-(--surf)`, `text-(--text2)`, `border-(--line)`. There are no shadcn colour names (`--background`, `--primary`…); `src/styles/tokens.test.ts` fails if one comes back.

### Surfaces and text

| Token | Dark | Light | Use |
|---|---|---|---|
| `--bg` | `#141518` | `#ffffff` | Main content |
| `--side` | `#0f1013` | `#f3f4f6` | Sidebar |
| `--surf` | `#1a1b1f` | `#f6f7f9` | Cards, fields, info strip, toolbar buttons, table headers |
| `--surf2` | `#26272d` | `#e9ebef` | Selected segment, chips, hover |
| `--panel` | `#1d1e23` | `#ffffff` | Sheets, palette, menus |
| `--field` | `#121316` | `#f6f7f9` | Text inputs, code blocks |
| `--text` | `#e8e8ea` | `#17181c` | Primary text |
| `--text2` | `#9a9ca5` | `#5f636e` | Secondary text, column headers |
| `--dim` | `#5f626b` | `#a3a6ae` | Zero values only, never text that must be read |
| `--line` | `rgba(255,255,255,0.08)` | `rgba(0,0,0,0.1)` | Borders, separators |
| `--scrim` | `rgba(0,0,0,0.45)` | `rgba(20,22,28,0.25)` | Behind sheets and the palette |

### Accent, AI and danger

| Token | Dark | Light | Use |
|---|---|---|---|
| `--accent` | `#f28c38` | `#b9560b` | Primary buttons, active tab underline, focus ring, switches |
| `--accent-text` | `#1a1208` | `#ffffff` | Text on `--accent` |
| `--sel` | `rgba(242,140,56,0.14)` | `rgba(185,86,11,0.1)` | Selected row or sidebar item |
| `--link` | `#f5a462` | `#a34b09` | Links |
| `--ai` | `#b6a8ff` | `#5b45c9` | Anything from an agent (✦), Ask AI buttons |
| `--ai-text` | `#120d26` | `#ffffff` | Text on `--ai` |
| `--ai-bg` | `rgba(182,168,255,0.1)` | `rgba(91,69,201,0.07)` | Agent notes, approval cards |
| `--ai-line` | `rgba(182,168,255,0.35)` | `rgba(91,69,201,0.3)` | Their borders |
| `--danger` | `#d63a40` | `#c62f34` | Destructive buttons only |
| `--danger-text` | `#ffffff` | `#ffffff` | Text on `--danger` |

The focus ring is the accent at 50 % opacity (`focus-visible:ring-3 ring-(--accent)/50`); the base layer also sets every element's outline colour to it.

### Job states

Each state has a fill colour and a text colour. The `-text` variant is for state-coloured text on `--bg` and on the state's own tinted pill (the fill at 15 %).

| State | Dark fill | Dark text | Light fill | Light text |
|---|---|---|---|---|
| waiting | `#9b8cf0` | `#9b8cf0` | `#5b4fd0` | `#5b4fd0` |
| active | `#4cc6e6` | `#4cc6e6` | `#0b7fa1` | `#076583` |
| delayed | `#e5b544` | `#e5b544` | `#9a5b06` | `#7f4b05` |
| failed | `#ff6b5a` | `#ff8f80` | `#d6402f` | `#b8321f` |
| completed | `#3fbf8f` | `#3fbf8f` | `#1d7f46` | `#17703d` |

Tokens: `--state-waiting`, `--state-waiting-text`, and so on. The other BullMQ states borrow the closest one: prioritized looks like waiting, stalled like failed, waiting-children and paused like delayed, unknown is neutral (`--surf2` on `--text2`).

### Environments

Each environment has a colour from a curated palette of ten (`src/features/connections/palette.ts`). The colour appears in two places only: the environment dot (`EnvDot`) and the environment tag (`EnvTag`). The values are the same in both themes except where noted; each reaches 3:1 against `--bg` and `--side`, since dots are graphics, not text.

| Token | Value | Hue |
|---|---|---|
| `--env-production` | `oklch(0.577 0.245 27.325)` | red |
| `--env-staging` | `oklch(0.6 0.15 85)` | amber |
| `--env-local` | `oklch(0.6 0.18 150)` | green |
| `--env-custom-1` | `oklch(0.55 0.2 264)` | blue |
| `--env-custom-2` | `oklch(0.55 0.18 300)` | magenta |
| `--env-custom-3` | `oklch(0.6 0.15 200)` | teal |
| `--env-custom-4` | `oklch(0.55 0.18 35)` | orange |
| `--env-custom-5` | `oklch(0.55 0.18 330)` | pink |
| `--env-custom-6` | `oklch(0.5 0.1 250)`, dark `oklch(0.6 0.1 250)` | slate |
| `--env-custom-7` | `oklch(0.45 0.12 170)`, dark `oklch(0.6 0.12 170)` | dark teal |

To add a colour, define `--env-<token>` in `globals.css` and add it to `palette.ts`.

### Code and charts

| Token | Dark | Light | Use |
|---|---|---|---|
| `--syntax-key` | `oklch(0.78 0.18 270)` | `oklch(0.55 0.18 270)` | JSON keys |
| `--syntax-string` | `oklch(0.78 0.18 145)` | `oklch(0.55 0.18 145)` | Strings |
| `--syntax-literal` | `oklch(0.78 0.2 30)` | `oklch(0.6 0.2 30)` | Numbers, booleans, null |

`--chart-1` to `--chart-5` (a green ramp, the same in both themes) colour the Recharts charts of the queue metrics view (`src/components/ui/chart.tsx`). The small charts use the state and accent tokens instead.

## Typography

### Fonts

| Role | Stack |
|---|---|
| Interface (`font-sans`, `font-heading`) | `-apple-system, BlinkMacSystemFont, "Inter Variable", "Cantarell", sans-serif` |
| Data (`font-mono`) | `"JetBrains Mono Variable", ui-monospace, "SF Mono", monospace` |

The interface uses the system font: San Francisco on macOS, Inter (bundled) where there is none. JetBrains Mono is bundled too. Both come from `@fontsource-variable` packages: the app never fetches a font at runtime and works offline.

Use the mono face for ids (`#1211`), Redis keys, queue keys, counts, durations, timestamps, latency, host and port, payloads, stack traces and key hints. The `font-tabular` utility combines the mono face with tabular figures; use it for any number that updates live.

### Sizes

| Utility | Size / line height | Typical use |
|---|---|---|
| `text-2xs` | 11 / 16 px | Column headers and state pills (semibold, uppercase, `tracking-wide`), key hints. The floor: nothing smaller. |
| `text-xs` | 12 / 16 px | Secondary text, info strip, small buttons |
| `text-sm` | 13 / 20 px | Body, table cells, sidebar, buttons; section titles in bold |
| `text-base` | 14 / 22 px | Larger body text |
| `text-lg` | 18 / 26 px | Page titles in the toolbar, bold |
| `text-xl` | 20 / 28 px | Large titles |
| `text-2xl` | 24 px | Figures in stat tiles (mono, semibold) |

Weights: 400 for body, 500 for buttons and tabs, 600 for selected items, headers and figures, 700 for titles.

## Spacing, sizes and radii

Layout sizes are Tailwind spacing tokens, so they work with any sizing utility (`w-sidebar`, `h-toolbar`, `h-row-dense`, `size-control`…).

| Token | Value | Use |
|---|---|---|
| `sidebar` | 240 px (220–320) | Sidebar width, resizable |
| `inspector` | 380 px (320–440) | Job inspector width, resizable |
| `toolbar` | 54 px | Top bar of every screen |
| `info-strip` | 32 px | Facts line under the toolbar (connection views) |
| `view-tabs` | 40 px | View tabs, 2 px accent underline |
| `row-queue` | 28 px | Queue rows in the sidebar |
| `row-connection` | 30 px | Connection rows in the sidebar |
| `row-dense` | 32 px | Dense lists |
| `row` | 42 px | Table rows |
| `row-two-line` | 56 px | Two-line rows |
| `control-sm` | 28 px | Small buttons and inputs |
| `control` | 30 px | Default buttons and inputs |
| `control-lg` | 34 px | Buttons in sheets |

The window opens at 1280 × 800 and cannot be narrower than 1100 px. Below 1280 px the inspector opens over the list instead of beside it, so the list keeps its width.

| Radius token | Value | Use |
|---|---|---|
| `rounded-tag` | 5 px | Tags, state pills |
| `rounded-control` | 7 px | Buttons, inputs |
| `rounded-card` | 10 px | Panels, stat tiles |
| `rounded-window` | 12 px | Window-level surfaces |
| `rounded-sheet` | 14 px | Sheets (bottom corners) |

The shadcn radius scale (`rounded-sm` to `rounded-4xl`, from `--radius: 0.625rem`) is still defined for the primitives that use it.

Spacing inside components follows Tailwind's 4 px scale.

## Components

Primitives come from Base UI and Radix, styled with the tokens. Icons are Lucide (`lucide-react`). The button and the data components are tested with long labels (a long queue name, a long error message) to check that nothing overflows.

### Primitives (`src/components/ui/`)

| Component | Notes |
|---|---|
| `button.tsx` | The one button. Variants `default` (neutral surface), `primary` (accent, the one main action of a screen or sheet), `ai` (hands work to an agent), `danger` (destructive confirmations only), `ghost` (toolbar and row actions). Sizes `sm` 28, `default` 30, `lg` 34, plus square `icon-sm`, `icon`, `icon-lg`. Labels never wrap or shrink. An optional `shortcut` prop shows the key hint in mono after the label. |
| `sheet.tsx` | A sheet attached to the top of the window, like a macOS sheet: up to 34 rem wide, over `--scrim`, focus kept inside, Esc closes. Replaces centred dialogs; only the palette keeps one. |
| `input.tsx`, `textarea.tsx`, `label.tsx`, `radio-group.tsx`, `form.tsx` | Form controls on `--field`; `form.tsx` is the react-hook-form glue. |
| `switch.tsx` | 34 × 20 px (`default`) or 28 × 16 px (`sm`), accent when on. |
| `progress-bar.tsx` | A 6 px track with the value label outside it. Tones: `active` for a worker's progress, the accent for work the user started, `warning` for work that looks stuck. |
| `dropdown-menu.tsx`, `context-menu.tsx` | In-window menus for the browser runtimes. The desktop uses native menus. |
| `tooltip.tsx`, `popover.tsx`, `collapsible.tsx`, `resizable.tsx`, `skeleton.tsx` | Restyled primitives. |
| `sonner.tsx` | Toasts for in-app confirmations. Background events use native notifications. |
| `chart.tsx` | Recharts wrapper, used by the queue metrics view only. |

### Data components (`src/components/data/`)

| Component | Notes |
|---|---|
| `DataTable` | A virtualised table on a CSS grid: one column template shared by the sticky header and every row, a fixed row height, the focused row kept in view. Each column declares the narrowest table width at which it still shows: columns that do not fit are left out, never squeezed until they clip. Rows follow the grid with `DATA_TABLE_GRID` and `useColumnVisible`. |
| `Sparkline`, `HeatStrip`, `StackedBar`, `StatTile` (`charts.tsx`) | Hand-drawn SVG on the tokens. A sparkline is a line over time; a heat strip shows buckets as cells (darker is busier, the red part is the failed share, hatched when metrics are off); a stacked bar shows a mix; a stat tile is a label, a figure in mono, a sub-line and an optional trend. |
| `TraceBar` | A job's life on one time axis: one row per wait, attempt and backoff (hatched), with what is still to come dashed. Labels sit beside the track, never on it; a span too short to see still shows as a 2 px mark. |
| `StatePill` | A job state as a small uppercase pill, the same in lists, flows and the inspector. |
| `EnvDot`, `EnvTag` (`Env.tsx`) | The environment dot, with its name as accessible label, and the dot plus name as a tinted tag. |
| `Toolbar` | The 54 px top bar: where you are, what it is, actions on the right. A long title wraps and the bar grows. Empty space drags the window on macOS. |
| `InfoStrip` | A quiet 32 px line of facts in mono 12 under the toolbar: host, port, db, Redis version. |
| `ViewTabs`, `ViewTabsRoot`, `ViewPanel` | A screen's views: 40 px tabs with an accent underline and an optional count in mono. |
| `Segmented` | A pill group for filters and ranges; arrow keys move the choice. |
| `SectionHeader` | A section title with an optional mono count and actions. |
| `Panel`, `PanelHeader`, `PanelTitle`, `PanelBody` | Surface, border, radius, for grouped content only. |
| `ConfirmSheet` | The confirmation for destructive actions: a question as title, what is removed and what is kept with counts, Cancel focused by default, and a button that says how much it does ("Drain 400 jobs"). On a protected connection the user types the target's name before the action runs. |
| `CodeBlock`, `CodeToken` | Code and payloads in mono on `--field` with the `--syntax-*` colours. Long lines scroll inside the block. |
| `Kbd` | A key hint in mono, with the keys spelled out for screen readers ("⇧⌘U" → "Shift Command U"). |
| `EmptyState` | What is empty, why, and the one thing to do about it. |

### Shell and other shared pieces

- `src/layouts/AppShell.tsx`: the window, a resizable sidebar and the screen. No footer.
- `src/layouts/Screen.tsx`: toolbar, optional info strip, optional view tabs, content and an optional inspector.
- `src/components/menus/TargetMenu.tsx`: a right-click menu described as data, rendered natively on the desktop and in-window in a browser.
- `src/features/notes/AiNote.tsx`: an agent's note on a failure cause, drawn with the AI tokens.
- `src/components/RanchMark.tsx`: the logo (see [Logo](#logo)).

## Rules every screen follows

**No clipped or ellipsised labels.** Text is never cut off and never ends in "…" because it ran out of room. Layouts are sized for their content: long titles and queue names wrap, tables drop columns that do not fit, code scrolls inside its block. Button labels never end with "…"; only native menu items that open a dialog do ("Settings…"). A pending action shows a spinner, not an ellipsis.

**AA text contrast in both themes.** Text reaches 4.5:1 against its background, or 3:1 for large text, in dark and light. `--dim` is for zero values only. Environment dots, as graphics, reach 3:1.

**Colour has one meaning.** Accent for the user's actions, AI colour for agents, danger for destructive buttons only, state colours for job states, environment colours for environments.

**Destructive actions say what they do.** A confirmation states counts before anything runs, and Cancel has the focus.

**Every action is reachable from the keyboard.** Every command in a menu shows its shortcut, and the palette shows the same one. Lists move with the arrow keys; sheets close with Esc.

**Checked by machines, not only by eye.**

- `pnpm check:overflow` runs Playwright against Ranch web on every screen, in both themes, at 1440 × 900 and 1100 × 700. It fails on text that is clipped or ellipsised, runs into its neighbour, or leaves the window. The same run checks text contrast on every screen (`e2e/contrast/`), the sheets and the palette (`e2e/sheets/`), and that nothing of AI shows when AI features are off (`e2e/ai/`).
- `src/styles/tokens.test.ts` (part of `pnpm test`) computes the contrast of the token pairs in both themes: `--text` and `--text2` on `--bg`, `--surf` and `--side`; each state's text colour on `--bg`; `--accent-text`, `--ai-text` and `--danger-text` on their fills; `--link` on `--bg`; and the environment colours at 3:1.

A new screen adds its route to `e2e/overflow/routes.ts` and demo data to `src/dev/webPreview.ts`, so the checks cover it.

## Logo

The mark is three bars: the head of the queue, in the accent colour, sliding out of the line.

- **Component.** `src/components/RanchMark.tsx` draws it in SVG: the top bar in `var(--accent)`, the two others in `currentColor`. At 16 px and below it switches to a drawing on the pixel grid so the bars stay crisp. It is decorative (`aria-hidden`): the name always sits next to it.
- **Wordmark.** `ranch`, in lowercase, set in JetBrains Mono at weight 600.
- **App icon.** The mark on a graphite tile, `#17181c`, with the head bar in `#f28c38` and the others in `#ececee`.

Sources, in `src-tauri/icons/source/`:

| File | What it is |
|---|---|
| `ranch-icon.svg` | The app icon on the macOS grid: an 824 px body inside 1024 px, the mark at 56 % of the body, a soft shadow and a faint light edge. |
| `ranch-tile.svg` | The tile without the macOS margin, for favicons and the in-app logomark. |
| `ranch-tile-32.svg` | The tile drawn on the 32 px grid: 4 px bars, 2 px gaps. |
| `ranch-tile-16.svg` | The tile drawn on the 16 px grid: 2 px bars, 1 px gaps. |

The platform icons in `src-tauri/icons/` and the favicons in `public/` are renderings of these sources.
