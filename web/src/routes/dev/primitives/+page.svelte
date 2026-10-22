<script lang="ts">
	import {
		Badge,
		Button,
		Card,
		Checkbox,
		Dialog,
		Drawer,
		EmptyState,
		FileInput,
		IconButton,
		Input,
		Link,
		Menu,
		NavLink,
		Select,
		Skeleton,
		Tabs,
		Textarea,
		Tooltip,
		toast,
		type BadgeTone,
		type ButtonVariant,
		type DrawerSide,
		type ToastTone,
		type TooltipPlacement
	} from '#lib/components/ui';

	// Design-review gallery: every primitive in every variant and state. Reachable only by URL.

	const variants: ButtonVariant[] = ['primary', 'secondary', 'ghost', 'danger'];
	const tones: BadgeTone[] = ['neutral', 'primary', 'success', 'warning', 'danger', 'info'];
	const stages = [
		{ value: 'applied', label: 'Applied' },
		{ value: 'interview', label: 'Interviewing' },
		{ value: 'offer', label: 'Offer' },
		{ value: 'closed', label: 'Closed', disabled: true }
	];

	let clicks = $state(0);
	let company = $state('');
	let notes = $state('');
	let pickedFile = $state<string | null>(null);
	let stage = $state('');
	let remote = $state(true);

	const sections = [
		{ value: 'overview', label: 'Overview' },
		{ value: 'notes', label: 'Notes' },
		{ value: 'archived', label: 'Archived', disabled: true },
		{ value: 'emails', label: 'Emails' }
	];
	const toastTones: ToastTone[] = ['neutral', 'success', 'info', 'warning', 'danger'];
	const placements: TooltipPlacement[] = ['top', 'bottom', 'left', 'right'];
	const sides: DrawerSide[] = ['right', 'left'];

	let section = $state('overview');
	let dialogOpen = $state(false);
	let footerDialogOpen = $state(false);
	let drawers = $state<Record<DrawerSide, boolean>>({ right: false, left: false });
	let nestedDrawerOpen = $state(false);
	let nestedDialogOpen = $state(false);
	let closes = $state(0);

	const moveItems = [
		{ value: 'applied', label: 'Applied' },
		{ value: 'interviewing', label: 'Interviewing' },
		{ value: 'offer', label: 'Offer' },
		{ value: 'rejected', label: 'Rejected' }
	];
	const actionItems = [
		{ value: 'edit', label: 'Edit' },
		{ value: 'archive', label: 'Archive', disabled: true },
		{ value: 'delete', label: 'Delete' }
	];
	let picked = $state('nothing yet');
	let menuDialogOpen = $state(false);
</script>

<svelte:head>
	<title>Primitives · Waypoint</title>
</svelte:head>

