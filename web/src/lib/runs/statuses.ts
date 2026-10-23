import type { Permission, RunStatus, StepStatus, ToolCallStatus } from '#lib/api';
import type { BadgeTone } from '#lib/components/ui';

/** A badge's text and tone. */
export interface BadgeSpec {
	label: string;
	tone: BadgeTone;
}

export const runStatusBadge = {
	queued: { label: 'Queued', tone: 'neutral' },
	running: { label: 'Running', tone: 'info' },
	paused: { label: 'Paused', tone: 'warning' },
	succeeded: { label: 'Succeeded', tone: 'success' },
	failed: { label: 'Failed', tone: 'danger' }
} as const satisfies Record<RunStatus, BadgeSpec>;

export const stepStatusBadge = {
	pending: { label: 'Pending', tone: 'neutral' },
	running: { label: 'Running', tone: 'info' },
	succeeded: { label: 'Succeeded', tone: 'success' },
	failed: { label: 'Failed', tone: 'danger' }
} as const satisfies Record<StepStatus, BadgeSpec>;

export const toolCallStatusBadge = {
	running: { label: 'Running', tone: 'info' },
	succeeded: { label: 'Succeeded', tone: 'success' },
	failed: { label: 'Failed', tone: 'danger' },
	invalid_input: { label: 'Invalid input', tone: 'warning' },
	invalid_output: { label: 'Invalid output', tone: 'warning' },
	unknown_tool: { label: 'Unknown tool', tone: 'danger' }
} as const satisfies Record<ToolCallStatus, BadgeSpec>;

/** Permission levels: `read` is neutral, `write` a warning, `external` leaves Waypoint. */
export const permissionBadge = {
	read: { label: 'read', tone: 'neutral' },
	write: { label: 'write', tone: 'warning' },
	external: { label: 'external', tone: 'danger' }
} as const satisfies Record<Permission, BadgeSpec>;

/** The badge for a tool call's permission; `null` (an unknown tool) is "unknown". */
export function permissionSpec(permission: Permission | null): BadgeSpec {
	return permission === null ? { label: 'unknown', tone: 'neutral' } : permissionBadge[permission];
}
