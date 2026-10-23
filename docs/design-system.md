# Design system

Waypoint's web UI is built from one set of design tokens and a small set of primitives. They encode the UI conventions in [AGENTS.md](../AGENTS.md): the 4px grid, `cursor-pointer` on everything clickable, hover, `focus-visible`, active and disabled states, and shared motion tokens that honour `prefers-reduced-motion`. Later screens compose these pieces instead of restyling raw elements.

- Tokens: `web/src/lib/styles/tokens.css`, imported from `web/src/routes/layout.css`.
- Motion helper: `web/src/lib/motion.ts`, imported as `#lib/motion.js`.
- Primitives: `web/src/lib/components/ui/`, imported from the barrel `#lib/components/ui`.
- Gallery: `/dev/primitives` shows every primitive in every variant and state. It has no nav link; open it by URL.

## Tokens

Runtime values are plain `--wp-*` custom properties on `:root`. `@theme inline` maps them into Tailwind, so utilities read the variable at runtime and the theme overrides apply without a rebuild.

### Colour

Use these as Tailwind colour utilities: `bg-*`, `text-*`, `border-*`, `outline-*`, `accent-*`.

| Token | Use |
|---|---|
| `bg` | Page background |
| `surface`, `surface-raised` | Cards and controls; hover fills and skeletons |
| `border`, `border-strong` | Hairlines; control borders |
| `fg`, `fg-muted`, `fg-subtle` | Body text; secondary text; placeholders and disabled text |
| `primary`, `primary-hover`, `primary-active`, `primary-fg` | Primary actions and links, with the text colour on top |
| `danger`, `danger-hover`, `danger-active`, `danger-fg` | Destructive actions and error text |
| `focus` | Focus ring |
| `{neutral,primary,success,warning,danger,info}-soft` and `-soft-fg` | Badge background and text pairs; `-soft-fg` is also the toast accent bar |
| `overlay` | Dialog and Drawer backdrop: black at 40% (light) or 60% (dark) |

Values come from Tailwind's neutral, indigo, red, green, amber and sky scales. Text and background pairs meet WCAG AA (4.5:1) in both themes.

### Spacing, radius, shadow, font

- Spacing: `--spacing: 0.25rem`, so `p-1` is 4px and `p-4` is 16px. Use whole steps only. Half steps (`*-0.5`, `*-1.5`, `*-2.5`, `*-3.5`) are off the grid, and a test fails if a primitive uses one.
- Radius: `rounded-sm` (4px), `rounded-md` (8px), `rounded-lg` (12px), `rounded-full`.
- Shadow: `shadow-sm`, `shadow-md`, `shadow-lg`. The dark theme uses stronger, darker shadows.
- Font: `font-sans` and `font-mono` are system stacks. There is no web font. Text sizes are Tailwind's defaults.

### Themes

The app follows the OS colour scheme. To force a theme, set `data-theme` on `<html>`:

```html
<html data-theme="dark"> <!-- or "light" -->
```

Dark values apply under `@media (prefers-color-scheme: dark)` for `:root:not([data-theme="light"])`, and again for `:root[data-theme="dark"]`. `body` uses the `bg` and `fg` tokens. There is no toggle UI yet.

## Motion

| CSS token | Value | `motion.ts` |
|---|---|---|
| `--duration-fast` | 120ms | `durations.fast` |
| `--duration-base` | 200ms | `durations.base` |
| `--duration-slow` | 320ms | `durations.slow` |
| `--ease-standard` | `cubic-bezier(0.2, 0, 0, 1)` | `easings.standard` (`cubicOut`) |
| `--ease-enter` | `cubic-bezier(0, 0, 0.2, 1)` | `easings.enter` (`cubicOut`) |
| `--ease-exit` | `cubic-bezier(0.4, 0, 1, 1)` | `easings.exit` (`cubicIn`) |

