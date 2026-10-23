<script lang="ts" module>
	import type { DataDeletion } from '#lib/api';

	export interface DataCardProps {
		/** Deletes all data; resolves the API's body, or `null` after it showed an error. */
		ondelete: (confirmation: string) => Promise<DataDeletion | null>;
	}
</script>

<script lang="ts">
	import { Button, toast } from '#lib/components/ui';
	import DashboardCard from './DashboardCard.svelte';
	import DeleteAllDataDialog from './DeleteAllDataDialog.svelte';

	let { ondelete }: DataCardProps = $props();

	let open = $state(false);

	async function submit(confirmation: string): Promise<boolean> {
		const deleted = await ondelete(confirmation);
		if (!deleted) return false;
		toast.success('All data deleted');
		if (deleted.google === 'not_revoked') {
			toast.show({
				tone: 'warning',
				duration: 0,
				title: "Google didn't confirm the revocation",
				description: 'Remove Waypoint at myaccount.google.com/permissions.'
			});
		}
		return true;
	}
</script>

<DashboardCard title="Data">
	<p class="text-sm text-fg-muted">
		Delete every application, resume, run, job and model-call log, and revoke Waypoint's Google
		access. This can't be undone.
	</p>
	<div><Button variant="danger" onclick={() => (open = true)}>Delete all data…</Button></div>
</DashboardCard>

<DeleteAllDataDialog bind:open {submit} />
