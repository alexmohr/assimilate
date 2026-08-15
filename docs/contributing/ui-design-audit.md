<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# UI Design Language Audit

A static audit of `frontend/src` against the design system declared in
`frontend/src/style.css`. Every finding below records what the code does today,
where, and what it should do instead.

Counts are occurrences in source, not runtime instances.

Scope: 39 components, 25 views, 1 layout, 37,405 lines.

## Summary

| Metric | Value |
| --- | --- |
| Hand-rolled modals bypassing `BaseModal` | 43 |
| `:focus-visible` rules in the entire frontend | 0 |
| Distinct `font-size` values | 29 |
| Base badge styles | 9 |
| Table styles | 7 |
| Copies of `.panel` | 12 |
| `.close-btn` elements with an accessible name | 0 of 43 |

Severity legend: Critical (blocks keyboard/AT use), High (visible inconsistency
or systemic duplication), Medium, Low.

## Part I: Foundations

### F-01 (High) There is no type scale, only 29 opinions

`@theme` defines a spacing unit, a radius set and two font families but no font
sizes, so every component picks its own. Sizes cluster into near-duplicates that
are indistinguishable on screen but guarantee misalignment when two components
sit side by side: `0.8rem` / `0.8125rem` / `0.82rem` / `0.825rem` / `0.83rem`
are five separate values in the codebase.

Distribution: `0.875rem` (113), `0.8rem` (92), `0.75rem` (83), `0.85rem` (60),
`0.7rem` (37), `0.78rem` (37), `0.82rem` (32), `0.8125rem` (24), `0.65rem` (24),
`0.72rem` (21), plus 19 more.

Fix: add a seven-step scale to `@theme`, then map each literal to the nearest
step.

```css
/* frontend/src/style.css - inside @theme */
--text-2xs: 0.6875rem;   --text-2xs--line-height: 1.4;
--text-xs:  0.75rem;     --text-xs--line-height: 1.45;
--text-sm:  0.8125rem;   --text-sm--line-height: 1.5;
--text-base:0.875rem;    --text-base--line-height: 1.5;
--text-md:  0.9375rem;   --text-md--line-height: 1.55;
--text-lg:  1.125rem;    --text-lg--line-height: 1.35;
--text-xl:  1.5rem;      --text-xl--line-height: 1.25;
```

Add a stylelint `declaration-property-value-allowed-list` rule for `font-size`
so the scale cannot drift again.

### F-02 (Medium) Radius and spacing bypass the tokens that already exist

`--radius` and `--radius-sm` are used 205 times, which is the system working.
Then 27 declarations use raw pixels instead, and the pill shape is written four
different ways, none of them a token: `999px` (25), `9999px`, `99px` (2), plus
`3px` (7), `4px` (6), `2px` (3), `1px`, `5px`, `6px`, `8px`, `10px`, `0.2rem`,
`0.25rem`.

`gap` uses 19 distinct rem values, including `0.1`, `0.125`, `0.15`, `0.2`,
`0.25`, `0.3`, `0.35`, `0.375`, `0.4`.

Fix: add `--radius-pill: 999px` and `--radius-lg: 0.875rem` to `@theme` and
replace all pixel radii. Collapse the gap swarm onto the existing `--spacing`
unit (0.25rem); the sub-0.4rem values are noise.

### F-03 (High) Five CSS variables are referenced but never defined

`var(--muted, #6b7280)` looks like a themed value with a safety net. It is not:
`--muted` does not exist anywhere in the codebase, so the fallback is the only
thing that ever renders. The cancelled-run badge in `ScheduleDetailView` is
therefore hardcoded light grey in both themes, a near-white chip on a `#18181b`
card.

| Site | Undefined variable |
| --- | --- |
| `ScheduleDetailView.vue:2543` | `--muted-subtle` |
| `ScheduleDetailView.vue:2544` | `--muted` |
| `ScheduleDetailView.vue:2554` | `--danger-hover` |
| `BackendUnreachable.vue:60` | `--radius-lg` |
| `RecentActivityWidget.vue:241`, `AgentDetailView.vue:2520` | `--bg-code` |

