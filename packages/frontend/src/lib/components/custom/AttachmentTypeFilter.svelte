<script module lang="ts">
	// Stable per-instance id suffix (deterministic across SSR/hydration).
	let idCounter = 0;
</script>

<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import { cn } from '$lib/utils.js';

	type Category = { label: string; exts: string[] };

	/** Sections shown in the dropdown; a category row toggles all of its types. */
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

	const uid = `attachment-type-${idCounter++}`;

	function parse(v: string): string[] {
		return v
			.split(',')
			.map((e) => e.trim().toLowerCase())
			.filter(Boolean);
	}

	let open = $state(false);
	let searchValue = $state('');
	let selected = $state<string[]>(parse(value));
	/** The single highlighted row, shared by mouse hover AND arrow keys. */
	let activeIndex = $state(-1);
	let containerEl = $state<HTMLDivElement | null>(null);
	let listEl = $state<HTMLDivElement | null>(null);

	// Keep the internal selection in sync when `value` changes externally (e.g. the
	// page reloads with filters from the URL). Compared as a set so the selection
	// order never triggers a spurious reset.
	$effect(() => {
		const incoming = parse(value).sort().join(',');
		if (incoming !== [...selected].sort().join(',')) {
			selected = parse(value);
		}
	});

	function commit(next: string[]): void {
		value = [...new Set(next)].join(',');
		onValueChange?.(value);
	}

	const query = $derived(searchValue.trim().toLowerCase());
	const visibleCategories = $derived(
		CATEGORIES.map((cat) => ({
			label: cat.label,
			// A category whose name matches shows all its types; otherwise only the
			// individual extensions that match the query.
			exts:
				query === '' || cat.label.toLowerCase().includes(query)
					? cat.exts
					: cat.exts.filter((e) => e.includes(query))
		})).filter((cat) => cat.exts.length > 0)
	);

	// Flatten categories + their items into ONE list of navigable rows, so the
	// mouse and the keyboard share a single highlighted index.
	type Row = { type: 'category'; label: string; exts: string[] } | { type: 'item'; ext: string };
	const rows = $derived.by<Row[]>(() => {
		const out: Row[] = [];
		for (const cat of visibleCategories) {
			out.push({ type: 'category', label: cat.label, exts: cat.exts });
			for (const ext of cat.exts) out.push({ type: 'item', ext });
		}
		return out;
	});

	const selectedSet = $derived(new Set(selected));
	const triggerLabel = $derived(
		selected.length === 0
			? 'Any file type'
			: `${selected.length} file type${selected.length === 1 ? '' : 's'}`
	);

	// Drop a stale highlight if filtering shrank the list past it.
	$effect(() => {
		if (activeIndex >= rows.length) activeIndex = -1;
	});

	function closeMenu(): void {
		open = false;
		searchValue = '';
		activeIndex = -1;
	}

	function toggleExt(ext: string): void {
		const set = new Set(selected);
		if (set.has(ext)) set.delete(ext);
		else set.add(ext);
		selected = [...set];
		commit(selected);
	}
	/** Select or clear every extension in a category (over what's currently shown). */
	function toggleCategory(exts: string[]): void {
		const set = new Set(selected);
		const allOn = exts.every((e) => set.has(e));
		for (const e of exts) {
			if (allOn) set.delete(e);
			else set.add(e);
		}
		selected = [...set];
		commit(selected);
	}
	function toggleRow(row: Row): void {
		if (row.type === 'category') toggleCategory(row.exts);
		else toggleExt(row.ext);
	}

	function scrollActiveIntoView(): void {
		requestAnimationFrame(() => {
			listEl?.querySelector(`[data-index="${activeIndex}"]`)?.scrollIntoView({ block: 'nearest' });
		});
	}
	function move(dir: 1 | -1): void {
		if (rows.length === 0) return;
		const start = activeIndex < 0 ? (dir === 1 ? -1 : 0) : activeIndex;
		activeIndex = (start + dir + rows.length) % rows.length;
		scrollActiveIntoView();
	}

	function onInputKeydown(e: KeyboardEvent): void {
		switch (e.key) {
			case 'ArrowDown':
				e.preventDefault();
				if (!open) open = true;
				else move(1);
				break;
			case 'ArrowUp':
				e.preventDefault();
				if (!open) open = true;
				else move(-1);
				break;
			case 'Enter':
				if (open && activeIndex >= 0 && activeIndex < rows.length) {
					e.preventDefault();
					toggleRow(rows[activeIndex]);
				}
				break;
			case 'Escape':
				if (open) {
					e.preventDefault();
					closeMenu();
				}
				break;
			case 'Home':
				if (open) {
					e.preventDefault();
					activeIndex = 0;
					scrollActiveIntoView();
				}
				break;
			case 'End':
				if (open) {
					e.preventDefault();
					activeIndex = rows.length - 1;
					scrollActiveIntoView();
				}
				break;
		}
	}

	// Close when the pointer goes down outside the component.
	$effect(() => {
		if (!open) return;
		function onPointerDown(e: PointerEvent) {
			if (containerEl && !containerEl.contains(e.target as Node)) closeMenu();
		}
		document.addEventListener('pointerdown', onPointerDown, true);
		return () => document.removeEventListener('pointerdown', onPointerDown, true);
	});
