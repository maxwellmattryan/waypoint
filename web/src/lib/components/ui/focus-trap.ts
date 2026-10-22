import type { Attachment } from 'svelte/attachments';

const focusable =
	'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusables(root: HTMLElement): HTMLElement[] {
	return [...root.querySelectorAll<HTMLElement>(focusable)];
}

/**
 * Keeps keyboard focus inside an overlay panel. Use as `{@attach trapFocus}` on an element with
 * `tabindex="-1"`. On attach it focuses the first `[data-autofocus]` element, else the first
 * focusable descendant, else the panel. Tab and Shift+Tab wrap at the ends. On cleanup, focus
 * returns to the element that had it before.
 */
export const trapFocus: Attachment<HTMLElement> = (panel) => {
	const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;

	const initial =
		panel.querySelector<HTMLElement>('[data-autofocus]') ?? focusables(panel)[0] ?? panel;
	initial.focus();

	panel.setAttribute('data-focus-trap', '');

	function onkeydown(event: KeyboardEvent) {
		if (event.key !== 'Tab') return;
		// A nested trap (a Dialog inside a Drawer) handles its own Tab.
		if (event.target instanceof Element && event.target.closest('[data-focus-trap]') !== panel) {
			return;
		}
		const items = focusables(panel);
		if (items.length === 0) {
			event.preventDefault();
			panel.focus();
			return;
		}
		const first = items[0];
		const last = items[items.length - 1];
		const active = document.activeElement;
		if (event.shiftKey && (active === first || active === panel || !panel.contains(active))) {
			event.preventDefault();
			last.focus();
		} else if (!event.shiftKey && (active === last || !panel.contains(active))) {
			event.preventDefault();
			first.focus();
		}
	}

	panel.addEventListener('keydown', onkeydown);
	return () => {
		panel.removeEventListener('keydown', onkeydown);
		if (previous?.isConnected) previous.focus();
	};
};