Fix: point the five call sites at real tokens (`--bg-hover`, `--text-muted`,
`--radius`), and define `--danger-hover` and `--bg-code` in both `:root` and
`.dark` if they are genuinely wanted.

### F-04 (Medium) The responsive system ignores its own breakpoints

`@theme` declares `sm 640 / md 768 / lg 1024 / xl 1280`. The stylesheets use six
ad-hoc `max-width` queries, four of which match nothing in the scale: 500, 640,
700 (5 uses), 768, 900, 1100. The layout therefore breaks in three separate
places as the window narrows.

Fix: snap all six to the declared four. Where a component genuinely needs an
intermediate reflow, that is a container-query case (`@container`), not a fifth
breakpoint.

### F-05 (Medium) Ten transition durations, and nothing respects reduced motion

`0.15s` is the house standard (76 uses). The other 42 declarations pick from
nine alternatives: `0.1s` (16), `0.2s` (8), `1.5s` (6), `0.3s` (5), `0.7s`,
`0.4s`, `1s`, `1.4s`, `0.75s`.

Separately, all 14 keyframe animations run unconditionally, including
`pulse-importing` (`style.css:329`) which pulses a badge infinitely while an
import runs. There is no `prefers-reduced-motion` block anywhere in the
frontend.

```css
/* style.css - three durations, one escape hatch */
--duration-fast: 0.1s;  --duration-base: 0.15s;  --duration-slow: 0.3s;

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

## Part II: Components that exist several times

### F-06 (High) `.panel` is copy-pasted into 12 files as two incompatible shapes

The same four declarations appear in twelve scoped stylesheets. Worse, they are
not the same component: eight files define a padded panel, four views define an
overflow-hidden panel whose body is flush and whose header carries a rule. Both
are called `.panel`. Neither is in `style.css`.

- Padded: `BackupStatsWidget`, `StorageTrendWidget`, `RepoHealthWidget`,
  `NextScheduledWidget`, `RecentActivityWidget`, `TrendsChart`,
  `BackupCalendar`, `DashboardView`
- Sectioned: `ScheduleDetailView:2559`, `RepoDetailView:2883`,
  `ExcludesView:196`, `ArchivesView:883`

```css
/* style.css @layer components - promote once, delete 12 copies */
.panel { background: var(--bg-card); border: 1px solid var(--border);
         border-radius: var(--radius); padding: 1.25rem; }
.panel--sectioned { padding: 0; overflow: hidden; }
.panel--sectioned > .panel-header { padding: 0.875rem 1.25rem;
         border-bottom: 1px solid var(--border); margin-bottom: 0; }
.panel--sectioned > .panel-body { padding: 1.25rem; }
```

### F-07 (High) Panel headings have four different treatments

`.panel-title` is defined 14 times in four mutually exclusive styles.

| Variant | Style | Files |
| --- | --- | --- |
| A | `0.875rem` / 600 / `--text-primary` | 9 |
| B | `0.75rem` / 600 / uppercase / 0.06em / `--text-muted` | 2 |
| C | `0.8rem` / 600 / uppercase / 0.06em / `--text-muted` | 2 |
| D | `0.8rem` / 700 / uppercase / 0.06em / `--text-muted` | 1 |

The Dashboard renders nine panels at once; the detail views render the uppercase
kind. A user moving from Dashboard to Repository sees the heading language
change under them.

Fix: keep treatment A. It is already the majority and it reads as a heading
rather than a form label. Reserve uppercase-muted for the `.field-label` role it
already owns.

### F-08 (High) Nine badge styles, three casings, one that is not even a pill

Status is the single most repeated visual idea in a backup tool, and it has no
canonical form.

| Site | Style |
| --- | --- |
| `style.css:306` | `.status-badge` - 0.7rem / uppercase / 999px |
| `AuditLogView:442`, `ActivityLogView:1271` | `.badge` - 0.75rem / capitalize |
| `ScheduleDetailView:2513` | `.badge` - 0.72rem / capitalize |
| `ProfileView:1193` | `.badge` - 0.75rem / no casing rule |
| `ActivityLogView:1534` | `.badge-level` - 0.65rem / 700 / uppercase |
| `AgentDetailView:2371`, `SchedulesView:668` | `.type-badge` - 0.65rem |
| `HostsView:1313` | `.badge-imported` - 0.68rem / plus 1px border |
| `EntityStatusBadges:96` | `.entity-status-pill` - 0.65rem / uppercase |
| `UsersView:808` | `.role-badge` - `border-radius: var(--radius-sm)` |

`UsersView`'s role badge is the only one with a rounded-rectangle shape, so on
the Users page a squared chip sits inches from a rounded one.

```css
/* style.css @layer components */
.badge { display: inline-flex; align-items: center; gap: 0.3rem;
  padding: 0.2rem 0.55rem; border-radius: var(--radius-pill);
  font-size: var(--text-2xs); font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.04em; }
