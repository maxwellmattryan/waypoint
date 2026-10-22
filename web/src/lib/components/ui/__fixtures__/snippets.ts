import { createRawSnippet } from 'svelte';

/** A snippet rendering static HTML, for `children` and other snippet props in tests. */
export function html(markup: string) {
	return createRawSnippet(() => ({ render: () => markup }));
}

/** A snippet rendering text in a span. */
export function text(value: string) {
	return html(`<span>${value}</span>`);
}
