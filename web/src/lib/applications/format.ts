const mediumDate = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' });
const mediumDateTime = new Intl.DateTimeFormat(undefined, {
	dateStyle: 'medium',
	timeStyle: 'short'
});

/** An API timestamp as a medium date in the user's locale, e.g. "Oct 22, 2026". */
export function formatDate(iso: string): string {
	return mediumDate.format(new Date(iso));
}

/** An API timestamp as a date and time in the user's locale, e.g. "Oct 22, 2026, 3:04 PM". */
export function formatDateTime(iso: string): string {
	return mediumDateTime.format(new Date(iso));
}

/** Whether `url` is an absolute http or https URL, the only kind rendered as a link. */
export function isWebUrl(url: string): boolean {
	try {
		const { protocol } = new URL(url);
		return protocol === 'http:' || protocol === 'https:';
	} catch {
		return false;
	}
}