.badge--success { background: var(--success-subtle); color: var(--success); }
.badge--warning { background: var(--warning-subtle); color: var(--warning); }
.badge--danger  { background: var(--danger-subtle);  color: var(--danger);  }
.badge--info    { background: var(--info-subtle);    color: var(--info);    }
.badge--neutral { background: var(--bg-hover);       color: var(--text-muted); }
```

Then delete `.status-badge`, `.badge-*`, `.type-badge`, `.role-badge`,
`.badge-imported`, `.badge-level` and `.entity-status-pill`. Suggested
convention: a leading dot marks live state (Online, Running, Failed); no dot
marks classification (Manual, Imported, Admin).

### F-09 (High) Seven table styles; the shared one is used by five of twelve

`style.css` ships `.data-table`. Twelve files render a `<table>`; five use it.

| Site | Style |
| --- | --- |
| `style.css:406` | `.data-table` - 0.875rem, sentence-case th |
| `UsersView:774` | `.users-table` - byte-for-byte copy of `.data-table` |
| `TunnelsView:679` | `.tunnels-table` - uppercase th, `--bg-card` header fill |
| `ProfileView:1153`, `ApiTokenTable:66` | 0.85rem, bordered and rounded |
| `ScheduleDetailView:2452` | `.reports-table` - 0.82rem, th weight 700 |
| `SystemView:1038` | `.storage-table` - 0.75rem, right-aligned |
| `RolesView:465` | `.matrix-table` - 0.8125rem, centred th |

Six different body font sizes, three unrelated header treatments.

Fix: delete the six clones. Add exactly one modifier,
`.data-table--compact` (`--text-sm`, tighter cell padding, `tabular-nums`), for
the storage and permission matrices that genuinely need density. Wrap every
table in `.table-wrap { overflow-x: auto }`; only three currently have one.

### F-10 (Medium) Four segmented controls, four sizes, two ways to say white

The same "pick one of these" control is implemented four times.

| Site | Style |
| --- | --- |
| `BackupStatsWidget`, `StorageTrendWidget`, `TrendsChart`, `DashboardView` | `.toggle-btn` - 0.65rem uppercase, `--text-muted` |
| `ScheduleDetailView:2247` | `.seg-btn` - 0.82rem, active `color: #fff` |
| `ActivityLogView:1055` | `.segment-btn` - 0.875rem, active `color: #fff` |
| `FileSearch:289` | `.mode-btn` - 0.8rem, `--text-secondary` |

The active state uses `var(--text-on-accent)` in two and a hardcoded `#fff` in
the other two, so the token is bypassed for the exact case it exists to cover.

Fix: extract `BaseSegmented.vue` taking `options: {value, label}[]` and
`v-model`. It should render `role="radiogroup"` with `aria-checked`, which none
of the four do today.

### F-11 (Medium) Three tab bars; the schedule one is a third smaller

`RepoDetailView:2446` and `AgentDetailView:2059` define byte-identical
`.tab-btn` blocks: `0.75rem 1.25rem` padding, `0.875rem`, weight 500,
`--text-secondary` at rest. `ScheduleDetailView:1804` defines a third:
`0.6rem 1.2rem`, `0.82rem`, weight 600, `--text-muted` at rest.

