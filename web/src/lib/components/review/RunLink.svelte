<script lang="ts">
	import { resolve } from '$app/paths';
	import { Link } from '#lib/components/ui';

	/** The "Run #N" link to a run's inspector page. */
	let { runId }: { runId: number } = $props();

	const href = $derived(resolve('/runs/[id]', { id: String(runId) }));
</script>

<Link {href} class="text-xs">Run #{runId}</Link>
