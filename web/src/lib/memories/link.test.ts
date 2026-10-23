import { describe, expect, it } from 'vitest';
import { memoryHref, parseMemoryParam } from './link';

describe('memory links', () => {
	it('links to the memories page with the memory parameter', () => {
		expect(memoryHref(42)).toBe('/memories?memory=42');
	});

	it('parses only positive safe integers', () => {
		expect(parseMemoryParam('42')).toBe(42);
		expect(parseMemoryParam('9007199254740991')).toBe(9007199254740991);
		for (const raw of [
			null,
			'',
			' 4',
			'0',
			'07',
			'-3',
			'1.5',
			'1e3',
			'abc',
			'4abc',
			'9007199254740992'
		]) {
			expect(parseMemoryParam(raw), String(raw)).toBeNull();
		}
	});
});