</script>

<div
	class="relative"
	bind:this={containerEl}
	onfocusout={(e) => {
		// Close when focus leaves the whole field (Tab / Shift+Tab away). Option
		// rows are tabindex=-1 and clicks preventDefault their blur, so focus stays
		// on the input during use — this only fires on a real tab-away.
		if (containerEl && !containerEl.contains(e.relatedTarget as Node | null)) closeMenu();
	}}
>
	<input
		type="text"
		role="combobox"
		aria-expanded={open}
		aria-controls={`${uid}-list`}
		aria-activedescendant={open && activeIndex >= 0 ? `${uid}-row-${activeIndex}` : undefined}
		aria-label="Filter by attachment type"
		placeholder={triggerLabel}
		bind:value={searchValue}
		oninput={() => {
			open = true;
			activeIndex = -1;
		}}
		onclick={() => (open = true)}
		onkeydown={onInputKeydown}
		class={cn(
			'border-input focus-visible:border-ring focus-visible:ring-ring/50 dark:bg-input/30 shadow-xs flex h-9 w-full cursor-pointer rounded-md border bg-transparent px-3 py-2 pr-9 text-sm outline-none transition-[color,box-shadow] focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50',
			selected.length > 0 && 'placeholder:text-foreground'
		)}
	/>
	<div
		aria-hidden="true"
		class="text-muted-foreground pointer-events-none absolute inset-y-0 right-0 flex items-center pr-2"
	>
		<ChevronsUpDown class="size-4 opacity-50" />
	</div>

	{#if open}
		<div
			bind:this={listEl}
			id={`${uid}-list`}
			role="listbox"
			aria-multiselectable="true"
			class="bg-popover text-popover-foreground absolute left-0 top-full z-50 mt-1 max-h-80 w-full min-w-[14rem] overflow-y-auto rounded-md border p-1 shadow-md"
		>
			{#if rows.length === 0}
				<div class="text-muted-foreground px-2 py-6 text-center text-sm">
					No file types match “{searchValue}”.
				</div>
			{:else}
				{#each rows as row, i (row.type === 'category' ? `c:${row.label}` : `e:${row.ext}`)}
					{#if row.type === 'category'}
						{@const allOn = row.exts.every((e) => selectedSet.has(e))}
						<button
							type="button"
							id={`${uid}-row-${i}`}
							data-index={i}
							tabindex={-1}
							role="option"
							aria-selected="false"
							class="flex w-full cursor-pointer items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-xs font-semibold {activeIndex ===
							i
								? 'bg-accent text-accent-foreground'
								: 'text-muted-foreground'}"
							onpointerenter={() => (activeIndex = i)}
							onmousedown={(e) => e.preventDefault()}
							onclick={() => toggleRow(row)}
						>
							<span>{row.label}</span>
							<span class="text-[11px] font-medium opacity-70">{allOn ? 'Clear' : 'All'}</span>
						</button>
					{:else}
						<button
							type="button"
							id={`${uid}-row-${i}`}
							data-index={i}
							tabindex={-1}
							role="option"
							aria-selected={selectedSet.has(row.ext)}
							class="relative flex w-full cursor-pointer select-none items-center rounded-sm py-1.5 pl-8 pr-2 text-left text-sm outline-none {activeIndex ===
							i
								? 'bg-accent text-accent-foreground'
								: ''}"
							onpointerenter={() => (activeIndex = i)}
							onmousedown={(e) => e.preventDefault()}
							onclick={() => toggleRow(row)}
						>
							<span class="absolute left-2 flex size-3.5 items-center justify-center">
								{#if selectedSet.has(row.ext)}
									<Check class="size-4" />
								{/if}
							</span>
							.{row.ext}
						</button>
					{/if}
				{/each}
			{/if}
		</div>
	{/if}
</div>