All three views are reached from the same navigation, so the tab strip visibly
shifts as you move between them.

Fix: extract `BaseTabs.vue` with `role="tablist"`, `aria-selected` and arrow-key
roving focus.

### F-12 (Critical) 43 hand-rolled modals; the one that works is used by four files

`BaseModal.vue` does everything correctly: `Teleport`, `role="dialog"`,
`aria-modal`, Escape to close, Tab focus trap, body scroll lock, focus restore
on close, enter/leave transition. It is imported by four files: `ArchiveDiff`,
`RestoreWizard`, `RepoDetailView`, `ReposView`.

Everywhere else, dialogs are hand-written `.overlay > .dialog` markup with none
of those behaviours:

| View | Raw dialogs |
| --- | --- |
| `RepoDetailView` | 7 |
| `NotificationsView` | 5 |
| `TunnelsView` | 4 |
| `AgentDetailView` | 4 |
| `ProfileView` | 3 |
| `GroupsView` | 3 |
| `UsersView` | 3 |
| `HostsView`, `SystemView`, `RolesView` | 2 each |
| 8 more files | 1 each |

Across the whole frontend, `role="dialog"`, `aria-modal` and the Escape handler
each appear in exactly one file: `BaseModal.vue`.

This is not only a consistency problem. A keyboard user cannot dismiss those 43
dialogs with Escape, Tab walks out of the dialog into the page behind it, and
the page scrolls under the overlay.

```vue
<!-- every raw dialog becomes this -->
<BaseModal :open="showDelete" title="Delete Repository" @close="showDelete = false">
  <p class="confirm-text">This permanently destroys the borg repository on disk.</p>
  <template #footer>
    <ModalFormActions :busy="deleting" confirm-label="Delete" tone="danger"
      @cancel="showDelete = false" @confirm="confirmDelete" />
  </template>
</BaseModal>
```

Then delete `.overlay`, `.dialog`, `.dialog-*` and `.close-btn` from
`style.css` so the old pattern cannot be reached again.

### F-13 (High) Two form vocabularies, three focus treatments, 56 unstyled inputs

`style.css` defines `.field` / `.field-label` / `.input`. Fourteen files use it.
`ProfileView` defines a parallel `.form-group` / `.form-label` / `.form-input`
set with the same visual intent; `UsersView` defines a third that styles bare
`label` and `input` descendants.

The focus states diverge in the way that matters most:

| Site | Focus treatment |
| --- | --- |
| `style.css:286` | border plus `box-shadow 0 0 0 3px var(--accent-subtle)` |
| `ProfileView:888` | border colour only, no ring |
| `UsersView:945` | border colour only, no ring |

On `UsersView` and `ProfileView` a focused field is nearly invisible.

Of 132 text `<input>` elements, 56 carry no class at all:
`ScheduleDetailView` 7, `SystemView` 7, `ArchivesView` 7, `ProfileView` 5,
`AuditLogView` 4, `LoginView` 4, `ArchiveFileBrowser` 3, `CronBuilder` 3,
`ActivityLogView` 3, plus 12 more files.

Fix: delete `.form-group`, `.form-label`, `.form-input` and the descendant
selectors in `UsersView`; rename their markup to `.field` / `.field-label` /
`.input`. Then extend `.input` in `style.css` to cover `select` and `textarea`
so unclassed controls inherit the right shape by default.

### F-14 (Low) `.state-msg` is redefined 21 times, including 8 exact duplicates

The global rule is `text-align: center; padding: 3rem; color: var(--text-muted)`.

- Identical re-declaration (adds specificity, changes nothing): `TunnelsView`,
  `HostsView`, `AuditLogView`, `AgentDetailView`, `RepoDetailView`, `ReposView`,
  `NotificationsView`, and `ActivityLogView` (adds only a font-size).
