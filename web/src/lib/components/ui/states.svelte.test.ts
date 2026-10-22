import { render } from '@testing-library/svelte';
import type { Component } from 'svelte';
import { describe, expect, it } from 'vitest';
import {
	Badge,
	Button,
	Card,
	Checkbox,
	EmptyState,
	IconButton,
	Input,
	Link,
	Select,
	Skeleton,
	Tabs,
	Textarea,
	Tooltip
} from './index';
import { text } from './__fixtures__/snippets';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyComponent = Component<any>;

type Row = {
	name: string;
	component: AnyComponent;
	props: Record<string, unknown>;
	/** The element that owns the state classes. */
	find: (container: HTMLElement) => Element | null;
	/** Whether the element gets `cursor-pointer` (text controls keep the text cursor). */
	pointer: boolean;
	/** Props that render the disabled state. */
	disabledProps: Record<string, unknown>;
	/** The class the disabled state must carry. */
	disabledClass: string;
};

const options = [{ value: 'a', label: 'A' }];
const tabItems = [
	{ value: 'a', label: 'A' },
	{ value: 'b', label: 'B' }
];

const interactiveRows: Row[] = [
	{
		name: 'Button',
		component: Button,
		props: { children: text('Go') },
		find: (c) => c.querySelector('button'),
		pointer: true,
		disabledProps: { disabled: true },
		disabledClass: 'disabled:cursor-not-allowed'
	},
	{
		name: 'IconButton',
		component: IconButton,
		props: { label: 'Close', children: text('x') },
		find: (c) => c.querySelector('button'),
		pointer: true,
		disabledProps: { disabled: true },
		disabledClass: 'disabled:cursor-not-allowed'
	},
	{
		name: 'Link',
		component: Link,
		props: { href: '/x', children: text('Go') },
		find: (c) => c.querySelector('a'),
		pointer: true,
		disabledProps: { disabled: true },
		disabledClass: 'cursor-not-allowed'
	},
	{
		name: 'Input',
		component: Input,
		props: { label: 'Name' },
		find: (c) => c.querySelector('input'),
		pointer: false,
		disabledProps: { disabled: true },
		disabledClass: 'disabled:cursor-not-allowed'
	},
	{
		name: 'Textarea',
		component: Textarea,
		props: { label: 'Notes' },
		find: (c) => c.querySelector('textarea'),
		pointer: false,
		disabledProps: { disabled: true },
		disabledClass: 'disabled:cursor-not-allowed'
	},
	{
		name: 'Select',
		component: Select,
		props: { label: 'Stage', options },
		find: (c) => c.querySelector('select'),
		pointer: true,
		disabledProps: { disabled: true },
		disabledClass: 'disabled:cursor-not-allowed'
	},
	{
		name: 'Checkbox',
		component: Checkbox,
		props: { label: 'Remote' },
		find: (c) => c.querySelector('input'),
		pointer: true,
		disabledProps: { disabled: true },
		disabledClass: 'disabled:cursor-not-allowed'
	},
	{
		name: 'Card with href',
		component: Card,
		props: { href: '/x', children: text('Card') },
		find: (c) => c.querySelector('a'),
		pointer: true,
		disabledProps: { 'aria-disabled': 'true' },
		disabledClass: 'aria-disabled:cursor-not-allowed'
	},
	{
		name: 'Tab',
		component: Tabs,
		props: { label: 'Sections', items: tabItems, children: text('Panel') },
		find: (c) => c.querySelector('[role="tab"]'),
		pointer: true,
		disabledProps: { items: [{ ...tabItems[0], disabled: true }, tabItems[1]] },
		disabledClass: 'disabled:cursor-not-allowed'
	}
];

function classesOf(el: Element | null): string[] {
	expect(el).not.toBeNull();
	return (el?.getAttribute('class') ?? '').split(/\s+/).filter(Boolean);
}

describe('interactive primitives own their state and cursor classes', () => {
	it.each(interactiveRows)('$name', ({ component, props, find, pointer, disabledProps, disabledClass }) => {
		const { container } = render(component, props);
		const classes = classesOf(find(container));
		expect(classes.some((c) => c.includes('hover:'))).toBe(true);
		expect(classes.some((c) => c.includes('active:'))).toBe(true);
		expect(classes.some((c) => c.startsWith('focus-visible:'))).toBe(true);
		expect(classes.includes('cursor-pointer')).toBe(pointer);

		const disabled = render(component, { ...props, ...disabledProps });
		expect(classesOf(find(disabled.container))).toContain(disabledClass);
	});
});

const staticRows: { name: string; component: AnyComponent; props: Record<string, unknown> }[] = [
	{ name: 'Badge', component: Badge, props: { children: text('New') } },
	{ name: 'EmptyState', component: EmptyState, props: { title: 'Nothing here' } },
	{ name: 'Skeleton', component: Skeleton, props: {} },
	{ name: 'Card without href', component: Card, props: { children: text('Card') } },
	{ name: 'Tooltip', component: Tooltip, props: { content: 'Hint', children: text('Trigger') } }
];

describe('non-interactive primitives have no pointer cursor', () => {
	it.each(staticRows)('$name', ({ component, props }) => {
		const { container } = render(component, props);
		const all = [container, ...container.querySelectorAll('*')];
		const pointers = all.filter((el) => el.classList.contains('cursor-pointer'));
		expect(pointers).toEqual([]);
	});
});
