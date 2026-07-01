<script lang="ts">
	import { getAlerts } from './alert-state.svelte';
	import AlertCard from './AlertCard.svelte';
	import { flip } from 'svelte/animate';
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';

	const alerts = $derived(getAlerts());
</script>

<!-- Global notification live region, rendered permanently. Alerts stack vertically;
     items slide in/out and the rest reflow smoothly via animate:flip. -->
{#if alerts.length > 0}
	<div
		aria-live="assertive"
		class="z-999999 pointer-events-none fixed inset-0 flex items-start px-4 py-6 sm:p-6"
	>
		<div class="flex w-full flex-col items-center gap-3 sm:items-end">
			{#each alerts as alert (alert.id)}
				<div
					class="w-full max-w-sm"
					in:fly={{ x: 400, duration: 300, easing: cubicOut }}
					out:fly={{ x: 400, duration: 250, easing: cubicOut }}
					animate:flip={{ duration: 250, easing: cubicOut }}
				>
					<AlertCard {alert} />
				</div>
			{/each}
		</div>
	</div>
{/if}