- Different component wearing the same name (tight inline message,
  `padding: 0.5rem 0`, left-aligned): `BackupStatsWidget`, `StorageTrendWidget`,
  `TrendsChart`, `BackupCalendar`, `RepoHealthWidget`, `NextScheduledWidget`,
  `RecentActivityWidget`, `DashboardView`, `FileSearch`.

Fix: delete the 8 duplicates outright. Rename the widget variant to
`.state-msg--inline` and put it in `style.css` next to the block version.

### F-15 (Medium) Four ways to say "nothing here yet"

`EmptyState.vue` exists with an icon slot, a title, a description and a call to
action. Nine of 25 views use it: `FileSearch`, `TunnelsView`, `HostsView`,
`AuditLogView`, `ActivityLogView`, `ReposView`, `NotificationsView`,
`SchedulesView`, `TokensView`.

Everywhere else the empty case is a centred grey sentence, a tight inline
sentence, or a spinner-shaped row, and none of them offer the action that would
resolve the emptiness. Local implementations: `RepositoryCapacity`,
`ArchiveFileBrowser`, `UpcomingWork`, `ScheduleDetailView`, `ProfileView`,
`ArchivesView`, plus nine widgets via `.state-msg`.

### F-16 (Medium) Two icon systems: an icon library, and HTML entities

23 files import `@lucide/vue` and render stroked 24-grid icons. Alongside that,
the app renders glyphs as text:

| Site | Glyph | Should be |
| --- | --- | --- |
| 43 files | `&times;` as `.close-btn` content | `X` |
| `ProfileView:614,622,630` | `&#9881; &#9788; &#9789;` | `Monitor`, `Sun`, `Moon` |
| `ArchivesView:434,553,559` | `&#8635; &#10003; &#9888;` | `RefreshCw`, `Check`, `AlertTriangle` |
| `BackupCalendar:240,247` | `&larr; &rarr;` | `ChevronLeft`, `ChevronRight` |
| `RepoDetailView:1748,1763` | `&#9656; &#9888;` | `ChevronRight`, `AlertTriangle` |
| `CardError:39` | literal triangles | `ChevronUp`, `ChevronDown` |

Entity glyphs take their weight from the system font, so they never match the
2px stroke around them and they shift between platforms.

Icon sizes are also unbounded: `12`, `14`, `16`, `18`, `20`, `22`, `40`, `48`
are eight sizes for four roles. Collapse to 14 (inline), 16 (control), 20
(header), 40 (empty state).

### F-17 (Low) Loading is expressed three ways, and punctuated two ways

`BaseSpinner` is used in 21 files. `BaseSkeleton` exists and is used in one, the
Dashboard, so every other page flashes a spinner where a skeleton would hold the
layout.

Busy button labels are almost all `'Saving...'` with three periods (15 uses),
`'Deleting...'` (11), plus 20 more `-ing...` labels. Two use a real ellipsis
character: `'Running...'`, `'Loading...'`.

Fix: pick one punctuation convention. Then use `BaseSkeleton` for any list or
table whose row count is known ahead of the fetch, and reserve `BaseSpinner` for
in-button and indeterminate cases.

### F-18 (Low) A styled toggle and 27 raw checkboxes, both meaning "on"

`ToggleSwitch.vue` is used in nine files for boolean settings. Ten other files
use native `<input type="checkbox">` for the same job: `UsersView` has 7,
`NotificationsView` 6.

Native checkboxes are correct for multi-select lists; they are not correct for a
setting that takes effect immediately, which is what most of these are.

Rule of thumb worth adding to the style guide: `ToggleSwitch` for a setting that
applies on change; native checkbox only inside a multi-select list or a form
with an explicit Save.

### F-19 (Low) Two components are rendered nowhere

`RepoHealthWidget.vue` (182 lines) and `NextScheduledWidget.vue` (219 lines) are
imported by nothing except their own unit tests. They are also the only two
carriers of panel-title treatment B in F-07, which means part of the
inconsistency being catalogued is invisible to users and pure maintenance cost.

Fix: either wire them into the Dashboard, or delete them with their tests.

## Part III: What the duplication costs in access

### F-20 (Critical) Not one `:focus-visible` rule in the entire frontend

