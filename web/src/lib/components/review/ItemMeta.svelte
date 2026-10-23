<script lang="ts">
	import { formatDateTime } from '#lib/applications/format.js';
	import RunLink from './RunLink.svelte';

	/** A queue or log entry's subject line: what it is, its run and when it was created. */
	let { name, runId, createdAt }: { name: string; runId: number; createdAt: string } = $props();
</script>

<p class="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
	<span class="font-mono font-semibold break-all text-fg">{name}</span>
	<RunLink {runId} />
	<time datetime={createdAt} class="text-xs text-fg-muted tabular-nums">
		{formatDateTime(createdAt)}
	</time>
</p>
