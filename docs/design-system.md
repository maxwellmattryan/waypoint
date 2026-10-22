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

A square button. `label` is required and becomes the `aria-label`. The icon is the `children`. `variant` defaults to `ghost`; `size` and `disabled` work as on Button.

```svelte
<IconButton label="Close" onclick={close}><CloseIcon /></IconButton>
```

### Link

An inline `<a>`. `external` adds `target="_blank"` and `rel="noopener noreferrer"`. `disabled` drops the `href`, sets `aria-disabled="true"` and `tabindex="-1"`, and shows `cursor-not-allowed`. Pass internal routes through `resolve()` from `$app/paths`.

```svelte
<Link href={resolve('/jobs')}>Jobs</Link>
<Link href="https://example.com" external>Posting</Link>
```

### Input, Textarea, Select

Form controls wrapped in a shared Field: label, optional `hint`, optional `error`. `value` is bindable. The label is linked to the control by id (pass `id` or let it be generated). With `error`, the control gets `aria-invalid="true"`, a danger border, and `aria-describedby` pointing at the error text; the hint is linked the same way. Text controls keep the text cursor; Select is `cursor-pointer`. Textarea `rows` defaults to 3.

Select takes `options: { value: string; label: string; disabled?: boolean }[]` and an optional `placeholder`, rendered as a disabled, empty-value first option.

```svelte
<Input label="Company" hint="As on the posting" bind:value={company} />
<Textarea label="Notes" bind:value={notes} />
<Select label="Stage" options={stages} placeholder="Pick a stage" bind:value={stage} error={stageError} />
```

### Checkbox

A native checkbox with a required `label`. `checked` is bindable. The input and label are `cursor-pointer` (`cursor-not-allowed` when disabled), and clicking the label toggles it.

```svelte
<Checkbox label="Remote only" bind:checked={remote} />
```

### Card

A surface with border, radius, small shadow and `p-4`. Without `href` it's a non-interactive `<div>`. With `href` it's an `<a>` with hover, focus-visible and active states.

```svelte
<Card>Summary</Card>
<Card href={resolve('/jobs/[id]', { id })}>Acme · Staff engineer</Card>
```

### Badge

`tone`: `neutral` (default) | `primary` | `success` | `warning` | `danger` | `info`. A non-interactive `<span>`.

```svelte
<Badge tone="success">Offer</Badge>
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

## Overlays

Dialog, Drawer, Toaster and Tooltip render where they're declared, with `position: fixed` (or `absolute` for Tooltip). There's no portal: the app has no transformed ancestors, and a portal complicates Svelte context and tests. Don't put an overlay inside an element with `transform`, `filter` or `contain`, which would trap its fixed positioning.

Layering, from `layers` in `styles.ts`:

| Layer | Class | Used by |
|---|---|---|
| `overlay` | `z-40` | Dialog and Drawer (backdrop and panel) |
| `toast` | `z-50` | Toaster |
| `tooltip` | `z-50` | Tooltip |

Open Dialogs and Drawers register on a shared stack (`overlay.ts`). Escape closes only the topmost one, so a Dialog opened from a Drawer closes first. Page scroll (`document.body` `overflow`) is locked while any of them is open, and the previous value is restored when the last one closes. Overlay surfaces use `bg-surface border border-border rounded-lg shadow-lg` (`overlaySurface`).

## Tests

Component tests (`*.svelte.test.ts`) run in Vitest's `client` project under jsdom with Testing Library; other tests (`*.test.ts`) run in the `server` project under Node. `npm --prefix web test` runs both. jsdom can't evaluate hover or focus styles, so `states.svelte.test.ts` checks that each primitive renders its state and cursor classes, and the `/dev/primitives` gallery covers how they look.

jsdom has no Web Animations API, so `web/vitest-setup-client.ts` stubs `Element.prototype.animate` (finishing on the next microtask) and `getAnimations`. Transitions run to completion in tests: wait for an element to leave with `waitFor`. Test harnesses for overlays, tabs and tooltips live in `web/src/lib/components/ui/__fixtures__/`.
