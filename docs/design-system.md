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
| `{neutral,primary,success,warning,danger,info}-soft` and `-soft-fg` | Badge background and text pairs |

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

## Tests

Component tests (`*.svelte.test.ts`) run in Vitest's `client` project under jsdom with Testing Library; other tests (`*.test.ts`) run in the `server` project under Node. `npm --prefix web test` runs both. jsdom can't evaluate hover or focus styles, so `states.svelte.test.ts` checks that each primitive renders its state and cursor classes, and the `/dev/primitives` gallery covers how they look.
