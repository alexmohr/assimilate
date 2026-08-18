<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# UI Design Skill

Use when:

* adding or changing anything a user sees — a view, a panel, a dialog, a form,
  a table, a chart, a state message
* writing CSS anywhere under `frontend/src/`
* choosing between building something and reusing something

For the code rules that apply to the same files (typing, lint, build, test),
see `skills/frontend/SKILL.md`.

## The rule

**Reuse before you write.** Every visual constant is a token, and every
recurring shape is either a class in `frontend/src/style.css` or a component in
`frontend/src/components/`. Read the catalogue below before adding a rule to a
scoped `<style>` block.

A scoped `<style>` block is for what is genuinely unique to that component: its
own layout, its own one-off elements. It is not the place for a button, a
panel, a badge, a table, a form field, a breadcrumb, an empty state, or a
spinner — those already exist.

Three tests enforce this and fail CI:

| Test | Fails when |
| --- | --- |
| `design-tokens.test.ts` | a literal `font-size`, `border-radius` or `transition` duration, or a `var()` naming a token that does not exist |
| `shared-components.test.ts` | a scoped rule re-declares a property a shared class already sets, two scoped stylesheets declare the same rule verbatim, or a shared `@keyframes` is redefined locally |
| `ui-conventions.test.ts` | a text control without `.input`, an icon outside the size set, a glyph entity instead of an icon, an ellipsis character in a busy label, a `.state-msg` used as a block-level empty state, or a view over 1,800 lines |

`styles/contrast.test.ts` checks every foreground/background token pair against
WCAG AA (4.5:1). Three pairs are known failures marked `it.fails` — light
`--success` on base, and white on dark `--accent` and dark `--danger`. Fixing a
token means flipping its case from `it.fails` to a passing assertion.

## Design tokens

All visual constants live in `frontend/src/style.css`.

* **Type scale** — `--fs-2xs` `--fs-xs` `--fs-sm` `--fs-base` `--fs-md`
  `--fs-lg` `--fs-xl`, plus `--fs-2xl` / `--fs-3xl` for decorative numerals
  only (error codes, score rings). Never a literal `rem`. The `--fs-*` name is
  deliberate: Tailwind's theme owns `--text-*` to drive its `text-*` utilities,
  so a same-named token here would resolve differently depending on whether it
  was reached through `var()` or through a utility class.
* **Radius** — `--radius-sm`, `--radius`, `--radius-pill`. Literal `50%` (a
  circle) and `0` (a deliberate square edge) are the only exceptions.
* **Duration** — `--duration-fast` (0.1s), `--duration-base` (0.15s, the
  default for hover and state feedback), `--duration-slow` (0.3s),
  `--duration-value` (0.4s, for progress fills and chart sweeps). Keyframe
  `animation` timings are exempt; `transition` is not.
* **Colour** — every colour comes from a token defined in both `:root` and
  `.dark`. A `var(--x, fallback)` with a hardcoded fallback is a bug: if `--x`
  exists the fallback is dead, and if it does not the value silently stops
  following the theme.
* **Focus** — one `:focus-visible` rule in `@layer base` covers every control,
  wrapped in `:where()` so it carries zero specificity. Do not add per-component
  focus styling. `outline: none` on `:focus` is safe because the global rule
  targets `:focus-visible`.
* **Motion** — `@media (prefers-reduced-motion: reduce)` in `@layer base`
  neutralises every animation and transition. Do not add motion that bypasses
  it.

## Page and layout

* **Page head** — `.page-header` with `.page-title`, `.header-actions` for the
  buttons on the right, `.page-description` for a sentence under it.
* **Toolbar** — `.toolbar` above a list, holding `.search-input`,
  `.filter-toggle` (with `.filter-badge` when a filter is applied) and
  `SortControls`.
* **Filter panel** — `.filters` containing `.filter-row` > `.filter-group` >
  `.filter-label`; `.row-count` for the result tally.
* **Card list** — `.card-grid`, a responsive auto-fill grid that collapses on
  narrow viewports.
* **Two-pane archive screens** — `ArchiveBrowserLayout`, with `narrow-list`
  when the list pane is a fixed narrow column.
* **Surfaces** — `.panel` is a list container (`.panel--sectioned` for the
  ruled-header/flush-body variant, `.panel-header` + `.panel-title`, plus
  `.panel-title--truncate` when the heading shares its row with controls).
  `.info-card` is the detail views' settings surface, with `.info-title`,
  `.info-grid` (`dt`/`dd` rows) and `.info-actions`.

## Components

Reach for these rather than rebuilding them.