22 declarations across 16 files set `outline: none`, removing the browser's
focus ring. Exactly one thing puts a visible indicator back: `.input:focus`.

Every button in the app (`.btn`, `.tab-btn`, `.close-btn`, `.toggle-btn`, table
row actions) is invisible when focused. A keyboard user cannot tell where they
are.

```css
/* style.css @layer base - one rule covers every control */
:where(a, button, input, select, textarea, summary, [tabindex]):focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
  border-radius: var(--radius-sm);
}
```

`:where()` keeps specificity at 0, so component styles still win on colour, and
the 22 `outline: none` rules only suppress `:focus`, not `:focus-visible`.

### F-21 (Critical) 43 dialogs are not dialogs as far as assistive tech is concerned

The direct consequence of F-12. Screen readers announce the raw overlays as
plain `div`s with no boundary, no name and no modal semantics; the page behind
stays in the reading order.

Because the dialogs are hand-written rather than composed, this cannot be fixed
centrally, which is the argument for fixing F-12 first. Resolved entirely by the
F-12 migration.

### F-22 (High) 43 close buttons, zero accessible names

Every `.close-btn` in the app contains only `&times;` and carries no
`aria-label`. A screen reader announces "multiplication sign, button".

`BaseModal.vue:117` (`aria-label="Close"`) and `ToastContainer.vue:47`
(`aria-label="Dismiss"`) both do this correctly, so the pattern is established;
it just was not carried into the copies.

The handful outside dialogs (chip removal in `FileChangePatternsEditor`,
`BackupCalendar`, `AgentDetailView` tag lists) need
`aria-label="Remove <name>"` individually.

### F-23 (Medium) Tabs and segmented controls have no ARIA roles

`role="tab"` and `aria-selected` appear zero times in `frontend/src`. None of
the three tab bars or four segmented controls emit them, and none support
arrow-key navigation. They are announced as a run of unrelated buttons, and the
selected one is distinguished only by colour.

Falls out of the `BaseTabs` and `BaseSegmented` extractions in F-10 and F-11.

## Part IV: Views that carry too much

### F-24 (High) Five views are over 2,000 lines; the largest is 3,359

| View | Total | Script | Template | Style | Refs | Fns | Dialogs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `RepoDetailView` | 3,359 | 1,018 | 1,317 | 1,014 | 71 | 39 | 7 |
| `AgentDetailView` | 2,819 | 867 | 1,131 | 811 | 73 | 47 | 4 |
| `ScheduleDetailView` | 2,646 | 675 | 1,061 | 900 | 51 | 20 | 1 |
| `ReposView` | 2,434 | 820 | 892 | 712 | 35 | 31 | 1 |
| `NotificationsView` | 2,031 | 685 | 850 | 486 | 47 | 43 | 5 |
| `ActivityLogView` | 1,694 | 510 | 484 | 690 | 31 | 16 | 0 |
| `DashboardView` | 1,404 | 409 | 401 | 584 | 25 | 13 | 0 |
| `HostsView` | 1,363 | 558 | 435 | 360 | 34 | 25 | 2 |

`RepoDetailView.vue` is not one screen. It is a repository editor, an archive
browser, a schedule list, a borg console and seven confirmation flows sharing a
single `<script setup>` scope, plus 124 CSS classes.

Every one of the Part II duplications lives in files like this, because a
1,000-line scoped stylesheet is where re-declaring `.panel` feels cheaper than
importing it.

Proposed split for `RepoDetailView` (3,359 lines to roughly 380):

| New file | Contents | Approx lines |
| --- | --- | ---: |
| `RepoDetailView.vue` | route, fetch, tab state, layout | 380 |
| `RepoOverviewTab.vue` | info card, edit form, tags | 520 |
| `RepoArchivesTab.vue` | archive list, filters, browser | 640 |
| `RepoSchedulesTab.vue` | schedule list for this repo | 230 |
| `RepoBorgConsole.vue` | console output panel | 200 |
| `RepoDangerZone.vue` | destroy, remove, relocate, reset, break lock | 340 |
| `useRepoDetail.ts` | composable: fetch, websocket, refresh | 260 |
| (7 dialogs move to `BaseModal` / `ConfirmDeleteDialog`) | | -460 |

