<script lang="ts">
	import { Combobox } from 'bits-ui';
	import { Check, Plus, Tag } from 'lucide-svelte';
	import { cn } from '$lib/utils.js';

	let {
		existingTags = [],
		disabled = false,
		placeholder = 'Add or search tags…',
		onSelect,
		class: className
	}: {
		/** All known tags to offer as suggestions. */
		existingTags?: string[];
		disabled?: boolean;
		placeholder?: string;
		/** Called with the chosen tag (existing or newly typed). */
		onSelect: (tag: string) => void;
		class?: string;
	} = $props();

	let open = $state(false);
	let searchValue = $state('');
	let value = $state('');

	const trimmed = $derived(searchValue.trim());
	const filtered = $derived(
		trimmed === ''
			? existingTags
			: existingTags.filter((t) => t.toLowerCase().includes(trimmed.toLowerCase()))
	);
	// Offer "create" only when the typed text doesn't already exist verbatim.
	const canCreate = $derived(
		trimmed !== '' && !existingTags.some((t) => t.toLowerCase() === trimmed.toLowerCase())
	);

	function choose(tag: string) {
		const v = tag.trim();
		// reset first so the field is ready for the next addition
		value = '';
		searchValue = '';
		open = false;
		if (v) onSelect(v);
	}
</script>

<Combobox.Root
	type="single"
	bind:value
	bind:open
	{disabled}
	onValueChange={(v) => {
		if (v) choose(v);
	}}
>
	<div class="relative">
		<Combobox.Input
			{placeholder}
			oninput={(e) => {
				searchValue = e.currentTarget.value;
				open = true;
			}}
			onclick={() => (open = true)}
			class={cn(
				'border-input focus-visible:border-ring focus-visible:ring-ring/50 dark:bg-input/30 shadow-xs flex h-8 w-56 cursor-pointer rounded-md border bg-transparent px-3 py-1 text-sm outline-none transition-[color,box-shadow] focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50',
				className
			)}
		/>
	</div>
	<Combobox.Portal>
		<Combobox.Content
			sideOffset={4}
			class="bg-popover text-popover-foreground data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 relative z-50 max-h-72 min-w-[14rem] overflow-y-auto overflow-x-hidden rounded-md border shadow-md"
		>
			<Combobox.Viewport class="p-1">
				{#if canCreate}
					<Combobox.Item
						value={trimmed}
						label={trimmed}
						class="data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground outline-hidden relative flex w-full cursor-default select-none items-center gap-2 rounded-sm py-1.5 pl-2 pr-2 text-sm"
					>
						{#snippet children()}
							<Plus class="size-3.5 opacity-70" />
							<span>Create “{trimmed}”</span>
						{/snippet}
					</Combobox.Item>
				{/if}
				{#each filtered as tag (tag)}
					<Combobox.Item
						value={tag}
						label={tag}
						class="data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground outline-hidden relative flex w-full cursor-default select-none items-center gap-2 rounded-sm py-1.5 pl-2 pr-8 text-sm"
					>
						{#snippet children({ selected })}
							<Tag class="size-3.5 opacity-60" />
							<span class="truncate">{tag}</span>
							{#if selected}
								<Check class="absolute right-2 size-4" />
							{/if}
						{/snippet}
					</Combobox.Item>
				{:else}
					{#if !canCreate}
						<div class="text-muted-foreground py-2 text-center text-sm">
							{trimmed ? 'No matching tags' : 'No tags yet — type to create one'}
						</div>
					{/if}
				{/each}
			</Combobox.Viewport>
		</Combobox.Content>
	</Combobox.Portal>
</Combobox.Root>