{#snippet plus()}
	<svg class="size-4" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
		<path d="M8 3v10M3 8h10" stroke-linecap="round" />
	</svg>
{/snippet}

{#snippet close()}
	<svg class="size-4" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
		<path d="M4 4l8 8M12 4l-8 8" stroke-linecap="round" />
	</svg>
{/snippet}

{#snippet more()}
	<svg class="size-4" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
		<circle cx="3" cy="8" r="1.5" />
		<circle cx="8" cy="8" r="1.5" />
		<circle cx="13" cy="8" r="1.5" />
	</svg>
{/snippet}

{#snippet inbox()}
	<svg class="size-8" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
		<path
			d="M3 13h5l1 3h6l1-3h5M5 5h14l2 8v6H3v-6l2-8z"
			stroke-linecap="round"
			stroke-linejoin="round"
		/>
	</svg>
{/snippet}

{#snippet addAction()}
	<Button size="sm">{@render plus()} Add application</Button>
{/snippet}

<!-- The app shell supplies the page's <main> and its padding. -->
<div class="flex flex-col gap-12">
	<header class="flex flex-col gap-2">
		<h1 class="text-2xl font-semibold text-fg">Primitives</h1>
		<p class="text-sm text-fg-muted">
			Every UI primitive in every variant and state. The page follows the OS theme; set
			<code class="font-mono">data-theme</code> on <code class="font-mono">&lt;html&gt;</code> to force
			one.
		</p>
	</header>

	<section class="flex flex-col gap-4" aria-labelledby="button">
		<h2 id="button" class="text-lg font-semibold text-fg">Button</h2>
		{#each ['md', 'sm'] as const as size (size)}
			<div class="flex flex-wrap items-center gap-2">
				{#each variants as variant (variant)}
					<Button {variant} {size} onclick={() => clicks++}>{variant} {size}</Button>
				{/each}
				{#each variants as variant (variant)}
					<Button {variant} {size} disabled>{variant} disabled</Button>
				{/each}
			</div>
		{/each}
		<div class="flex items-center gap-2">
			<Button>{@render plus()} With icon</Button>
			<span class="text-sm text-fg-muted">Clicked {clicks} times</span>
		</div>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="icon-button">
		<h2 id="icon-button" class="text-lg font-semibold text-fg">IconButton</h2>
		{#each ['md', 'sm'] as const as size (size)}
			<div class="flex flex-wrap items-center gap-2">
				{#each variants as variant (variant)}
					<IconButton label="Close ({variant})" {variant} {size}>{@render close()}</IconButton>
				{/each}
				<IconButton label="Close (disabled)" {size} disabled>{@render close()}</IconButton>
			</div>
		{/each}
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="link">
		<h2 id="link" class="text-lg font-semibold text-fg">Link</h2>
		<p class="flex flex-wrap items-center gap-4 text-sm text-fg">
			<Link href="#link">Internal link</Link>
			<Link href="https://github.com" external>External link</Link>
			<Link href="#link" disabled>Disabled link</Link>
		</p>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="navlink">
		<h2 id="navlink" class="text-lg font-semibold text-fg">NavLink</h2>
		<nav aria-label="NavLink states" class="flex flex-wrap items-center gap-1">
			<NavLink href="#navlink" current>Current</NavLink>
			<NavLink href="#navlink">Default</NavLink>
			<NavLink href="#navlink" disabled>Disabled</NavLink>
		</nav>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="fields">
		<h2 id="fields" class="text-lg font-semibold text-fg">Input, Textarea, Select</h2>
		<div class="grid gap-6 sm:grid-cols-2">
			<Input label="Company" placeholder="Acme" hint="As written on the posting." bind:value={company} />
			<Input label="Role" value="Staff engineer" error="This role is already tracked." />
			<Input label="Disabled input" value="Locked" disabled />
			<Textarea label="Notes" placeholder="Anything worth remembering" bind:value={notes} />
			<Textarea label="Notes with error" value="Too short" error="Add at least 20 characters." />
			<Textarea label="Disabled textarea" value="Locked" disabled />
			<Select
				label="Stage"
				options={stages}
				placeholder="Pick a stage"
				hint="Where the application stands."
				bind:value={stage}
			/>
			<Select label="Stage with error" options={stages} error="Choose a stage." />
			<Select label="Disabled select" options={stages} value="offer" disabled />
		</div>
		<p class="text-sm text-fg-muted">
			Bound values: {company || '–'} · {notes || '–'} · {stage || '–'}
		</p>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="file-input">
		<h2 id="file-input" class="text-lg font-semibold text-fg">FileInput</h2>
		<div class="grid gap-6 sm:grid-cols-2">
			<FileInput
				label="PDF file"
				accept="application/pdf,.pdf"
				buttonLabel="Choose PDF"
				hint="At most 10 MB."
				onchange={(file) => (pickedFile = file?.name ?? null)}
			/>
			<FileInput label="File with error" error="Must be a PDF" />
			<FileInput label="Disabled file input" disabled />
		</div>
		<p class="text-sm text-fg-muted">Picked: {pickedFile ?? '–'}</p>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="checkbox">
		<h2 id="checkbox" class="text-lg font-semibold text-fg">Checkbox</h2>
		<div class="flex flex-wrap items-center gap-6">
			<Checkbox label="Remote only ({remote ? 'on' : 'off'})" bind:checked={remote} />
			<Checkbox label="Unchecked" />
			<Checkbox label="Disabled" disabled />
			<Checkbox label="Disabled, checked" checked disabled />
		</div>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="card">
		<h2 id="card" class="text-lg font-semibold text-fg">Card</h2>
		<div class="grid gap-4 sm:grid-cols-2">
			<Card>
				<p class="font-medium">Static card</p>
				<p class="text-sm text-fg-muted">A div. Not interactive.</p>
			</Card>
			<Card href="#card">
				<p class="font-medium">Link card</p>
				<p class="text-sm text-fg-muted">An anchor with hover, focus and active states.</p>
			</Card>
			<Card draggable>
				<p class="font-medium">Draggable card</p>
				<p class="text-sm text-fg-muted">A drag source: grab cursor, grabbing while held.</p>
			</Card>
		</div>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="menu">
		<h2 id="menu" class="text-lg font-semibold text-fg">Menu</h2>
		<div class="flex flex-wrap items-center gap-4 pb-48">
			<Menu label="Move to" heading="Move to" items={moveItems} onselect={(value) => (picked = value)}>
				{#snippet trigger(t)}
					<Button {...t} variant="secondary">Move</Button>
				{/snippet}
			</Menu>
			<Menu
				label="Actions"
				items={actionItems}
				placement="bottom-start"
				onselect={(value) => (picked = value)}
			>
				{#snippet trigger(t)}
					<IconButton {...t} label="More actions" variant="secondary">{@render more()}</IconButton>
				{/snippet}
			</Menu>
			<Button variant="secondary" onclick={() => (menuDialogOpen = true)}>Open dialog with menu</Button>
			<span class="text-sm text-fg-muted">Picked: {picked}</span>
		</div>
		<Dialog bind:open={menuDialogOpen} title="Menu in a dialog" size="sm">
			<div class="flex flex-col items-start gap-4 pb-40">
				<p>Escape closes the menu first, then this dialog.</p>
				<Menu label="Move to" heading="Move to" items={moveItems} onselect={(value) => (picked = value)}>
					{#snippet trigger(t)}
						<Button {...t} variant="secondary">Move</Button>
					{/snippet}
				</Menu>
			</div>
		</Dialog>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="badge">
		<h2 id="badge" class="text-lg font-semibold text-fg">Badge</h2>
		<div class="flex flex-wrap items-center gap-2">
			{#each tones as tone (tone)}
				<Badge {tone}>{tone}</Badge>
			{/each}
		</div>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="empty-state">
		<h2 id="empty-state" class="text-lg font-semibold text-fg">EmptyState</h2>
		<div class="grid gap-4 sm:grid-cols-2">
			<Card>
				<EmptyState
					title="No applications yet"
					description="Add the first role you applied to and Waypoint starts tracking it."
					icon={inbox}
					action={addAction}
				/>
			</Card>
			<Card>
				<EmptyState title="Nothing to review" />
			</Card>
		</div>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="skeleton">
		<h2 id="skeleton" class="text-lg font-semibold text-fg">Skeleton</h2>
		<div class="flex items-center gap-4">
			<Skeleton shape="circle" class="size-10" />
			<div class="flex flex-1 flex-col gap-2">
				<Skeleton shape="text" class="w-1/2" />
				<Skeleton shape="text" class="w-3/4" />
			</div>
			<Skeleton class="h-16 w-32" />
		</div>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="tabs">
		<h2 id="tabs" class="text-lg font-semibold text-fg">Tabs</h2>
		<Tabs label="Application sections" items={sections} bind:value={section}>
			{#snippet children(active)}
				<p class="text-sm text-fg-muted">The {active} panel. Use the arrow keys, Home and End.</p>
			{/snippet}
		</Tabs>
		<p class="text-sm text-fg-muted">Bound value: {section}</p>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="dialog">
		<h2 id="dialog" class="text-lg font-semibold text-fg">Dialog and Drawer</h2>
		<div class="flex flex-wrap items-center gap-2">
			<Button variant="secondary" onclick={() => (dialogOpen = true)}>Open dialog</Button>
			<Button variant="secondary" onclick={() => (footerDialogOpen = true)}>
				Open dialog with footer
			</Button>
			{#each sides as side (side)}
				<Button variant="secondary" onclick={() => (drawers[side] = true)}>
					Open {side} drawer
				</Button>
			{/each}
			<Button variant="secondary" onclick={() => (nestedDrawerOpen = true)}>
				Open drawer with dialog
			</Button>
			<span class="text-sm text-fg-muted">Closed {closes} times</span>
		</div>

		<Dialog
			bind:open={dialogOpen}
			title="Archive application"
			description="Archived applications leave the board but keep their history."
			onclose={() => closes++}
		>
			<Input label="Reason" placeholder="Optional" data-autofocus />
		</Dialog>

		<Dialog
			bind:open={footerDialogOpen}
			title="Withdraw application"
			size="sm"
			closeOnBackdrop={false}
			onclose={() => closes++}
		>
			<p>A backdrop click doesn't close this one. Use Escape, Close or Cancel.</p>
			{#snippet footer()}
				<Button variant="secondary" onclick={() => (footerDialogOpen = false)}>Cancel</Button>
				<Button variant="danger" onclick={() => (footerDialogOpen = false)}>Withdraw</Button>
			{/snippet}
		</Dialog>

		{#each sides as side (side)}
			<Drawer
				bind:open={drawers[side]}
				{side}
				title="Acme · Staff engineer"
				description="Opened from the {side}."
				onclose={() => closes++}
			>
				<div class="flex flex-col gap-4">
					<Input label="Company" value="Acme" />
					<Textarea label="Notes" />
				</div>
				{#snippet footer()}
					<Button onclick={() => (drawers[side] = false)}>Done</Button>
				{/snippet}
			</Drawer>
		{/each}

		<Drawer bind:open={nestedDrawerOpen} title="Nested overlays">
			<div class="flex flex-col items-start gap-4">
				<p>Escape closes the dialog first, then this drawer.</p>
				<Button onclick={() => (nestedDialogOpen = true)}>Open dialog</Button>
			</div>
			<Dialog bind:open={nestedDialogOpen} title="On top of the drawer" size="sm">
				<p>Page scroll stays locked until the drawer closes too.</p>
			</Dialog>
		</Drawer>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="toast">
		<h2 id="toast" class="text-lg font-semibold text-fg">Toast</h2>
		<div class="flex flex-wrap items-center gap-2">
			{#each toastTones as tone (tone)}
				<Button
					variant="secondary"
					onclick={() => toast.show({ title: `A ${tone} toast`, description: 'Gone in 5s.', tone })}
				>
					{tone}
				</Button>
			{/each}
			<Button
				variant="secondary"
				onclick={() =>
					toast.show({ title: 'Sticky toast', description: 'Stays until dismissed.', duration: 0 })}
			>
				sticky
			</Button>
		</div>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="tooltip">
		<h2 id="tooltip" class="text-lg font-semibold text-fg">Tooltip</h2>
		<div class="flex flex-wrap items-center gap-12 py-8 pl-16">
			{#each placements as placement (placement)}
				<Tooltip content="Tooltip on {placement}" {placement}>
					{#snippet children(t)}
						<IconButton {...t} label="Show {placement} tooltip" variant="secondary">
							{@render plus()}
						</IconButton>
					{/snippet}
				</Tooltip>
			{/each}
		</div>
	</section>
</div>