- A bare `transition-colors` (or any `transition-*`) uses `--duration-fast` and `--ease-standard`. Pick another duration with `duration-(--duration-base)`, and another easing with `ease-enter` or `ease-exit`.
- Under `prefers-reduced-motion: reduce` every duration token is `0ms`. Put looping animations behind `motion-safe:` (as Skeleton does).
- The CSS tokens and `motion.ts` must stay in sync. Both files say so; there is no drift test.

For Svelte transitions, use the helper so reduced motion is honoured:

```svelte
<script lang="ts">
	import { fade } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
</script>

<div transition:fade={{ duration: motionDuration('base'), easing: easings.standard }}>…</div>
```

`motionDuration(name, reduced?)` returns the token's ms value, or `0` when `reduced` is true. `reduced` defaults to `prefersReducedMotion.current` from `svelte/motion`.

## App shell

Every route renders inside `AppShell` (`web/src/lib/components/app/AppShell.svelte`), which the root layout wraps around the page: a top bar with the "Waypoint" wordmark, the main nav and a live-connection Badge (Connecting, Live, Reconnecting), then the page in a centred `main` (`max-w-5xl`, `px-4 py-8`, `sm:px-8`). Pages don't add their own `main` or outer padding.

The main nav (`<nav aria-label="Main">`) is six [NavLinks](#navlink): Dashboard (`/`), Applications (`/applications`), Resumes (`/resumes`), Memory (`/memories`), Review (`/review`) and Runs (`/runs`). The shell marks the current section from `page.url.pathname`: Dashboard only on `/`, and every other link on its own path and every path under it (Applications on `/applications` and every `/applications/<id>`, Memory on `/memories`, and so on), but not on a path that only starts with the same letters (`/resumesx`). Below `sm` the nav moves onto its own row under the wordmark and badge, and wraps when the links don't fit.

Screens are composed from the primitives only: `primitives-only.test.ts` in `web/src/lib/components/dashboard/` fails if the shell, the root layout or page, a dashboard card, a component in `components/applications`, `components/resumes`, `components/memories`, `components/runs` or `components/review`, or a page under the matching `routes/` folder uses a raw interactive tag, a `cursor-*` class or a half spacing step.

## Primitives

Every primitive accepts `class`, merged after its own classes, and spreads other attributes onto its root interactive element (for Input, Textarea, Select and Checkbox, that's the control itself, not the wrapper). Interactive primitives own their cursor, hover, `focus-visible`, active and disabled styles, so call sites never add `cursor-pointer` or focus styles. Shared class strings live in `web/src/lib/components/ui/styles.ts`.

```ts
import { Button, Input, Card } from '#lib/components/ui';
```

### Button

`variant`: `primary` (default) | `secondary` | `ghost` | `danger`. `size`: `sm` (h-8) | `md` (default, h-10). `type` defaults to `"button"`. A disabled Button doesn't call `onclick`.

```svelte
<Button onclick={save}>Save</Button>
<Button variant="danger" size="sm" disabled>Delete</Button>
```

### IconButton

A square button. `label` is required and becomes the `aria-label`. The icon is the `children`. `variant` defaults to `ghost`; `disabled` works as on Button. `size`: `xs` (size-5, for buttons inside small controls such as a Tag) | `sm` (size-8) | `md` (default, size-10).

```svelte
<IconButton label="Close" onclick={close}><CloseIcon /></IconButton>
```

### Link

An inline `<a>`. `external` adds `target="_blank"` and `rel="noopener noreferrer"`. `disabled` drops the `href`, sets `aria-disabled="true"` and `tabindex="-1"`, and shows `cursor-not-allowed`. Pass internal routes through `resolve()` from `$app/paths`.

```svelte
<Link href={resolve('/jobs')}>Jobs</Link>
<Link href="https://example.com" external>Posting</Link>
```

### NavLink

A navigation `<a>` for the app's sections, styled as a tab-like pill (`rounded-md px-3 py-2 text-sm font-medium`): muted text by default, `surface-raised` background on hover, `border` on active, and the focus ring. `current` sets `aria-current="page"` and keeps the raised background and full-contrast text. `disabled` works as on Link: no `href`, `aria-disabled="true"`, `tabindex="-1"` and `cursor-not-allowed`. Pass internal routes through `resolve()`.

```svelte
<nav aria-label="Main">
	<NavLink href={resolve('/')} current={path === resolve('/')}>Dashboard</NavLink>
	<NavLink href={resolve('/applications')} current={inApplications}>Applications</NavLink>
</nav>
```

### Input, Textarea, Select

Form controls wrapped in a shared Field: label, optional `hint`, optional `error`. `value` is bindable. The label is linked to the control by id (pass `id` or let it be generated). With `error`, the control gets `aria-invalid="true"`, a danger border, and `aria-describedby` pointing at the error text; the hint is linked the same way. Text controls keep the text cursor; Select is `cursor-pointer`. Textarea `rows` defaults to 3.

Select takes `options: { value: string; label: string; disabled?: boolean }[]` and an optional `placeholder`, rendered as a disabled, empty-value first option.

```svelte
<Input label="Company" hint="As on the posting" bind:value={company} />
<Textarea label="Notes" bind:value={notes} />
<Select label="Stage" options={stages} placeholder="Pick a stage" bind:value={stage} error={stageError} />
```

### FileInput

A file picker wrapped in the same Field as Input: `label`, optional `hint` and `error`, generated or passed `id`. The control is the native `<input type="file">`, so keyboard, focus and the picker work as usual, styled like the other controls: `cursor-pointer`, hover and active backgrounds, the focus ring, and `cursor-not-allowed` with reduced opacity when `disabled`. The browser's own button and file-name text are hidden and redrawn as a "Choose file" chip (`buttonLabel` to change it) followed by the picked file's name, or "No file chosen", so it reads the same in every browser. Pass `accept` to filter the picker. `onchange` is called with the picked `File`, or `null` when the pick is cleared; the component doesn't bind the file. It picks one file at a time.

```svelte
<FileInput
	label="PDF file"
	accept="application/pdf,.pdf"
	buttonLabel="Choose PDF"
	error={fileError}
	onchange={(picked) => (file = picked)}
/>
```

### Checkbox

A native checkbox with a required `label`. `checked` is bindable. The input and label are `cursor-pointer` (`cursor-not-allowed` when disabled), and clicking the label toggles it.

```svelte
<Checkbox label="Remote only" bind:checked={remote} />
```

### Card

A surface with border, radius, small shadow and `p-4`. Without `href` it's a non-interactive `<div>`. With `href` it's an `<a>` with hover, focus-visible and active states.

`draggable` (default `false`) makes the `<div>` a drag source: `draggable="true"`, `cursor-grab`, `active:cursor-grabbing` and a hover border. Toggling it keeps the same element, so a card that's briefly not draggable (a move in flight) keeps its focus and transitions. With `href` set, `draggable` is ignored and the card stays a link. Pass `ondragstart` and `ondragend` like any other attribute; inner links should set `draggable="false"` so a drag that starts on them moves the card rather than the URL.

```svelte
<Card>Summary</Card>
<Card href={resolve('/jobs/[id]', { id })}>Acme · Staff engineer</Card>
<Card draggable={!pending} aria-busy={pending} {ondragstart}>…</Card>
```

### Badge

`tone`: `neutral` (default) | `primary` | `success` | `warning` | `danger` | `info`. A non-interactive `<span>`.

```svelte
<Badge tone="success">Offer</Badge>
```

### Tag

A small neutral pill for one tag, Badge-sized (`h-6`, `rounded-full`, `text-xs`). Props: `label` (required), `onclick`, `onremove`, `disabled` and `class`. Without handlers it's a non-interactive `<span>`. With `onclick` it's a `<button>` with `cursor-pointer`, a primary-soft hover, an active background, the focus ring and `cursor-not-allowed` when disabled. With `onremove` it gets a trailing `xs` IconButton labelled "Remove tag <label>"; with both, the label and the remove button are separate buttons. Long labels truncate.

```svelte
<Tag label="remote" />
<Tag label="remote" onclick={() => filterBy('remote')} />
<Tag label="remote" onremove={() => remove('remote')} />
```

### TagInput

A list of tags in a Field, like Input: `label`, optional `hint` and `error`, generated or passed `id` (the text input's). `value: string[]` is bindable. The tags show as removable Tags, followed by a text input inside one box that looks like a text control and shows the focus ring while the input has focus.

- Enter or a comma adds the typed text, normalised by `normaliseTag` (trimmed, lowercased, whitespace runs collapsed). Text with commas adds each part. A blank or duplicate tag isn't added. Leaving the input adds what was typed.
- Backspace in an empty input removes the last tag; each Tag's remove button removes that tag and returns focus to the input.
- `max` (default 20) stops adding: the hint becomes "At most <max> tags" and the typed text stays in the input.
- Enter never submits an enclosing form. Tags fade in and out with the `fast` token.

`normaliseTag` and `normaliseTags` (in `tags.ts`, exported from the barrel) match `normalise_tag(s)` in `api/src/memories.rs`.

```svelte
<TagInput label="Tags" bind:value={tags} error={errors.tags} />
```

### EmptyState

A centred block for "nothing here yet" states. `title` is required; `description`, and the `icon` and `action` snippets, are optional.

```svelte
<EmptyState title="No applications yet" description="Add one to start tracking.">
	{#snippet action()}<Button onclick={add}>Add application</Button>{/snippet}
</EmptyState>
```

### Skeleton

A placeholder block. `shape`: `rect` (default) | `text` | `circle`. It's `aria-hidden="true"` and pulses only when motion is allowed. Size it with `class`.

```svelte
<Skeleton shape="text" class="w-1/2" />
<Skeleton shape="circle" class="size-10" />
```

### Stat

A non-interactive tile for one figure: a small muted `label`, a large tabular-nums `value` and an optional `detail` line. `tone` (a Badge tone, default `neutral`) colours the value; `neutral` keeps the body text colour.

```svelte
<Stat label="Cost" value="$0.04" />
<Stat label="Status" value="Failed" tone="danger" detail="after 3 attempts" />
```

### KeyValue

A `<dl>` of label and value pairs in two columns (`grid-cols-[max-content_1fr]`). `items: { key: string; value: string | Snippet }[]`; a snippet value renders richer content such as a Badge or a JsonView. Keys must be unique.

```svelte
{#snippet permission()}<Badge>read</Badge>{/snippet}
<KeyValue items={[{ key: 'Tool', value: 'lookup' }, { key: 'Permission', value: permission }]} />
```

### JsonView

Pretty-prints any JSON value (`JSON.stringify(value, null, 2)`) as the text of a `<pre>`. It never renders markup: a string such as `"<img src=x>"` shows as its characters. `label` is required and becomes the block's `aria-label`. The block scrolls past `max-h-80`, so it is focusable (`tabindex="0"`) with the focus-visible ring. For `null` or `undefined` it shows the `empty` text (default "None") in muted style instead.

```svelte
<JsonView value={step.input} label="Step input" />
<JsonView value={run.output} label="Run output" empty="No output yet" />
```

### Tabs

Horizontal tabs with automatic activation. Props: `label` (required, the tablist's `aria-label`), `items: { value: string; label: string; disabled?: boolean }[]`, `value` (bindable, defaults to the first enabled item) and a `children` snippet that receives the active value and renders its panel. The panel fades in when the value changes.

Keyboard: only the active tab is in the Tab order (roving `tabindex`). ArrowRight and ArrowLeft select the next or previous enabled tab and wrap at the ends; Home and End select the first and last enabled tab. Disabled tabs are skipped and ignore clicks. The tabpanel is focusable and labelled by the active tab.

```svelte
<Tabs label="Application" items={[{ value: 'overview', label: 'Overview' }, { value: 'notes', label: 'Notes' }]} bind:value={tab}>
	{#snippet children(active)}
		{#if active === 'overview'}<Overview />{:else}<Notes />{/if}
	{/snippet}
</Tabs>
```

### Dialog

A centred modal. Props: `open` (bindable, default `false`), `title` (required, the accessible name), optional `description` (linked with `aria-describedby`), `children`, an optional `footer` snippet for actions, `size`: `sm` | `md` (default) | `lg`, `closeOnBackdrop` (default `true`) and `onclose`.

- On open, focus moves to the first `[data-autofocus]` element in the panel, else the first focusable element (the header's Close button).
- Tab and Shift+Tab cycle inside the panel.
- Escape, the Close button or a backdrop click (when `closeOnBackdrop`) set `open = false` and call `onclose`. Setting `open = false` yourself (a Cancel button) doesn't call `onclose`.
- On close, focus returns to the element that had it before the dialog opened. Page scroll is locked while it's open.
- The backdrop fades; the panel fades and scales from 0.96.

```svelte
<Button onclick={() => (open = true)}>Archive</Button>
<Dialog bind:open title="Archive application" description="It leaves the board but keeps its history.">
	<Input label="Reason" data-autofocus bind:value={reason} />
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button onclick={archive}>Archive</Button>
	{/snippet}
</Dialog>
```

### ConfirmDialog

A small Dialog that asks before an action, with Cancel and a confirm button in its footer. Props: `open` (bindable), `title`, optional `description`, `confirmLabel` (default "Delete"), `pendingLabel` (default "Deleting…"), `tone`: `danger` (default) | `primary`, an optional `children` body, and `onconfirm: () => Promise<boolean>`. Confirm calls `onconfirm`; while it runs the confirm button is disabled and shows `pendingLabel`. Resolving `true` closes the dialog; `false` keeps it open (show the error yourself, e.g. with a toast). Cancel closes it without calling `onconfirm`. Keyboard, focus and Escape work as on Dialog.

```svelte
<ConfirmDialog
	bind:open={deleting}
	title="Delete memory"
	description="Waypoint will forget this. This can't be undone."
	onconfirm={remove}
/>
```

### Drawer

A full-height side panel with the same props, keyboard, focus, scroll-lock and close behaviour as Dialog, minus `size`, plus `side`: `right` (default) | `left`. It slides in from its edge over a fading backdrop. The body scrolls; the footer stays at the bottom.

```svelte
<Drawer bind:open side="right" title="Acme · Staff engineer">
	<ApplicationDetails {id} />
</Drawer>
```

### Toast

A service plus one renderer. `<Toaster />` is mounted once in `web/src/routes/+layout.svelte`; don't mount another. Call the `toast` service from anywhere:

- `toast.show({ title, description?, tone?, duration? })` returns the toast's id. `tone`: `neutral` (default) | `success` | `info` | `warning` | `danger`. `duration` is in ms, default `5000`; `0` keeps the toast until it's dismissed.
- `toast.success(title, opts?)` and `toast.error(title, opts?)` are shorthands for the `success` and `danger` tones.
- `toast.dismiss(id)` removes one toast, and `toast.items` is the reactive list.

Toasts stack at the bottom-right, newest at the bottom, each with a tone accent bar and a "Dismiss notification" button. They slide up and fade in, fade out, and the rest move into place. The region is `aria-live="polite"`; a `danger` toast is `role="alert"`, the others `role="status"`.

```ts
import { toast } from '#lib/components/ui';

toast.success('Application saved');
toast.error('Gmail sync failed', { description: 'Reconnect your account.', duration: 0 });
```

### Tooltip

A short, non-interactive label for a trigger. Props: `content`, `placement`: `top` (default) | `bottom` | `left` | `right`, `delay` in ms (default `300`) and a `children` snippet that renders the trigger.

The trigger snippet receives `{ 'aria-describedby' }`. Spread it onto the trigger so the trigger points at the tooltip while it's visible:

```svelte
<Tooltip content="Archive">
	{#snippet children(t)}
		<IconButton {...t} label="Archive application"><ArchiveIcon /></IconButton>
	{/snippet}
</Tooltip>
```

It shows after `delay` on pointer enter and at once when the trigger gets focus, and hides on pointer leave, blur or Escape. Escape on a visible tooltip stops there, so it doesn't also close a Dialog behind it. It's positioned with CSS next to its wrapper, with no collision handling, so keep the content short and leave room on the chosen side.

### Menu

A menu button: a trigger that opens a short list of actions. Props: `label` (required, the menu's `aria-label`), `items: { value: string; label: string; disabled?: boolean }[]`, `onselect(value)`, an optional `heading` shown as plain text at the top of the panel, `placement`: `bottom-end` (default) | `bottom-start`, and a `trigger` snippet.

The trigger snippet receives `{ id, 'aria-haspopup': 'menu', 'aria-expanded', 'aria-controls', onclick, onkeydown }`. Spread it onto a `Button` or `IconButton`, which bring the cursor and state classes:

```svelte
<Menu label="Move to" heading="Move to" items={targets} onselect={move}>
	{#snippet trigger(t)}
		<IconButton {...t} size="sm" label="Move Acme · Staff engineer"><MoreIcon /></IconButton>
	{/snippet}
</Menu>
```

- Click, Enter, Space or ArrowDown on the trigger opens it and focuses the first enabled item; ArrowUp opens it on the last.
- In the menu, ArrowDown and ArrowUp move between enabled items and wrap; Home and End jump to the ends. Disabled items are skipped and ignore clicks.
- Choosing an item calls `onselect`, closes the menu and returns focus to the trigger.
- Escape closes it and returns focus to the trigger without propagating, so a Dialog behind it stays open. Tab, or a pointer-down outside, closes it and leaves focus alone.
- The panel (`role="menu"`, `overlaySurface`, `py-1`) fades in and out with the `fast` token. Items are `h-8 px-3 text-sm` rows with hover, active, focus-visible and disabled states (`menuItem` in `styles.ts`).

Like Tooltip, the panel is `absolute` under its wrapper with no portal and no collision handling. Inside a scrolling container (`overflow: auto`) it scrolls with the content and can be clipped, so leave room below the trigger.

## Overlays

Dialog, Drawer, Toaster, Tooltip and Menu render where they're declared, with `position: fixed` (or `absolute` for Tooltip and Menu). There's no portal: the app has no transformed ancestors, and a portal complicates Svelte context and tests. Don't put an overlay inside an element with `transform`, `filter` or `contain`, which would trap its fixed positioning.

Layering, from `layers` in `styles.ts`:

| Layer | Class | Used by |
|---|---|---|
| `menu` | `z-30` | Menu panel. Below overlays; a Menu inside a Dialog stacks within the Dialog's layer |
| `overlay` | `z-40` | Dialog and Drawer (backdrop and panel) |
| `toast` | `z-50` | Toaster |
| `tooltip` | `z-50` | Tooltip |

Open Dialogs and Drawers register on a shared stack (`overlay.ts`). Escape closes only the topmost one, so a Dialog opened from a Drawer closes first. Page scroll (`document.body` `overflow`) is locked while any of them is open, and the previous value is restored when the last one closes. Overlay surfaces use `bg-surface border border-border rounded-lg shadow-lg` (`overlaySurface`).

## Tests

Component tests (`*.svelte.test.ts`) run in Vitest's `client` project under jsdom with Testing Library; other tests (`*.test.ts`) run in the `server` project under Node. `npm --prefix web test` runs both. jsdom can't evaluate hover or focus styles, so `states.svelte.test.ts` checks that each primitive renders its state and cursor classes, and the `/dev/primitives` gallery covers how they look.

jsdom has no Web Animations API, so `web/vitest-setup-client.ts` stubs `Element.prototype.animate` (finishing on the next microtask) and `getAnimations`. Transitions run to completion in tests: wait for an element to leave with `waitFor`. Test harnesses for overlays, tabs, tooltips and menus live in `web/src/lib/components/ui/__fixtures__/`.
