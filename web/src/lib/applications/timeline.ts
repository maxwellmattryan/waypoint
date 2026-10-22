import type { ApplicationEvent, ApplicationStatus } from '#lib/api';
import { statusLabel } from './statuses';

const label = (status: ApplicationStatus | null) => (status ? statusLabel[status] : 'unknown');

/** The one-line description of a timeline entry, e.g. "Status changed from Applied to Offer". */
export function describeEvent(event: ApplicationEvent): string {
	const stage = event.stage_name ?? '';
	switch (event.kind) {
		case 'created':
			return event.to_status ? `Added as ${statusLabel[event.to_status]}` : 'Added';
		case 'status_changed':
			return `Status changed from ${label(event.from_status)} to ${label(event.to_status)}`;
		case 'stage_added':
			return `Stage added: ${stage}`;
		case 'stage_completed':
			return `Stage completed: ${stage}`;
		case 'stage_reopened':
			return `Stage reopened: ${stage}`;
		case 'stage_removed':
			return `Stage removed: ${stage}`;
		case 'note':
			return 'Note';
		case 'rejection_feedback':
			return 'Rejection feedback';
	}
}
