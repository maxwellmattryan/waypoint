<script lang="ts">
	import CloseIcon from './CloseIcon.svelte';
	import IconButton from './IconButton.svelte';

	// Title, optional description and Close button shared by Dialog and Drawer. Internal.
	let {
		title,
		titleId,
		description,
		descriptionId,
		onclose
	}: {
		title: string;
		titleId: string;
		description?: string;
		descriptionId: string;
		onclose: () => void;
	} = $props();
</script>

<header class="flex items-start justify-between gap-4 p-4">
	<div class="flex min-w-0 flex-col gap-1 pt-2">
		<h2 id={titleId} class="text-base font-semibold text-fg">{title}</h2>
		{#if description}
			<p id={descriptionId} class="text-sm text-fg-muted">{description}</p>
		{/if}
	</div>
	<IconButton label="Close" onclick={onclose}><CloseIcon /></IconButton>
</header>
