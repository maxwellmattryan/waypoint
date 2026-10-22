import { describe, expect, it } from 'vitest';
import { defaultName, formatBytes, isPdfFile } from './format';

describe('formatBytes', () => {
	it.each([
		[0, '0 B'],
		[512, '512 B'],
		[1023, '1023 B'],
		[1024, '1 KB'],
		[9.5 * 1024, '9.5 KB'],
		[182 * 1024 + 300, '182 KB'],
		[1024 * 1024, '1 MB'],
		[1.25 * 1024 * 1024, '1.3 MB'],
		[10 * 1024 * 1024, '10 MB']
	])('%d bytes is %s', (bytes, want) => {
		expect(formatBytes(bytes)).toBe(want);
	});
});

describe('defaultName', () => {
	it.each([
		['resume.pdf', 'resume'],
		['Backend CV.PDF', 'Backend CV'],
		['notes.pdf.txt', 'notes.pdf.txt'],
		['a.pdf.pdf', 'a.pdf'],
		['plain', 'plain']
	])('%s becomes %s', (filename, want) => {
		expect(defaultName(filename)).toBe(want);
	});
});

describe('isPdfFile', () => {
	it.each([
		[{ name: 'cv.pdf', type: '' }, true],
		[{ name: 'CV.PDF', type: '' }, true],
		[{ name: 'cv', type: 'application/pdf' }, true],
		[{ name: 'cv.docx', type: 'application/msword' }, false],
		[{ name: 'cv.pdf.exe', type: '' }, false]
	])('%o is a PDF: %s', (file, want) => {
		expect(isPdfFile(file)).toBe(want);
	});
});
