<script lang="ts">
	import { contactName } from '$lib/stores/contacts.svelte';

	let {
		email,
		fallbackName = '',
		class: className = ''
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
		<span class="truncate font-medium" title={name}>{name}</span>
		<span class="text-muted-foreground truncate text-xs" title={addr}>{addr}</span>
	</span>
{:else}
	<span class="block truncate {className}" title={addr}>{addr || '—'}</span>
{/if}
