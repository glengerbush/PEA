<script lang="ts">
	import type { AlertItem } from './alert-state.svelte';
	import { dismissAlert } from './alert-state.svelte';
	import { CircleCheck, CircleX, TriangleAlert } from 'lucide-svelte';

	let { alert }: { alert: AlertItem } = $props();

	const AlertIcon = $derived(
		alert.type === 'success' ? CircleCheck : alert.type === 'error' ? CircleX : TriangleAlert
	);

	const styleConfig = $derived(
		alert.type === 'success'
			? {
					color: 'text-green-600',
					messageColor: 'text-green-500',
					bgColor: 'bg-green-50',
				}
			: alert.type === 'error'
				? {
						color: 'text-red-600',
						messageColor: 'text-red-500',
						bgColor: 'bg-red-50',
					}
				: {
						color: 'text-yellow-600',
						messageColor: 'text-yellow-600',
						bgColor: 'bg-yellow-50',
					}
	);

	let timeout: ReturnType<typeof setTimeout> | undefined;

	function schedule() {
		if (alert.duration > 0) {
			timeout = setTimeout(() => dismissAlert(alert.id), alert.duration);
		}
	}
	function pause() {
		clearTimeout(timeout);
	}

	$effect(() => {
		schedule();
		return () => clearTimeout(timeout);
	});
</script>

<div
	class="pointer-events-auto w-full overflow-hidden rounded-lg shadow-lg ring-1 ring-black/5 {styleConfig.bgColor}"
	role="alert"
	onmouseenter={pause}
	onmouseleave={schedule}
>
	<div class="p-4">
		<div class="flex items-start">
			<div class="shrink-0">
				<AlertIcon class="size-6 {styleConfig.color}" />
			</div>
			<div class="ml-3 w-0 flex-1 pt-0.5">
				<p class="text-sm font-medium {styleConfig.color}">{alert.title}</p>
				{#if alert.message}
					<p class="mt-1 text-sm {styleConfig.messageColor}">{alert.message}</p>
				{/if}
			</div>
			<div class="ml-4 flex shrink-0">
				<button
					type="button"
					class="inline-flex rounded-md {styleConfig.color} cursor-pointer"
					onclick={() => dismissAlert(alert.id)}
				>
					<span class="sr-only">Close</span>
					<svg
						class="size-5"
						viewBox="0 0 20 20"
						fill="currentColor"
						aria-hidden="true"
						data-slot="icon"
					>
						<path
							d="M6.28 5.22a.75.75 0 0 0-1.06 1.06L8.94 10l-3.72 3.72a.75.75 0 1 0 1.06 1.06L10 11.06l3.72 3.72a.75.75 0 1 0 1.06-1.06L11.06 10l3.72-3.72a.75.75 0 0 0-1.06-1.06L10 8.94 6.28 5.22Z"
						/>
					</svg>
				</button>
			</div>
		</div>
	</div>
</div>
