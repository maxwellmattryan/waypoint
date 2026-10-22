<script lang="ts">
	import {
		Badge,
		Button,
		Card,
		Checkbox,
		EmptyState,
		IconButton,
		Input,
		Link,
		Select,
		Skeleton,
		Textarea,
		type BadgeTone,
		type ButtonVariant
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
	let stage = $state('');
	let remote = $state(true);
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

<main class="mx-auto flex max-w-4xl flex-col gap-12 p-8">
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
		</div>
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
</main>