Split on the tab boundary first: it is the cheapest cut and it immediately caps
the scoped stylesheet size. The same shape applies to `AgentDetailView` and
`ScheduleDetailView`.

### F-25 (High) Dialog markup is inlined into views instead of composed

Seven dialogs in `RepoDetailView`, five in `NotificationsView`, four each in
`TunnelsView` and `AgentDetailView`. Each is roughly 45 lines of near-identical
markup, so about 1,900 lines of the frontend is copies of the same overlay.

Five of the seven in `RepoDetailView` (delete, remove, relocate, reset, break
lock) are plain destructive confirmations, and `ConfirmDeleteDialog.vue` already
exists for exactly that. It is used by none of them.

Fix: route every yes/no destructive prompt through `ConfirmDeleteDialog`; route
the rest through `BaseModal` with `ModalFormActions` in the footer slot.

### F-26 (Medium) Per-view stylesheets up to 1,014 lines, mostly re-declaring shared patterns

`RepoDetailView` carries 1,014 lines of scoped CSS defining 124 classes.
`ActivityLogView`'s stylesheet (690 lines) is larger than its template (484).

Duplicated wholesale across views:

- `.danger-zone` plus `.danger-body` - 3 views, identical
- `.info-card` - 4 files
- `.tag-pill` and `.tag-dropdown` - `HostsView`, `ReposView`,
  `AgentDetailView`, `RepoDetailView`
- `.stats-select` - 2 files, identical
- `.filter-badge` - 4 views
- `.toolbar` plus `.search-input` - 3 views, each overriding the global
  `.search-input` width from 260px to 220px

Fix: every promotion in Part II deletes lines here. Extract `TagPicker.vue` for
the four copies of the tag dropdown and `DangerZone.vue` for the three copies of
the destructive-actions panel.

### F-27 (Medium) A local override silently breaks the global button contract

`ScheduleDetailView:2547` redefines `.btn-danger` inside its scoped block. The
global rule (`style.css:163`) hovers by dropping opacity to 0.9; the local one
swaps the background for a darkened mix that resolves to an undefined token (see
F-03).

So the Delete button on that one page hovers differently from the identical
Delete button everywhere else, and the difference is invisible in review because
the override sits 700 lines below the markup.

Fix: delete the override. Add a lint guard so scoped styles cannot define a
class that already exists in `style.css`'s `@layer components`; a short
stylelint plugin or a CI check over the known component-class list catches all
of these, including the `.state-msg` and `.panel` cases.

## Order of work

Ordered so each pass makes the next one smaller. Tokens first, because every
component fix references them; the modal migration second, because it is the
largest single deletion and it closes three access findings at once.

| Pass | Work | Findings |
| --- | --- | --- |
| 1 | Land the tokens: type scale, radius and duration tokens, the undefined variables, the global `:focus-visible` rule, the reduced-motion block. Roughly 60 lines added to `style.css`, nothing else touched. | F-01, F-02, F-03, F-05, F-20 |
| 2 | Migrate every dialog to `BaseModal`. 43 call sites, mechanical, roughly 1,400 lines deleted. Then remove `.overlay`, `.dialog*` and `.close-btn` from `style.css`. | F-12, F-21, F-22, F-25 |
| 3 | Promote the shared components into `style.css` and delete the local copies. Extract `BaseTabs`, `BaseSegmented`, `TagPicker`, `DangerZone`. Add the F-27 lint guard in the same change. | F-06 to F-11, F-14, F-23, F-26, F-27 |
| 4 | Unify forms, icons and states: one form vocabulary, one icon system, `EmptyState` everywhere, one busy-label convention, one toggle rule. | F-13, F-15, F-16, F-17, F-18 |
| 5 | Split the oversized views on their tab boundaries. Done last on purpose: after passes 1 to 4 these files are already several hundred lines lighter, so the split is a move rather than a rewrite. | F-04, F-19, F-24 |
