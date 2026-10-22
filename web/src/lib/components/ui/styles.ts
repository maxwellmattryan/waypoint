// Shared class strings for the UI primitives. The AGENTS.md cursor and state rules live here so
// call sites never add `cursor-pointer` or focus styles themselves.

/** Keyboard focus ring. The 2px width and offset count as hairlines on the 4px grid. */
export const focusRing =
	'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus';

/** Base for native controls that support `:disabled` (buttons, select). */
export const interactive =
	'cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 transition-colors';

export const buttonVariants = {
	primary:
		'bg-primary text-primary-fg enabled:hover:bg-primary-hover enabled:active:bg-primary-active',
	secondary:
		'border border-border-strong bg-surface text-fg enabled:hover:bg-surface-raised enabled:active:bg-border',
	ghost: 'bg-transparent text-fg enabled:hover:bg-surface-raised enabled:active:bg-border',
	danger: 'bg-danger text-danger-fg enabled:hover:bg-danger-hover enabled:active:bg-danger-active'
} as const;

export type ButtonVariant = keyof typeof buttonVariants;

export const buttonSizes = {
	sm: 'h-8 px-3 text-sm',
	md: 'h-10 px-4 text-sm'
} as const;

export const iconButtonSizes = {
	sm: 'size-8',
	md: 'size-10'
} as const;

export type ButtonSize = keyof typeof buttonSizes;

/** Base for text-like form controls (Input, Textarea, Select). Text controls keep the text cursor. */
export const controlBase =
	'w-full rounded-md border bg-surface text-sm text-fg placeholder:text-fg-subtle transition-colors disabled:cursor-not-allowed disabled:opacity-50';

/** Border colour and hover/active states for a control, by error state. */
export function controlBorder(invalid: boolean): string {
	return invalid
		? 'border-danger enabled:hover:border-danger-hover enabled:active:border-danger-active'
		: 'border-border-strong enabled:hover:border-fg-subtle enabled:active:border-primary';
}

/** `aria-describedby` for a Field-wrapped control. */
export function describedBy(id: string, hint?: string, error?: string): string | undefined {
	const ids = [hint ? `${id}-hint` : '', error ? `${id}-error` : ''].filter(Boolean);
	return ids.length ? ids.join(' ') : undefined;
}

export const badgeTones = {
	neutral: 'bg-neutral-soft text-neutral-soft-fg',
	primary: 'bg-primary-soft text-primary-soft-fg',
	success: 'bg-success-soft text-success-soft-fg',
	warning: 'bg-warning-soft text-warning-soft-fg',
	danger: 'bg-danger-soft text-danger-soft-fg',
	info: 'bg-info-soft text-info-soft-fg'
} as const;

export type BadgeTone = keyof typeof badgeTones;

export const skeletonShapes = {
	rect: 'rounded-md',
	text: 'h-4 rounded-sm',
	circle: 'rounded-full'
} as const;

export type SkeletonShape = keyof typeof skeletonShapes;

/** Stacking order: overlays (Dialog, Drawer) sit under toasts and tooltips. */
export const layers = { overlay: 'z-40', toast: 'z-50', tooltip: 'z-50' } as const;

/** Surface for overlay panels and toasts. */
export const overlaySurface = 'bg-surface border border-border rounded-lg shadow-lg';

export const dialogSizes = {
	sm: 'max-w-sm',
	md: 'max-w-md',
	lg: 'max-w-2xl'
} as const;

export type DialogSize = keyof typeof dialogSizes;

export type DrawerSide = 'left' | 'right';

/** The accent bar colour for each toast tone. */
export const toastTones = {
	neutral: 'bg-neutral-soft-fg',
	success: 'bg-success-soft-fg',
	info: 'bg-info-soft-fg',
	warning: 'bg-warning-soft-fg',
	danger: 'bg-danger-soft-fg'
} as const;

export type ToastTone = keyof typeof toastTones;

/** Tooltip position relative to its trigger wrapper. */
export const tooltipPlacements = {
	top: 'bottom-full left-1/2 -translate-x-1/2 mb-2',
	bottom: 'top-full left-1/2 -translate-x-1/2 mt-2',
	left: 'right-full top-1/2 -translate-y-1/2 mr-2',
	right: 'left-full top-1/2 -translate-y-1/2 ml-2'
} as const;

export type TooltipPlacement = keyof typeof tooltipPlacements;
