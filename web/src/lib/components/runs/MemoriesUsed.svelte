<script lang="ts">
	import { EmptyState } from '#lib/components/ui';
</script>

<!-- Memories arrive in MVP 4; until then nothing is fetched and the section stays empty. -->
<section class="flex flex-col gap-4" aria-labelledby="memories-heading">
	<h2 id="memories-heading" class="text-lg font-semibold text-fg">Memories used</h2>
	<EmptyState
		title="No memories used"
		description="Memories the run reads will show here."
		class="rounded-lg border border-dashed border-border"
	/>
</section>
