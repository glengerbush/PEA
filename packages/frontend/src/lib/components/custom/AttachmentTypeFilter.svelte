<script lang="ts">
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu';
	import { Button } from '$lib/components/ui/button';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';

	type Category = { label: string; exts: string[] };

	/** Sections shown in the dropdown; checking a section selects all its types. */
	const CATEGORIES: Category[] = [
		{
			label: 'Pictures',
			exts: ['jpg', 'jpeg', 'png', 'gif', 'bmp', 'tif', 'tiff', 'webp', 'heic', 'heif', 'svg']
		},
		{
			label: 'Documents',
			exts: ['pdf', 'doc', 'docx', 'rtf', 'txt', 'md', 'odt', 'pages', 'wpd']
		},
		{ label: 'Spreadsheets', exts: ['xls', 'xlsx', 'csv', 'ods', 'numbers'] },
		{ label: 'Presentations', exts: ['ppt', 'pptx', 'odp', 'key'] },
		{ label: 'Archives', exts: ['zip', 'rar', '7z', 'tar', 'gz', 'tgz'] },
		{
			label: '3D Printing',
			exts: ['stl', '3mf', 'obj', 'gcode', 'step', 'stp', 'scad', 'f3d']
		},
		{ label: 'Audio', exts: ['mp3', 'wav', 'm4a', 'aac', 'ogg', 'flac'] },
		{ label: 'Video', exts: ['mp4', 'mov', 'avi', 'mkv', 'wmv', 'webm'] },
		{ label: 'Email & Contacts', exts: ['eml', 'ics', 'vcf'] }
	];

	let {
		value = $bindable(''),
		onValueChange = undefined
	}: {
		/** Comma-separated list of selected extensions (URL-param friendly). */
		value?: string;
		onValueChange?: (value: string) => void;
	} = $props();

	const selected = $derived(
		new Set(
			value
				.split(',')
				.map((e) => e.trim().toLowerCase())
				.filter(Boolean)
		)
	);

	function commit(next: Set<string>) {
		value = [...next].join(',');
		onValueChange?.(value);
	}

	function toggleExt(ext: string) {
		const next = new Set(selected);
		if (next.has(ext)) {
			next.delete(ext);
		} else {
			next.add(ext);
		}
		commit(next);
	}

	function categoryCount(cat: Category): number {
		return cat.exts.filter((e) => selected.has(e)).length;
	}

	function toggleCategory(cat: Category) {
		const next = new Set(selected);
		if (categoryCount(cat) === cat.exts.length) {
			cat.exts.forEach((e) => next.delete(e));
		} else {
			cat.exts.forEach((e) => next.add(e));
		}
		commit(next);
	}

	const triggerLabel = $derived(
		selected.size === 0
			? 'Any file type'
			: `${selected.size} file type${selected.size === 1 ? '' : 's'}`
	);
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="outline" class="w-full justify-between gap-2 font-normal">
				<span class="truncate">{triggerLabel}</span>
				<ChevronsUpDown class="h-4 w-4 flex-shrink-0 opacity-50" />
			</Button>
		{/snippet}
	</DropdownMenu.Trigger>
	<DropdownMenu.Content class="max-h-80 w-60 overflow-y-auto">
		{#if selected.size > 0}
			<DropdownMenu.Item onclick={() => commit(new Set())}>Clear selection</DropdownMenu.Item>
			<DropdownMenu.Separator />
		{/if}
		{#each CATEGORIES as cat, i (cat.label)}
			{#if i > 0}
				<DropdownMenu.Separator />
			{/if}
			<DropdownMenu.CheckboxItem
				class="font-semibold"
				closeOnSelect={false}
				checked={categoryCount(cat) === cat.exts.length}
				indeterminate={categoryCount(cat) > 0 && categoryCount(cat) < cat.exts.length}
				onCheckedChange={() => toggleCategory(cat)}
			>
				{cat.label}
			</DropdownMenu.CheckboxItem>
			{#each cat.exts as ext (ext)}
				<DropdownMenu.CheckboxItem
					class="pl-10"
					closeOnSelect={false}
					checked={selected.has(ext)}
					onCheckedChange={() => toggleExt(ext)}
				>
					.{ext}
				</DropdownMenu.CheckboxItem>
			{/each}
		{/each}
	</DropdownMenu.Content>
</DropdownMenu.Root>
