import { describe, expect, it } from 'vitest';
import { apiBaseUrl, apiTarget } from './config';

describe('apiTarget', () => {
	it('uses VITE_API_URL when set', () => {
		expect(apiTarget({ VITE_API_URL: 'http://127.0.0.1:9000' })).toBe('http://127.0.0.1:9000');
	});

	it('defaults to localhost API', () => {
		expect(apiTarget({})).toBe('http://127.0.0.1:8000');
		expect(apiTarget({ VITE_API_URL: '' })).toBe('http://127.0.0.1:8000');
	});
});

describe('apiBaseUrl', () => {
	it('is the page origin plus the /api prefix', () => {
		expect(apiBaseUrl('http://127.0.0.1:5173')).toBe('http://127.0.0.1:5173/api');
	});
});