### Structure and navigation

* **Dialogs** — `BaseModal`. Supplies `role="dialog"`, `aria-modal`, Escape,
  the focus trap, the scroll lock and focus restore. Use its `form` prop when a
  footer submit button must submit fields in the body. Never hand-roll an
  overlay.
  * yes/no destructive prompt → `ConfirmDeleteDialog` (`show`, `title`,
    `submitting`, `error`, `confirmLabel`, `submittingLabel`)
  * a form → `ModalFormActions` in the `#footer` slot
* **Tabs** — `BaseTabs` (`tabs`, `v-model`, `label`). Carries the ARIA roles
  and keyboard behaviour; a hand-rolled row of buttons does not. Tab panels get
  `.fade-in` for the panel-swap transition.
* **Segmented controls** — `BaseSegmented`, same reasoning.
* **Breadcrumbs** — `.detail-breadcrumb` with `.crumb-link` / `.crumb-sep` /
  `.crumb-current` for a detail view's trail. A *path* trail is `.path-crumbs`
  with `.crumb`, `.crumb-last` on the tail and `.crumb-root` on a `/` root.
* **Sorting** — `SortControls` driven by the `useListSort` composable.

### Content

* **Entity cards** — `.entity-card` is the clickable card in a `.card-grid`,
  with `.entity-card--notable` for a thing switched off but still listed,
  `--hidden` / `--dim` for the two steps back from that, and `--highlighted`
  for the one a link landed on. Inside: `.card-top` / `.card-info` /
  `.card-name` / `.card-meta` / `.card-stats` (`.stat` + `.stat-value` +
  `.stat-label`) / `.card-actions` (`--spread` when a control leads the row).
  `.meta-pill` for a count or classification beside the name. A schedule
  renders through `ScheduleCard`, which fills that shape from a `ScheduleRow`.
* **Stats** — a labelled number is `.stat-label` + `.stat-value`, with
  `.stat-sub` for the line under it saying what the number is of. The two
  larger roles are modifiers: `.stat-value--lg` for a panel tile, `--xl` for a
  dashboard headline. Both lay out as a row, so a badge or status dot beside
  the number needs no extra wrapper.
* **Entity rows** — for lists too long for a card each: `.rows` is the bordered
  container, `.agent-row` one entry, with `.agent-row-stripe` (`--success` /
  `--warning` / `--danger` / `--muted`) down its left edge and
  `.agent-row-name` / `-when` / `-sub` / `-stats` / `-actions` across it.
  `.agent-row-detail` is the expanded body under a row.
* **Group labels** — `.group-label` is the small uppercase caption over a group
  of values, `--lg` where it heads an editable form group, `--warning` /
  `--danger` where it takes the tone of the block it names.
* **Tables** — `.data-table` inside `.table-wrap` (`--framed` when the table is
  itself the card). `.data-table--compact` only for wide numeric grids.
  Cell typography is `.cell-ts` / `.cell-date` / `.cell-host` / `.cell-size` /
  `.cell-mono` / `.cell-muted`, plus `.cell-truncate` for a message of
  unbounded length. PrimeVue `DataTable`s render through the global passthrough
  config in `primevue-pt.ts`, which draws row separators via `tbody: divide-y`
  — a per-view `border-bottom` on `th`/`td` doubles that line. A scoped rule
  aimed at a `DataTable`'s own `th`/`td` needs `:deep()`; without it the
  selector picks up the view's scope id, which PrimeVue's internals never
  carry, and the rule silently does nothing.
* **Badges** — `.badge` with one of `.badge--success|warning|danger|info|`
  `accent|neutral`, chosen through `src/utils/badge.ts`. Add `.badge-dot` for
  live state (Online, Running); omit it for classification (Manual, Admin).
  `.badge--pulse` marks work in flight. `EntityStatusBadges` renders the
  running/notable/issue row on a card.
* **Tags** — `EntityTags` for the editor; `.tag-pill`, `.tag-dropdown` and
  friends for the filter dropdown.
* **Charts** — `MetricLineChart` for a labelled line chart,
  `ChartRangeControls` for the repo/range picker in a panel header,
  `.chart-desc` for the caption under the heading. Options come from
  `useChartTheme` and `useBytesLineChartOptions`; data from
  `useRangeFilteredFetch`.
* **Archive browsing** — `ArchiveFileBrowser`, which owns the path, contents,
  index-status polling and download URLs through `useArchiveBrowser`. A caller
  picks the archive and passes `repo-id` and `archive`.
