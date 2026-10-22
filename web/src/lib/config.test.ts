import { describe, expect, it } from 'vitest';
import { apiBaseUrl } from './config';

describe('apiBaseUrl', () => {
	it('uses VITE_API_URL when set', () => {
		expect(apiBaseUrl({ VITE_API_URL: 'http://127.0.0.1:9000' })).toBe('http://127.0.0.1:9000');
	});

	it('defaults to localhost API', () => {
		expect(apiBaseUrl({})).toBe('http://127.0.0.1:8000');
		expect(apiBaseUrl({ VITE_API_URL: '' })).toBe('http://127.0.0.1:8000');
	});
});
