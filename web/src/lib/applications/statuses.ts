import type { ApplicationStatus } from '#lib/api';
import type { BadgeTone, SelectOption } from '#lib/components/ui';

/** Every status, in board order. */
export const STATUSES: readonly ApplicationStatus[] = [
	'sourced',
	'applied',
	'interviewing',
	'offer',
	'rejected'
];

export const statusLabel: Record<ApplicationStatus, string> = {
	sourced: 'Sourced',
	applied: 'Applied',
	interviewing: 'Interviewing',
	offer: 'Offer',
	rejected: 'Rejected'
};

export const statusTone: Record<ApplicationStatus, BadgeTone> = {
	sourced: 'neutral',
	applied: 'info',
	interviewing: 'primary',
	offer: 'success',
	rejected: 'danger'
};

/** The statuses as Select options, in board order. */
export const statusOptions: SelectOption[] = STATUSES.map((value) => ({
	value,
	label: statusLabel[value]
}));

/** Whether a string is one of the five statuses. */
export function isStatus(value: string): value is ApplicationStatus {
	return (STATUSES as readonly string[]).includes(value);
}
