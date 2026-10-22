/** The largest PDF the API accepts: 10 MiB, as `MAX_PDF_BYTES` in api/src/resumes.rs. */
export const MAX_PDF_BYTES = 10 * 1024 * 1024;

/** The message for a file over {@link MAX_PDF_BYTES}, from the client check or a 413. */
export const TOO_LARGE = 'At most 10 MB';

const KB = 1024;
const MB = 1024 * KB;

function scaled(value: number, unit: string): string {
	const shown = value < 10 ? Math.round(value * 10) / 10 : Math.round(value);
	return `${shown} ${unit}`;
}

/** A byte count for people: "512 B", "9.5 KB", "182 KB", "1.2 MB" (one decimal under 10). */
export function formatBytes(bytes: number): string {
	if (bytes < KB) return `${bytes} B`;
	if (bytes < MB) return scaled(bytes / KB, 'KB');
	return scaled(bytes / MB, 'MB');
}

/** "Used by 1 application" / "Used by 3 applications". */
export function usedBy(count: number): string {
	return `Used by ${count} ${count === 1 ? 'application' : 'applications'}`;
}

/** The default version name for a file: its name without a trailing `.pdf`, any case. */
export function defaultName(filename: string): string {
	return filename.replace(/\.pdf$/i, '');
}

/** Whether a picked file looks like a PDF: a `.pdf` name or the `application/pdf` type. */
export function isPdfFile(file: Pick<File, 'name' | 'type'>): boolean {
	return /\.pdf$/i.test(file.name) || file.type === 'application/pdf';
}
