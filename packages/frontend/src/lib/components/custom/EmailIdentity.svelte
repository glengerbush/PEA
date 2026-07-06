<script lang="ts">
	import { contactName } from '$lib/stores/contacts.svelte';
	import * as Tooltip from '$lib/components/ui/tooltip';

	let {
		email,
		fallbackName = '',
		class: className = '',
	}: {
		email: string | null | undefined;
		/** Name from the email itself (e.g. sender display name), used if no contact matches. */
		fallbackName?: string | null;
		class?: string;
	} = $props();

	const addr = $derived((email || '').trim());
	const name = $derived(contactName(addr) || (fallbackName || '').trim());
	// Only stack when we actually have a name distinct from the address.
	const showName = $derived(!!name && name.toLowerCase() !== addr.toLowerCase());
</script>

{#if showName}
	<span class="flex min-w-0 flex-col leading-tight {className}">
		<Tooltip.Root
			><Tooltip.Trigger
				>{#snippet child({ props })}<span {...props} class="truncate font-medium"
						>{name}</span
					>{/snippet}</Tooltip.Trigger
			><Tooltip.Content>{name}</Tooltip.Content></Tooltip.Root
		>
		<Tooltip.Root
			><Tooltip.Trigger
				>{#snippet child({ props })}<span
						{...props}
						class="text-muted-foreground truncate text-xs">{addr}</span
					>{/snippet}</Tooltip.Trigger
			><Tooltip.Content>{addr}</Tooltip.Content></Tooltip.Root
		>
	</span>
{:else}
	<Tooltip.Root
		><Tooltip.Trigger
			>{#snippet child({ props })}<span {...props} class="block truncate {className}"
					>{addr || '—'}</span
				>{/snippet}</Tooltip.Trigger
		><Tooltip.Content>{addr}</Tooltip.Content></Tooltip.Root
	>
{/if}
