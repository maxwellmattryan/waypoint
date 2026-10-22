<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { MenuPlacement } from './styles';

	export interface MenuItem {
		value: string;
		label: string;
		disabled?: boolean;
	}

	/** Spread onto the trigger (a `Button` or `IconButton`) so it opens and points at the menu. */
	export type MenuTriggerProps = {
		id: string;
		'aria-haspopup': 'menu';
		'aria-expanded': boolean;
		'aria-controls': string | undefined;
		onclick: (event: MouseEvent) => void;
		onkeydown: (event: KeyboardEvent) => void;
	};

	export interface MenuProps {
		/** The menu's accessible name, e.g. "Move to". */
		label: string;
		items: MenuItem[];
		/** Called with the chosen item's value. The menu then closes and refocuses the trigger. */
		onselect: (value: string) => void;
		/** Optional non-interactive text at the top of the panel. */
		heading?: string;
		/** Default `bottom-end`. */
		placement?: MenuPlacement;
		/** Renders the trigger. Spread the argument onto it. */
		trigger: Snippet<[MenuTriggerProps]>;
	}
</script>

<script lang="ts">
	import { tick } from 'svelte';
	import { fade } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
	import { focusRing, interactive, layers, menuItem, menuPlacements, overlaySurface } from './styles';

	let { label, items, onselect, heading, placement = 'bottom-end', trigger }: MenuProps = $props();

	const id = $props.id();
	const triggerId = `${id}-trigger`;
	const menuId = `${id}-menu`;

	let open = $state(false);
	let wrapper = $state<HTMLElement>();
	let panel = $state<HTMLElement>();

	function enabledItems(): HTMLElement[] {
		return panel ? [...panel.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)')] : [];
	}

	function focusItem(index: number) {
		const enabled = enabledItems();
		if (enabled.length === 0) return;
		enabled[(index + enabled.length) % enabled.length].focus();
	}

	async function show(focus: 'first' | 'last') {
		open = true;
		await tick();
		focusItem(focus === 'first' ? 0 : -1);
	}

	function close(returnFocus: boolean) {
		open = false;
		if (returnFocus) document.getElementById(triggerId)?.focus();
	}

	function onTriggerClick(event: MouseEvent) {
		if (!open) void show('first');
		// A click the keyboard synthesised (detail 0) right after opening doesn't toggle it shut.
		else if (event.detail > 0) close(false);
	}

	function onTriggerKeydown(event: KeyboardEvent) {
		switch (event.key) {
			case 'Enter':
			case ' ':
			case 'ArrowDown':
				event.preventDefault();
				void show('first');
				return;
			case 'ArrowUp':
				event.preventDefault();
				void show('last');
				return;
		}
	}

	function onMenuKeydown(event: KeyboardEvent) {
		const enabled = enabledItems();
		const at = enabled.indexOf(document.activeElement as HTMLElement);
		switch (event.key) {
			case 'ArrowDown':
				event.preventDefault();
				focusItem(at + 1);
				return;
			case 'ArrowUp':
				event.preventDefault();
				focusItem(at <= 0 ? -1 : at - 1);
				return;
			case 'Home':
				event.preventDefault();
				focusItem(0);
				return;
			case 'End':
				event.preventDefault();
				focusItem(-1);
				return;
			case 'Escape':
				// Keep Escape from also closing a Dialog or Drawer behind the menu.
				event.preventDefault();
				event.stopPropagation();
				close(true);
				return;
			case 'Tab':
				close(false);
				return;
		}
	}

	function choose(item: MenuItem) {
		if (item.disabled) return;
		onselect(item.value);
		close(true);
	}

	$effect(() => {
		if (!open) return;
		const onpointerdown = (event: PointerEvent) => {
			if (!wrapper?.contains(event.target as Node)) close(false);
		};
		document.addEventListener('pointerdown', onpointerdown, true);
		return () => document.removeEventListener('pointerdown', onpointerdown, true);
	});
</script>

<div class="relative inline-flex" bind:this={wrapper}>
	{@render trigger({
		id: triggerId,
		'aria-haspopup': 'menu',
		'aria-expanded': open,
		'aria-controls': open ? menuId : undefined,
		onclick: onTriggerClick,
		onkeydown: onTriggerKeydown
	})}
	{#if open}
		<div
			bind:this={panel}
			role="menu"
			id={menuId}
			aria-label={label}
			tabindex="-1"
			class={['absolute min-w-40 py-1 outline-none', overlaySurface, layers.menu, menuPlacements[placement]]}
			onkeydown={onMenuKeydown}
			transition:fade={{ duration: motionDuration('fast'), easing: easings.standard }}
		>
			{#if heading}
				<div aria-hidden="true" class="px-3 py-1 text-xs font-medium text-fg-muted">{heading}</div>
			{/if}
			{#each items as item (item.value)}
				<button
					type="button"
					role="menuitem"
					tabindex="-1"
					disabled={item.disabled}
					class={[menuItem, interactive, focusRing]}
					onclick={() => choose(item)}
				>
					{item.label}
				</button>
			{/each}
		</div>
	{/if}
</div>
