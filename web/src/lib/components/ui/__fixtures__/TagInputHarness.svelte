<script lang="ts">
	import { TagInput } from '../index';

	let {
		initial = [],
		max,
		onsubmit
	}: { initial?: string[]; max?: number; onsubmit?: () => void } = $props();

	// svelte-ignore state_referenced_locally
	let tags = $state([...initial]);
</script>

<form
	onsubmit={(event) => {
		event.preventDefault();
		onsubmit?.();
	}}
>
	<TagInput label="Tags" bind:value={tags} {max} />
</form>
<output data-testid="tags">{JSON.stringify(tags)}</output>
