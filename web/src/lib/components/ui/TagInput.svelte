<script lang="ts" module>
	export interface TagInputProps {
		label: string;
		/** Id of the text input; generated when omitted. */
		id?: string;
		hint?: string;
		error?: string;
		/** The tags, normalised. Bindable. */
		value?: string[];
		/** Most tags; adding stops there. Default 20. */
		max?: number;
		placeholder?: string;
		disabled?: boolean;
		class?: string;
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { motionDuration } from '#lib/motion.js';
	import Field from './Field.svelte';
	import Tag from './Tag.svelte';
	import { describedBy, tagInputBox } from './styles';
	import { normaliseTag } from './tags';

	const uid = $props.id();

	let {
		label,
		hint,
		error,
		id = uid,
		value = $bindable([]),
		max = 20,
		placeholder,
		disabled = false,
		class: className
	}: TagInputProps = $props();

	let draft = $state('');
	let input = $state<HTMLInputElement>();

	const full = $derived(value.length >= max);
	const shownHint = $derived(full ? `At most ${max} tags` : hint);

	/**
	 * Adds each comma-separated part of the draft that isn't already a tag, then clears the
	 * draft. A draft with a part that doesn't fit under `max` is kept, so nothing typed is lost.
	 */
	function commit() {
		let next = value;
		let refused = false;
		for (const part of draft.split(',')) {
			const tag = normaliseTag(part);
			if (!tag || next.includes(tag)) continue;
			if (next.length >= max) {
				refused = true;
				continue;
			}
			next = [...next, tag];
		}
		if (next !== value) value = next;
		if (!refused) draft = '';
	}

	function remove(tag: string) {
		value = value.filter((t) => t !== tag);
		input?.focus();
	}

	function onkeydown(event: KeyboardEvent) {
		if (event.key === 'Enter' || event.key === ',') {
			// Never submits an enclosing form, even with a blank draft.
			event.preventDefault();
			commit();
		} else if (event.key === 'Backspace' && draft === '' && value.length > 0) {
			event.preventDefault();
			value = value.slice(0, -1);
		}
	}
</script>

<Field {id} {label} hint={shownHint} {error}>
	<div class={[tagInputBox(!!error, disabled), className]}>
		{#each value as tag (tag)}
			<span
				class="inline-flex max-w-full"
				in:fade={{ duration: motionDuration('fast') }}
				out:fade={{ duration: motionDuration('fast') }}
			>
				<Tag label={tag} {disabled} onremove={() => remove(tag)} />
			</span>
		{/each}
		<input
			{id}
			bind:this={input}
			bind:value={draft}
			type="text"
			{placeholder}
			{disabled}
			{onkeydown}
			onblur={commit}
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy(id, shownHint, error)}
			class="h-8 min-w-24 flex-1 bg-transparent px-1 text-sm text-fg outline-none placeholder:text-fg-subtle disabled:cursor-not-allowed"
		/>
	</div>
</Field>