* **Monospace output** — `.detail-pre`, or `.error-pre` / `.warning-pre` for
  the toned variants. `CardError` for a collapsible error with a toggle.
* **Secrets** — the one-time reveal of a token is `.token-notice` /
  `.token-warning` / `.token-box` / `.token-text`; a repository passphrase is
  the same shape as `.passphrase-*`.
* **Progress** — `.progress-row` > `.progress-track` > `.progress-bar`, with
  `.progress-label` alongside.
* **Borg pattern help** — `BorgPatternReference`, `variant="inline"` under a
  textarea or `variant="sidebar"` beside an editor.

### Forms

* **Vocabulary** — `.field` / `.field-label` / `.input`, with `.field-hint` for
  help text and `.form-error` / `.form-success` for messages. Every text-shaped
  control (`input`, `select`, `textarea`) carries `.input`; checkboxes, radios
  and file pickers do not. `.form-group` / `.form-label` / `.form-input` and
  `.msg-error` / `.msg-success` are retired names and are rejected by CI.
* **Layout** — `.form-grid` (two columns, `.field-full` to span both) or
  `.form-stack` (one column); `.field-row` for fields side by side, with
  `.field-narrow` for a short value.
* **Inline variants** — `.field field-inline` puts a control beside its label
  instead of under it (used for toggles); `.field-label-row` shares a label's
  row with a secondary control; `.toggle-row` (+ `--spread`) pairs a toggle
  with `.toggle-row-label`.
* **Sizing** — `.input-sm` for a toolbar or table-header control;
  `.select-input` (+ `--sm` / `--md` / `--lg`) for a `<select>` that sizes to
  its content; `.filter-input` for a table column filter; `.area-input`
  (+ `--sm`) for a textarea of paths or patterns.
* **Toggles** — `ToggleSwitch` for a setting that applies on change. A native
  checkbox only inside a multi-select list, or in a form with an explicit Save.
* **Editable cards** — `EditableInfoCard` owns the whole read/edit/Cancel/Save
  shell for a card whose header is just a title. A card that drives its own
  `.info-card-header` (because it carries extra actions there) shares only the
  footer, via `EditFormActions`.
* **Per-agent overrides** — `PerAgentFields`, for a schedule form section
  configured separately per selected agent.

### States

* **Loading** — `BaseSpinner` (`sm` / `md` / `lg`); `.loading-row` to centre
  one in the space the content will take; `BaseSkeleton` for a content
  placeholder. Never hand-roll a bordered spinning circle.
* **Empty** — `EmptyState`, with an action wherever one would resolve the
  emptiness. `.state-msg` is for errors and for `.state-msg--inline` inside a
  dashboard widget — not for "nothing here yet", which CI rejects.
* **Error** — `.state-msg state-error` inline, `.error-banner` above the
  content it invalidates, `.form-error` under the control that failed.
  `ErrorPage` for a full-screen failure (`code`, `tone`, slots for a source
  line and detail).
* **Danger zone** — `.danger-zone` / `.danger-body` / `.danger-info` /
  `.danger-heading` / `.danger-desc` on a detail page's destructive panel.
* **Live activity** — `.pulse-dot` (`--success` / `--accent`) for a breathing
  dot; `.spinning` to turn any icon into a busy indicator.
* **Toasts** — `useToast` and `ToastContainer` for transient outcomes;
  `.save-success` for a confirmation that stays beside a submit button.

## Wording and icons

* **Icons** — `@lucide/vue` only, never an HTML entity or a literal glyph.
  Sizes are 12 (inline with small text), 14 (inline and in controls), 16
  (headings), 20 (section headers), 40 (empty states). CI rejects any other
  size.
* **Busy labels** — three periods, `'Saving...'`, never an ellipsis character.
* **Secondary text** — `.muted` for a hint, an unset placeholder or an inactive
  value. `.mono` for machine values inline.

## Adding something new

1. **Is it a variant of something here?** Add a modifier
   (`.thing--variant`) next to the base rule in `style.css`.
2. **Is this its second use?** Move the rule into `style.css`'s
   `@layer components`, delete both copies, and add the class to the `OWNED`
   list in `shared-components.test.ts`.
3. **Is the markup duplicated too?** Extract a component into
   `frontend/src/components/`, with props for what differs and slots for what
   the callers own.
4. **Is it genuinely one-of-a-kind?** A scoped `<style>` block is correct.
   Keep the view under 1,800 lines; split on a tab or dialog boundary before
   raising that number.

New visual constants go in `style.css` as tokens, never inline. New shared
classes belong in `@layer components` so Tailwind's cascade layers keep utility
classes winning over them.
