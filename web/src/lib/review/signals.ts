import type { MemorySignal } from '#lib/api';
import type { BadgeTone } from '#lib/components/ui';

/** A signal's badge: what the user decided, with "Approved with edits" for an edited approval. */
export function signalLabel(signal: MemorySignal): { label: string; tone: BadgeTone } {
	switch (signal.kind) {
		case 'approval':
			return (signal.changes?.length ?? 0) > 0
				? { label: 'Approved with edits', tone: 'info' }
				: { label: 'Approved', tone: 'success' };
		case 'rejection':
			return { label: 'Rejected', tone: 'danger' };
		case 'dismissal':
			return { label: 'Dismissed', tone: 'neutral' };
	}
}

/** A diff value as compact JSON. */
export function compactJson(value: unknown): string {
	return JSON.stringify(value) ?? 'null';
}
