// Stack of open modal overlays (Dialog, Drawer). It locks page scroll while any overlay is open
// and sends Escape to the topmost one only, so a Dialog opened from a Drawer closes first.

type Entry = { onEscape: () => void };

const stack: Entry[] = [];
let savedOverflow = '';

function onkeydown(event: KeyboardEvent) {
	if (event.key !== 'Escape' || event.defaultPrevented) return;
	const top = stack[stack.length - 1];
	if (!top) return;
	event.preventDefault();
	top.onEscape();
}

/** Registers an open overlay. Returns the function that unregisters it. */
export function pushOverlay(onEscape: () => void): () => void {
	const entry: Entry = { onEscape };
	if (stack.length === 0) {
		savedOverflow = document.body.style.overflow;
		document.body.style.overflow = 'hidden';
		window.addEventListener('keydown', onkeydown);
	}
	stack.push(entry);

	return () => {
		const index = stack.indexOf(entry);
		if (index === -1) return;
		stack.splice(index, 1);
		if (stack.length === 0) {
			document.body.style.overflow = savedOverflow;
			window.removeEventListener('keydown', onkeydown);
		}
	};
}
