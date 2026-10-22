<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export interface TabItem {
		value: string;
		label: string;
		disabled?: boolean;
	}

	export interface TabsProps {
		/** The tablist's accessible name. */
		label: string;
		items: TabItem[];
		/** The active tab's value. Bindable; defaults to the first enabled item. */
		value?: string;
		/** Renders the panel for the active value. */
		children: Snippet<[string]>;
		class?: string;
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
	import { focusRing, interactive } from './styles';

	let { label, items, value = $bindable(), children, class: className }: TabsProps = $props();

	const id = $props.id();
	const panelId = `${id}-panel`;
	const tabId = (index: number) => `${id}-tab-${index}`;

	const current = $derived(value ?? items.find((item) => !item.disabled)?.value);
	const activeIndex = $derived(items.findIndex((item) => item.value === current));

	// Write the default back so a bound `value` reflects the shown tab.
	$effect.pre(() => {
		if (value === undefined && current !== undefined) value = current;
	});

	function select(index: number) {
		const item = items[index];
		if (!item || item.disabled) return;
		value = item.value;
	}

	function onkeydown(event: KeyboardEvent) {
		const enabled = items.flatMap((item, index) => (item.disabled ? [] : [index]));
		if (enabled.length === 0) return;
		const at = enabled.indexOf(activeIndex);
		const n = enabled.length;
		let next: number;
		switch (event.key) {
			case 'ArrowRight':
				next = enabled[(at + 1) % n];
				break;
			case 'ArrowLeft':
				next = enabled[at <= 0 ? n - 1 : at - 1];
				break;
			case 'Home':
				next = enabled[0];
				break;
			case 'End':
				next = enabled[n - 1];
				break;
			default:
				return;
		}
		event.preventDefault();
		select(next);
		document.getElementById(tabId(next))?.focus();
	}
</script>

<div class={className}>
	<div
		role="tablist"
		aria-label={label}
		tabindex="-1"
		class="flex gap-1 border-b border-border"
		{onkeydown}
	>
		{#each items as item, index (item.value)}
			{@const active = index === activeIndex}
			<button
				type="button"
				role="tab"
				id={tabId(index)}
				aria-selected={active}
				aria-controls={panelId}
				tabindex={active ? 0 : -1}
				disabled={item.disabled}
				class={[
					'-mb-px h-10 border-b-2 px-3 text-sm font-medium',
					interactive,
					focusRing,
					active
						? 'border-primary text-fg enabled:hover:border-primary-hover enabled:active:border-primary-active'
						: 'border-transparent text-fg-muted enabled:hover:border-border-strong enabled:hover:text-fg enabled:active:text-fg'
				]}
				onclick={() => select(index)}
			>
				{item.label}
			</button>
		{/each}
	</div>
	{#key current}
		<div
			role="tabpanel"
			id={panelId}
			aria-labelledby={activeIndex >= 0 ? tabId(activeIndex) : undefined}
			tabindex="0"
			class={['rounded-sm pt-4', focusRing]}
			in:fade={{ duration: motionDuration('fast'), easing: easings.enter }}
		>
			{#if current !== undefined}{@render children(current)}{/if}
		</div>
	{/key}
</div>
