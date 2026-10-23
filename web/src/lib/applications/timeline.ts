import type { ApplicationEvent, ApplicationStatus } from '#lib/api';
import { formatDate } from './format';
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
		case 'interview_scheduled':
			return 'Interview added to Google Calendar';
	}
}

/** An email's subject as shown: trimmed, or "(no subject)" when blank. */
export function emailSubject(subject: string): string {
	return subject.trim() || '(no subject)';
}

/** An email's sender as shown: the display name, else the address, else null. */
export function emailSender(fromName: string | null, fromAddress: string | null): string | null {
	return fromName?.trim() || fromAddress?.trim() || null;
}

/**
 * The line naming the email that caused an event, e.g.
 * `From email: "Update on your application" · Vandelay Industries · Sep 10, 2026`, or null for
 * an event no email caused. A missing sender or date is left out.
 */
export function describeSource(event: ApplicationEvent): string | null {
	const source = event.source_email;
	if (!source) return null;
	const parts = [`From email: "${emailSubject(source.subject)}"`];
	const sender = emailSender(source.from_name, source.from_address);
	if (sender) parts.push(sender);
	if (source.sent_at) parts.push(formatDate(source.sent_at));
	return parts.join(' · ');
}
