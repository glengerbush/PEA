<script lang="ts">
	import { swipeBackGesture as swipe } from '$lib/stores/swipe-back.svelte';

	const underlayOffset = $derived((swipe.progress - 1) * 6);
	const shadeOpacity = $derived((1 - swipe.progress) * 0.16);
	const transition = $derived(
		swipe.transitionDurationMs > 0
			? `transform ${swipe.transitionDurationMs}ms cubic-bezier(0.22, 0.8, 0.2, 1)`
			: 'none'
	);
</script>

{#if swipe.active}
	<div
		data-swipe-snapshot-exclude
		class="bg-background pointer-events-none fixed inset-0 z-0 overflow-hidden"
		aria-hidden="true"
	>
		<div
			class="absolute inset-0"
			style:transform={`translate3d(${underlayOffset}vw, 0, 0)`}
			style:transition
		>
			{#if swipe.snapshot}
				<img
					src={swipe.snapshot.dataUrl}
					alt=""
					class="h-full w-full object-cover"
					draggable="false"
				/>
			{/if}
			<div
				class="absolute inset-0 bg-black"
				style:opacity={shadeOpacity}
				style:transition={swipe.transitionDurationMs > 0
					? `opacity ${swipe.transitionDurationMs}ms ease-out`
					: 'none'}
			></div>
		</div>
	</div>
{/if}
