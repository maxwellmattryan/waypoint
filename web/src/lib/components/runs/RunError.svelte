<script lang="ts">
	let { message }: { message: string } = $props();
</script>

<p
	class="rounded-md bg-danger-soft p-3 text-sm break-words whitespace-pre-wrap text-danger-soft-fg"
>
	{message}
</p>
