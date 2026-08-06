<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import { get } from 'svelte/store';
	import { onMount, type Snippet } from 'svelte';
	import { Button } from '$lib/components/ui/button';
	import { swipeBackGesture } from '$lib/stores/swipe-back.svelte';
	import { disableTwoFingerSwipe } from '$lib/stores/swipe.store';

	let {
		target,
		class: className,
		children,
	}: { target: string; class?: string; children: Snippet } = $props();
	// Targets are runtime local paths; the generated resolve overload only accepts
	// one statically known route at a time.
	let href = $derived(resolve(target as '/mailbox'));

	function targetUrl(): string {
		return target;
	}

	function navigateBack(): Promise<void> {
		return goto(resolve(targetUrl() as '/mailbox'));
	}

	onMount(() =>
		swipeBackGesture.configure({
			onComplete: navigateBack,
			targetUrl,
			isDisabled: () => get(disableTwoFingerSwipe),
		})
	);
</script>

<Button variant="ghost" size="sm" class={className} {href}>
	<ArrowLeft class="h-4 w-4" />
	{@render children()}
</Button>
