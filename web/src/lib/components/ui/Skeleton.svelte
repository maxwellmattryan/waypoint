<script lang="ts" module>
	import type { HTMLAttributes } from 'svelte/elements';
	import type { SkeletonShape } from './styles';

	export interface SkeletonProps extends HTMLAttributes<HTMLDivElement> {
		shape?: SkeletonShape;
	}
</script>

<script lang="ts">
	import { skeletonShapes } from './styles';

	let { shape = 'rect', class: className, ...rest }: SkeletonProps = $props();
</script>

<div
	aria-hidden="true"
	class={['bg-surface-raised motion-safe:animate-pulse', skeletonShapes[shape], className]}
	{...rest}
></div>
