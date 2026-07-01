<script lang="ts">
	import { Combobox } from 'bits-ui';
	import { Check, ChevronsUpDown } from 'lucide-svelte';
	import { cn } from '$lib/utils.js';

	type Option = { value: string; label: string };

	let {
		value = $bindable(''),
		options,
		placeholder = 'Select…',
		name,
		class: className
	}: {
		/** Selected option value (two-way bindable). */
		value?: string;
		options: Option[];
		placeholder?: string;
		name?: string;
		class?: string;
	} = $props();

	let open = $state(false);
	let inputValue = $state('');
	let searchValue = $state('');

	const selectedLabel = $derived(options.find((option) => option.value === value)?.label ?? '');
	const filtered = $derived(
		searchValue.trim() === ''
			? options
			: options.filter((option) =>
					option.label.toLowerCase().includes(searchValue.trim().toLowerCase())
				)
	);

	// While closed, keep the field showing the current selection. This also keeps
	// it correct when `value` changes externally (e.g. the page reloads with new
	// filters from the URL).
	$effect(() => {
		if (!open) {
			inputValue = selectedLabel;
			searchValue = '';
		}
	});
</script>

<Combobox.Root type="single" {name} bind:value bind:open {inputValue}>
	<div class="relative">
		<Combobox.Input
			{placeholder}
			oninput={(e) => (searchValue = e.currentTarget.value)}
			onfocus={(e) => e.currentTarget.select()}
			onclick={() => (open = true)}
			class={cn(
				'border-input focus-visible:border-ring focus-visible:ring-ring/50 dark:bg-input/30 shadow-xs flex h-9 w-full cursor-pointer rounded-md border bg-transparent px-3 py-2 pr-9 text-sm outline-none transition-[color,box-shadow] focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50',
				className
			)}
		/>
		<!-- Decorative chevron: clicks fall through to the input, so clicking
		     anywhere in the field opens the list and focuses the input. -->
		<div
			aria-hidden="true"
			class="text-muted-foreground pointer-events-none absolute inset-y-0 right-0 flex items-center pr-2"
		>
			<ChevronsUpDown class="size-4 opacity-50" />
		</div>
	</div>
	<Combobox.Portal>
		<Combobox.Content
			sideOffset={4}
			class="bg-popover text-popover-foreground data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 relative z-50 max-h-72 min-w-[8rem] overflow-y-auto overflow-x-hidden rounded-md border shadow-md"
		>
			<Combobox.Viewport class="p-1">
				{#each filtered as option (option.value)}
					<Combobox.Item
						value={option.value}
						label={option.label}
						class="data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground outline-hidden relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pl-2 pr-8 text-sm data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
					>
						{#snippet children({ selected })}
							<span class="absolute right-2 flex size-3.5 items-center justify-center">
								{#if selected}
									<Check class="size-4" />
								{/if}
							</span>
							{option.label}
						{/snippet}
					</Combobox.Item>
				{:else}
					<div class="text-muted-foreground py-2 text-center text-sm">
						{searchValue.trim() ? 'No results' : 'No options'}
					</div>
				{/each}
			</Combobox.Viewport>
		</Combobox.Content>
	</Combobox.Portal>
</Combobox.Root>
