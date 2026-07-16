<script lang="ts">
	import type { PageData } from './$types';
	import * as Table from '$lib/components/ui/table';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import { Switch } from '$lib/components/ui/switch';
	import * as Select from '$lib/components/ui/select';
	import * as Dialog from '$lib/components/ui/dialog';
	import SearchableSelect from '$lib/components/custom/SearchableSelect.svelte';
	import AttachmentTypeFilter from '$lib/components/custom/AttachmentTypeFilter.svelte';
	import EmailIdentity from '$lib/components/custom/EmailIdentity.svelte';
	import { describeDate } from '$lib/stores/datetime.svelte';
	import { goto, afterNavigate, beforeNavigate } from '$app/navigation';
	import { onDestroy } from 'svelte';
	import { page as appPage } from '$app/state';
	import { lastMailboxListUrl } from '$lib/stores/mailbox-nav';
	import { saveListScroll, getListScroll, lastOpenedEmailId } from '$lib/stores/list-view-state';
	import { api } from '$lib/api.client';
	import { t } from '$lib/translations';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { shouldSyncInputsFromUrl } from '$lib/search-filters';
	import TablePagination from '$lib/components/custom/TablePagination.svelte';
	import Paperclip from '@lucide/svelte/icons/paperclip';
	import Search from '@lucide/svelte/icons/search';
	import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import TagsIcon from '@lucide/svelte/icons/tags';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import X from '@lucide/svelte/icons/x';
	import type {
		ArchiveSortField,
		BulkDeleteArchivedEmailsResult,
		SearchHit,
		SortDirection,
		UpdateArchivedEmailTagsResult,
	} from '@pea/types';

	type SearchFieldMatch = 'all' | 'any';

	type SelectOption = {
		value: string;
		label: string;
	};

	let { data }: { data: PageData } = $props();

	let ingestionSources = $derived(data.ingestionSources);
	let searchResult = $derived(data.searchResult);
	let filters = $derived(data.filters);

	// The form inputs are the user's to edit, not the search's to overwrite.
	// They are seeded from the URL once here, then owned locally: typing/selecting
	// writes them, and buildArchiveUrl() writes them INTO the URL. A search
	// returning must never write back — when they were $derived(filters.…), a slow
	// response overwrote characters typed while it was in flight (deleting letters
	// mid-word). The URL re-seeds them only on user-initiated navigation, in the
	// afterNavigate reconcile below. resultsMatchQuery lets the UI compare the two
	// instead of syncing them.
	let q = $state(data.filters.q);
	let senderQuery = $state(data.filters.senderQuery);
	let recipientsQuery = $state(data.filters.recipientsQuery);
	let subjectQuery = $state(data.filters.subjectQuery);
	let bodyQuery = $state(data.filters.bodyQuery);
	let fieldMatch: SearchFieldMatch = $state(data.filters.fieldMatch);
	let ingestionSourceId = $state(data.filters.ingestionSourceId);
	let hasAttachments = $state(data.filters.hasAttachments);
	let attachmentExt = $state(data.filters.attachmentExt);
	let tags = $state(data.filters.tags);
	let sort: ArchiveSortField = $state(data.filters.sort);
	let direction: SortDirection = $state(data.filters.direction);
	let limit = $state(String(data.filters.limit));
	let showAdvanced = $state(
		Boolean(
			data.filters.senderQuery ||
			data.filters.recipientsQuery ||
			data.filters.subjectQuery ||
			data.filters.bodyQuery ||
			data.filters.ingestionSourceId !== 'all' ||
			data.filters.hasAttachments !== 'any' ||
			data.filters.attachmentExt ||
			data.filters.tags ||
			data.filters.fieldMatch === 'any'
		)
	);

	// True when the visible results are for exactly what's in the search box.
	// data.filters.q is the query the currently-shown results ran with (only the
	// last-fired navigation's data is ever applied), so this is false only while a
	// newer search is still pending — never because of an out-of-order response.
	let resultsMatchQuery = $derived(
		showAdvanced
			? data.filters.q === '' &&
					data.filters.senderQuery === senderQuery.trim() &&
					data.filters.recipientsQuery === recipientsQuery.trim() &&
					data.filters.subjectQuery === subjectQuery.trim() &&
					data.filters.bodyQuery === bodyQuery.trim() &&
					data.filters.fieldMatch === fieldMatch &&
					data.filters.ingestionSourceId === ingestionSourceId &&
					data.filters.hasAttachments === hasAttachments &&
					data.filters.attachmentExt === attachmentExt &&
					data.filters.tags === tags
			: data.filters.q === q.trim() &&
					data.filters.senderQuery === '' &&
					data.filters.recipientsQuery === '' &&
					data.filters.subjectQuery === '' &&
					data.filters.bodyQuery === '' &&
					data.filters.ingestionSourceId === 'all' &&
					data.filters.hasAttachments === 'any' &&
					data.filters.attachmentExt === '' &&
					data.filters.tags === ''
	);

	// Origin breadcrumb: dashboard drill-downs (charts/stat cards) land here with
	// ?from=/dashboard so the list can offer a way back up. Preserved by
	// buildArchiveUrl() so it survives searching/filtering on the way.
	const fromParam = $derived(appPage.url.searchParams.get('from') || '');
	const cameFromDashboard = $derived(fromParam.startsWith('/dashboard'));

	// The Clear button resets to the pristine /mailbox view, so it should show
	// whenever any filter/search is active. buildArchiveUrl() only writes a query
	// param when its value deviates from the default, so "any param present that
	// isn't display/pagination" is exactly the set of active filters. Listing the
	// non-filter params (rather than the filters) means a filter added to
	// buildArchiveUrl() in the future lights up Clear automatically — only a new
	// display/pagination/origin param would need to be added to this set.
	const DISPLAY_PARAMS = new Set(['sort', 'direction', 'limit', 'page', 'from']);
	let hasActiveFilters = $derived(
		[...appPage.url.searchParams.keys()].some((key) => !DISPLAY_PARAMS.has(key))
	);
	let selectedIds = $state<string[]>([]);
	let tagInput = $state('');
	let removeTagInput = $state('');
	let isUpdatingTags = $state(false);
	let isDeleting = $state(false);
	let isDeleteDialogOpen = $state(false);

	// Remember this list view (with its full search/filter/page query) so the
	// email detail page's Back button can return to the exact same view.
	$effect(() => {
		lastMailboxListUrl.set(appPage.url.pathname + appPage.url.search);
	});

	// Remember scroll position and which email was opened, then restore both on
	// return — the Back button/swipe uses goto(), so SvelteKit's own scroll
	// restoration (popstate-only) never fires. Keyed by full URL so each
	// filter/sort/page variant restores independently.
	beforeNavigate(({ to }) => {
		saveListScroll(appPage.url.pathname + appPage.url.search, window.scrollY);
		const openedId = to?.url.pathname.match(/^\/mailbox\/([^/]+)$/)?.[1];
		if (openedId) lastOpenedEmailId.set(openedId);
	});
	afterNavigate(({ type }) => {
		// Re-seed the inputs from the URL only on user-initiated navigation
		// (initial load, back/forward, Clear link, deep link) — never after our
		// own search-as-you-type goto, so a returning search can't edit what's
		// being typed. See the $state seeds above and shouldSyncInputsFromUrl.
		if (shouldSyncInputsFromUrl(type)) {
			q = data.filters.q;
			senderQuery = data.filters.senderQuery;
			recipientsQuery = data.filters.recipientsQuery;
			subjectQuery = data.filters.subjectQuery;
			bodyQuery = data.filters.bodyQuery;
			fieldMatch = data.filters.fieldMatch;
			ingestionSourceId = data.filters.ingestionSourceId;
			hasAttachments = data.filters.hasAttachments;
			attachmentExt = data.filters.attachmentExt;
			tags = data.filters.tags;
			sort = data.filters.sort;
			direction = data.filters.direction;
			limit = String(data.filters.limit);
			showAdvanced = Boolean(
				data.filters.senderQuery ||
				data.filters.recipientsQuery ||
				data.filters.subjectQuery ||
				data.filters.bodyQuery ||
				data.filters.ingestionSourceId !== 'all' ||
				data.filters.hasAttachments !== 'any' ||
				data.filters.attachmentExt ||
				data.filters.tags ||
				data.filters.fieldMatch === 'any'
			);
		}

		const y = getListScroll(appPage.url.pathname + appPage.url.search);
		if (y !== undefined) requestAnimationFrame(() => window.scrollTo(0, y));
	});

	const limitOptions: SelectOption[] = [
		{ value: '25', label: '25 per page' },
		{ value: '50', label: '50 per page' },
		{ value: '100', label: '100 per page' },
	];

	let sourceOptions = $derived([
		{ value: 'all', label: 'All sources' },
		...ingestionSources.map((source) => ({ value: source.id, label: source.name })),
	]);

	const sourceLabel = $derived(getOptionLabel(sourceOptions, ingestionSourceId, 'All sources'));

	// Column-header sorting: clicking a sortable header sets/toggles sort+direction
	// and reloads immediately (page size does the same). buildArchiveUrl() reads
	// these reactive vars, so we just update them and navigate.
	function toggleSort(field: ArchiveSortField) {
		if (sort === field) {
			direction = direction === 'asc' ? 'desc' : 'asc';
		} else {
			sort = field;
			direction = 'desc';
		}
		selectedIds = [];
		goto(buildArchiveUrl(1), { keepFocus: true });
	}
	function changePageSize(value: string) {
		limit = value;
		selectedIds = [];
		goto(buildArchiveUrl(1), { keepFocus: true });
	}

	// Filter dropdown options sourced from existing archive data instead of free text.
	let filterFacets = $derived(data.filterFacets);
	let tagFilterOptions = $derived([
		{ value: '', label: 'Any tag' },
		...filterFacets.tags.map((tag) => ({ value: tag, label: tag })),
	]);
	// Derive the range from the actual hits so an out-of-range page (e.g. a
	// stale /mailbox?page=999 deep link) shows 0-0, never an inverted "24951-30".
	const resultStart = $derived(
		searchResult.hits.length === 0 ? 0 : (searchResult.page - 1) * searchResult.limit + 1
	);
	const resultEnd = $derived(
		searchResult.hits.length === 0
			? 0
			: (searchResult.page - 1) * searchResult.limit + searchResult.hits.length
	);
	const visibleIds = $derived(searchResult.hits.map((email) => email.id));
	const selectedVisibleIds = $derived(selectedIds.filter((id) => visibleIds.includes(id)));
	const visibleTags = $derived.by(() => {
		const values: string[] = [];
		const seenKeys: string[] = [];
		for (const email of searchResult.hits) {
			if (!Array.isArray(email.tags)) continue;
			for (const tag of email.tags) {
				const key = tag.toLocaleLowerCase();
				if (seenKeys.includes(key)) continue;
				seenKeys.push(key);
				values.push(tag);
			}
		}
		return values.sort((a, b) => a.localeCompare(b));
	});
	const allVisibleSelected = $derived(
		visibleIds.length > 0 && selectedVisibleIds.length === visibleIds.length
	);
	const selectionState = $derived(
		allVisibleSelected
			? true
			: ((selectedVisibleIds.length > 0 ? 'indeterminate' : false) as any)
	);

	function getOptionLabel(options: SelectOption[], value: string, fallback: string): string {
		return options.find((option) => option.value === value)?.label || fallback;
	}

	function setParam(params: URLSearchParams, key: string, value: string, skipValue = '') {
		const trimmed = value.trim();
		if (trimmed && trimmed !== skipValue) {
			params.set(key, trimmed);
		}
	}

	// Overrides let change callbacks pass their fresh value directly: the bound
	// vars are $derived from loader data, so a navigation racing this build (e.g.
	// a debounced search keystroke) can reset them before the URL is assembled.
	type FilterOverrides = Partial<{
		senderQuery: string;
		recipientsQuery: string;
		subjectQuery: string;
		bodyQuery: string;
		fieldMatch: SearchFieldMatch;
		ingestionSourceId: string;
		hasAttachments: string;
		attachmentExt: string;
		tags: string;
	}>;

	function buildArchiveUrl(pageNumber = 1, overrides: FilterOverrides = {}): string {
		const params = new URLSearchParams();
		if (showAdvanced) {
			setParam(params, 'senderQuery', overrides.senderQuery ?? senderQuery);
			setParam(params, 'recipientsQuery', overrides.recipientsQuery ?? recipientsQuery);
			setParam(params, 'subjectQuery', overrides.subjectQuery ?? subjectQuery);
			setParam(params, 'bodyQuery', overrides.bodyQuery ?? bodyQuery);
			setParam(params, 'fieldMatch', overrides.fieldMatch ?? fieldMatch, 'all');
			setParam(
				params,
				'ingestionSourceId',
				overrides.ingestionSourceId ?? ingestionSourceId,
				'all'
			);
			setParam(params, 'hasAttachments', overrides.hasAttachments ?? hasAttachments, 'any');
			setParam(params, 'attachmentExt', overrides.attachmentExt ?? attachmentExt);
			setParam(params, 'tags', overrides.tags ?? tags);
		} else {
			setParam(params, 'q', q);
		}
		setParam(params, 'sort', sort, 'sentAt');
		setParam(params, 'direction', direction, 'desc');
		setParam(params, 'limit', limit, '25');
		setParam(params, 'from', fromParam);
		if (pageNumber > 1) {
			params.set('page', String(pageNumber));
		}
		const query = params.toString();
		return `/mailbox${query ? `?${query}` : ''}`;
	}

	// Search-as-you-type / instant filtering. Navigation re-derives all inputs
	// from the URL, so an apply after each change round-trips cleanly (no loops).
	let searchDebounce: ReturnType<typeof setTimeout> | undefined;
	function applyNow(overrides: FilterOverrides = {}) {
		if (searchDebounce) clearTimeout(searchDebounce);
		selectedIds = [];
		goto(buildArchiveUrl(1, overrides), { keepFocus: true, replaceState: true });
	}
	function setAdvancedMode(enabled: boolean) {
		showAdvanced = enabled;
		applyNow();
	}
	// Keystrokes: FTS5 prefix search answers in ~10-30ms; a short debounce just
	// avoids piling a navigation on every character.
	function applyDebounced() {
		if (searchDebounce) clearTimeout(searchDebounce);
		searchDebounce = setTimeout(applyNow, 250);
	}
	// Clear a pending debounce on unmount, or a keystroke-then-navigate within
	// 250ms fires applyNow() after we've left, bouncing back to /mailbox.
	onDestroy(() => {
		if (searchDebounce) clearTimeout(searchDebounce);
	});

	// Enter in the search box applies immediately (skips the debounce).
	function handleApplyFilters(event: SubmitEvent) {
		event.preventDefault();
		applyNow();
	}

	function toggleEmailSelection(emailId: string) {
		if (selectedIds.includes(emailId)) {
			selectedIds = selectedIds.filter((id) => id !== emailId);
			return;
		}
		selectedIds = [...selectedIds, emailId];
	}

	function toggleVisibleSelection(checked: boolean | 'indeterminate') {
		selectedIds = checked === true ? visibleIds : [];
	}

	function parseTagsInput(value: string): string[] {
		const seenKeys: string[] = [];
		const parsedTags: string[] = [];
		for (const rawTag of value.split(',')) {
			const tag = rawTag.trim().replace(/^#+/, '').trim();
			const key = tag.toLocaleLowerCase();
			if (!tag || seenKeys.includes(key)) continue;
			seenKeys.push(key);
			parsedTags.push(tag);
		}
		return parsedTags;
	}

	async function updateSelectedTags() {
		const emailIds = selectedVisibleIds;
		const addTags = parseTagsInput(tagInput);
		const removeTags = parseTagsInput(removeTagInput);
		if (emailIds.length === 0 || (addTags.length === 0 && removeTags.length === 0)) return;

		isUpdatingTags = true;
		try {
			const response = await api('/archived-emails/bulk/tags', {
				method: 'POST',
				body: JSON.stringify({ emailIds, addTags, removeTags }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to update tags');
			}

			const result = body as UpdateArchivedEmailTagsResult;
			const tagsById = new Map(result.emails.map((email) => [email.id, email.tags]));
			searchResult = {
				...searchResult,
				hits: searchResult.hits.map((email) =>
					tagsById.has(email.id)
						? {
								...email,
								tags: tagsById.get(email.id) || [],
							}
						: email
				),
			};

			tagInput = '';
			removeTagInput = '';
			setAlert({
				type: 'success',
				title: 'Tags updated',
				message: `${result.updatedCount} email${result.updatedCount === 1 ? '' : 's'} updated`,
				duration: 3000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Tag update failed',
				message: error instanceof Error ? error.message : 'Failed to update tags',
				duration: 5000,
				show: true,
			});
		} finally {
			isUpdatingTags = false;
		}
	}

	async function deleteSelectedEmails() {
		const emailIds = selectedVisibleIds;
		if (emailIds.length === 0) return;

		isDeleting = true;
		try {
			const response = await api('/archived-emails/bulk/delete', {
				method: 'POST',
				body: JSON.stringify({ emailIds }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to delete emails');
			}

			const result = body as BulkDeleteArchivedEmailsResult;
			selectedIds = [];
			isDeleteDialogOpen = false;

			// Re-run the loader so the page refills from the remaining emails.
			// Deleting can empty the current page entirely, so clamp to the
			// last page that still exists.
			const remaining = Math.max(0, searchResult.total - result.deletedCount);
			const totalPages = Math.max(1, Math.ceil(remaining / Number(limit || '25')));
			const targetPage = Math.min(filters.page, totalPages);
			await goto(buildArchiveUrl(targetPage), {
				invalidateAll: true,
				keepFocus: true,
				replaceState: true,
			});

			if (result.failed.length > 0) {
				setAlert({
					type: result.deletedCount > 0 ? 'warning' : 'error',
					title:
						result.deletedCount > 0
							? `Deleted ${result.deletedCount}, ${result.failed.length} could not be deleted`
							: 'No emails were deleted',
					message: `${result.failed.length} email${result.failed.length === 1 ? '' : 's'} could not be deleted.`,
					duration: 6000,
					show: true,
				});
			} else {
				setAlert({
					type: 'success',
					title: 'Emails deleted',
					message: `${result.deletedCount} email${result.deletedCount === 1 ? '' : 's'} deleted`,
					duration: 3000,
					show: true,
				});
			}
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Delete failed',
				message: error instanceof Error ? error.message : 'Failed to delete emails',
				duration: 5000,
				show: true,
			});
		} finally {
			isDeleting = false;
		}
	}
</script>

<svelte:head>
	<title>{$t('app.archived_emails_page.title')} - PEA</title>
</svelte:head>

{#if cameFromDashboard}
	<div class="mb-2">
		<Button variant="ghost" size="sm" class="gap-2" onclick={() => goto(fromParam)}>
			<ArrowLeft class="h-4 w-4" />
			{$t('app.archive.back_to_dashboard')}
		</Button>
	</div>
{/if}

<div class="mb-4 flex flex-col gap-1">
	<h1 class="text-2xl font-bold">{$t('app.archived_emails_page.header')}</h1>
	<p class="text-muted-foreground text-sm" aria-busy={!resultsMatchQuery}>
		{#if !resultsMatchQuery}
			Searching…
		{:else if searchResult.total > 0}
			Showing {resultStart}-{resultEnd} of {searchResult.total} emails in {searchResult.processingTimeMs}
			ms
		{:else}
			No emails found
		{/if}
	</p>
</div>

<form onsubmit={handleApplyFilters} class="mb-5">
	{#if !showAdvanced}
		<div class="bg-card flex items-center gap-1 rounded-xl border p-1.5 shadow-sm">
			<div class="relative min-w-0 flex-1">
				<Search
					class="text-muted-foreground pointer-events-none absolute left-3 top-1/2 h-5 w-5 -translate-y-1/2"
				/>
				<Input
					type="search"
					name="q"
					aria-label="Search everything"
					placeholder="Search everything in your archive"
					bind:value={q}
					oninput={applyDebounced}
					class="h-10 border-0 bg-transparent pl-10 text-base font-normal shadow-none focus-visible:ring-0"
				/>
			</div>
			{#if hasActiveFilters}
				<Button
					href="/mailbox{fromParam ? `?from=${encodeURIComponent(fromParam)}` : ''}"
					variant="ghost"
					size="sm"
					class="gap-2"
				>
					<X class="h-4 w-4" />
					<span class="hidden sm:inline">Clear</span>
				</Button>
			{/if}
			<Button
				type="button"
				variant="ghost"
				class="gap-2"
				onclick={() => setAdvancedMode(true)}
			>
				<SlidersHorizontal class="h-4 w-4" />
				<span class="hidden sm:inline">Advanced</span>
			</Button>
		</div>
	{:else}
		<div class="bg-card rounded-xl border shadow-sm">
			<div
				class="flex min-h-13 items-center justify-between gap-3 border-b px-4 py-2.5 lg:px-5"
			>
				<div class="flex items-center gap-2.5">
					<SlidersHorizontal class="text-muted-foreground h-5 w-5" />
					<div>
						<h2 class="text-sm font-semibold">Advanced search</h2>
						<p class="text-muted-foreground text-xs">
							Search specific parts of an email
						</p>
					</div>
				</div>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					onclick={() => setAdvancedMode(false)}
				>
					<X class="h-4 w-4" />
					<span class="sr-only">Close advanced search</span>
				</Button>
			</div>

			<div class="grid gap-5 p-4 lg:grid-cols-[minmax(0,1.35fr)_minmax(20rem,1fr)] lg:p-5">
				<fieldset class="min-w-0">
					<legend
						class="text-muted-foreground mb-2 text-xs font-semibold tracking-wide uppercase"
					>
						Email fields
					</legend>
					<div class="divide-y rounded-lg border">
						<label class="grid min-h-11 grid-cols-[6.5rem_1fr] items-center px-3">
							<span class="text-muted-foreground text-sm font-medium">Sender</span>
							<Input
								type="search"
								name="senderQuery"
								placeholder="Name or email address"
								bind:value={senderQuery}
								oninput={applyDebounced}
								class="h-10 rounded-none border-0 bg-transparent px-0 font-normal shadow-none focus-visible:ring-0"
							/>
						</label>
						<label class="grid min-h-11 grid-cols-[6.5rem_1fr] items-center px-3">
							<span class="text-muted-foreground text-sm font-medium">Recipients</span
							>
							<Input
								type="search"
								name="recipientsQuery"
								placeholder="To, Cc, or Bcc"
								bind:value={recipientsQuery}
								oninput={applyDebounced}
								class="h-10 rounded-none border-0 bg-transparent px-0 font-normal shadow-none focus-visible:ring-0"
							/>
						</label>
						<label class="grid min-h-11 grid-cols-[6.5rem_1fr] items-center px-3">
							<span class="text-muted-foreground text-sm font-medium">Subject</span>
							<Input
								type="search"
								name="subjectQuery"
								placeholder="Words in the subject line"
								bind:value={subjectQuery}
								oninput={applyDebounced}
								class="h-10 rounded-none border-0 bg-transparent px-0 font-normal shadow-none focus-visible:ring-0"
							/>
						</label>
						<label class="grid min-h-11 grid-cols-[6.5rem_1fr] items-center px-3">
							<span class="text-muted-foreground text-sm font-medium">Body</span>
							<Input
								type="search"
								name="bodyQuery"
								placeholder="Words in the message"
								bind:value={bodyQuery}
								oninput={applyDebounced}
								class="h-10 rounded-none border-0 bg-transparent px-0 font-normal shadow-none focus-visible:ring-0"
							/>
						</label>
					</div>
				</fieldset>

				<fieldset class="min-w-0">
					<legend
						class="text-muted-foreground mb-2 text-xs font-semibold tracking-wide uppercase"
					>
						Refine results
					</legend>
					<div class="grid gap-3 sm:grid-cols-2">
						<label class="flex min-w-0 flex-col gap-1.5 text-sm font-medium">
							<span>Source</span>
							<Select.Root
								type="single"
								name="ingestionSourceId"
								bind:value={ingestionSourceId}
								onValueChange={(value) => applyNow({ ingestionSourceId: value })}
							>
								<Select.Trigger class="w-full">{sourceLabel}</Select.Trigger>
								<Select.Content>
									{#each sourceOptions as option (option.value)}
										<Select.Item value={option.value} label={option.label}
											>{option.label}</Select.Item
										>
									{/each}
								</Select.Content>
							</Select.Root>
						</label>

						<label class="flex min-w-0 flex-col gap-1.5 text-sm font-medium">
							<span>Tags</span>
							<SearchableSelect
								name="tags"
								bind:value={tags}
								options={tagFilterOptions}
								placeholder="Any tag"
								onValueChange={(value) => applyNow({ tags: value })}
							/>
						</label>

						<label class="flex min-w-0 flex-col gap-1.5 text-sm font-medium">
							<span>Attachment type</span>
							<AttachmentTypeFilter
								bind:value={attachmentExt}
								onValueChange={(value) => applyNow({ attachmentExt: value })}
							/>
						</label>

						<div class="flex min-h-16 flex-col justify-end gap-2 pb-1">
							<div class="flex items-center gap-2">
								<Checkbox
									id="hasAttachments"
									checked={hasAttachments === 'true'}
									onCheckedChange={(checked) => {
										const value = checked === true ? 'true' : 'any';
										hasAttachments = value;
										applyNow({ hasAttachments: value });
									}}
								/>
								<label for="hasAttachments" class="text-sm font-medium"
									>Has attachments</label
								>
							</div>
						</div>
					</div>
				</fieldset>
			</div>

			<div
				class="bg-muted/30 flex flex-wrap items-center justify-between gap-3 rounded-b-xl border-t px-4 py-3 lg:px-5"
			>
				<div class="flex items-center gap-2">
					<span class="text-muted-foreground text-sm">Match</span>
					<span class="text-sm font-medium">all fields</span>
					<Switch
						id="fieldMatch"
						aria-label="Match any field instead of all fields"
						checked={fieldMatch === 'any'}
						onCheckedChange={(checked) => {
							const value: SearchFieldMatch = checked ? 'any' : 'all';
							fieldMatch = value;
							applyNow({ fieldMatch: value });
						}}
					/>
					<label for="fieldMatch" class="text-sm font-medium">any field</label>
				</div>

				<div class="flex items-center gap-2">
					{#if hasActiveFilters}
						<Button
							href="/mailbox{fromParam
								? `?from=${encodeURIComponent(fromParam)}`
								: ''}"
							variant="ghost"
							class="gap-2"
						>
							<X class="h-4 w-4" />
							Clear
						</Button>
					{/if}
					<Button type="submit" class="gap-2">
						<Search class="h-4 w-4" />
						Search
					</Button>
				</div>
			</div>
		</div>
	{/if}
</form>

{#if selectedVisibleIds.length > 0}
	<div
		class="mb-4 flex flex-col gap-3 rounded-md border p-3 xl:flex-row xl:items-end xl:justify-between"
	>
		<div class="flex flex-col gap-1">
			<div class="text-sm font-medium">{selectedVisibleIds.length} selected</div>
		</div>
		<div class="flex flex-col gap-2 lg:flex-row lg:items-end">
			<label class="flex min-w-56 flex-col gap-1 text-sm font-medium">
				<span>Add tags</span>
				<Input
					type="text"
					list="archive-visible-tags"
					placeholder="taxes, receipts"
					bind:value={tagInput}
				/>
			</label>
			<label class="flex min-w-56 flex-col gap-1 text-sm font-medium">
				<span>Remove tags</span>
				<Input
					type="text"
					list="archive-visible-tags"
					placeholder="old-tag"
					bind:value={removeTagInput}
				/>
			</label>
			<datalist id="archive-visible-tags">
				{#each visibleTags as tag (tag)}
					<option value={tag}></option>
				{/each}
			</datalist>
			<Button
				type="button"
				variant="outline"
				class="gap-2"
				disabled={isUpdatingTags || (!tagInput.trim() && !removeTagInput.trim())}
				onclick={updateSelectedTags}
			>
				<TagsIcon class="h-4 w-4" />
				{isUpdatingTags ? 'Updating...' : 'Apply tags'}
			</Button>
			<Button
				type="button"
				variant="destructive"
				class="gap-2"
				disabled={isDeleting}
				onclick={() => (isDeleteDialogOpen = true)}
			>
				<Trash2 class="h-4 w-4" />
				{isDeleting ? 'Deleting...' : 'Delete'}
			</Button>
		</div>
	</div>
{/if}

{#snippet sortHeader(field: ArchiveSortField, label: string)}
	<button
		type="button"
		class="hover:text-foreground -ml-2 inline-flex items-center gap-1 rounded px-2 py-1 font-medium"
		onclick={() => toggleSort(field)}
	>
		{label}
		{#if sort === field}
			{#if direction === 'asc'}
				<ArrowUp class="h-3.5 w-3.5" />
			{:else}
				<ArrowDown class="h-3.5 w-3.5" />
			{/if}
		{:else}
			<ChevronsUpDown class="h-3.5 w-3.5 opacity-40" />
		{/if}
	</button>
{/snippet}

<div class="rounded-md border">
	<Table.Root>
		<Table.Header>
			<Table.Row>
				<Table.Head class="w-12">
					<Checkbox
						checked={selectionState}
						onCheckedChange={toggleVisibleSelection}
						aria-label="Select visible emails"
					/>
				</Table.Head>
				<Table.Head
					>{@render sortHeader('sentAt', $t('app.archived_emails_page.date'))}</Table.Head
				>
				<Table.Head
					>{@render sortHeader(
						'subject',
						$t('app.archived_emails_page.subject')
					)}</Table.Head
				>
				<Table.Head
					>{@render sortHeader('sender', $t('app.archived_emails_page.from'))}</Table.Head
				>
				<Table.Head>{$t('app.archived_emails_page.to')}</Table.Head>
				<Table.Head>{$t('app.archived_emails_page.import_source')}</Table.Head>
			</Table.Row>
		</Table.Header>
		<Table.Body class="text-sm">
			{#if searchResult.hits.length > 0}
				{#each searchResult.hits as email (email.id)}
					<Table.Row
						class="hover:bg-muted/50 cursor-pointer {email.id === $lastOpenedEmailId
							? 'bg-primary/10'
							: ''}"
						onclick={() => goto(`/mailbox/${email.id}`)}
					>
						<!-- Checkbox cell: stop propagation so toggling selection doesn't open the email -->
						<Table.Cell onclick={(e) => e.stopPropagation()}>
							<Checkbox
								checked={selectedIds.includes(email.id)}
								onCheckedChange={() => toggleEmailSelection(email.id)}
								aria-label={`Select ${email.subject || 'email'}`}
							/>
						</Table.Cell>
						<Table.Cell class="whitespace-nowrap">
							{@const dateInfo = describeDate(email.timestamp, email.timestampKind)}
							{#if dateInfo.label === 'Received'}<span class="text-muted-foreground"
									>Received
								</span>{/if}{dateInfo.text}{#if dateInfo.qualifier}<span
									class="text-muted-foreground"
									title={dateInfo.qualifier}
								>
									(tz?)</span
								>{/if}
						</Table.Cell>

						<Table.Cell>
							<div class="flex max-w-80 items-center gap-2 truncate">
								{#if email.hasAttachments}
									<Paperclip
										class="text-muted-foreground h-4 w-4 flex-shrink-0"
										aria-label="Has attachments"
									/>
								{/if}
								<a class="truncate font-medium" href={`/mailbox/${email.id}`}>
									{email.subject || '(no subject)'}
								</a>
							</div>
							{#if email.tags && email.tags.length > 0}
								<div class="mt-1 flex max-w-80 flex-wrap gap-1">
									{#each email.tags.slice(0, 3) as tag (tag)}
										<span class="bg-muted rounded px-1.5 py-0.5 text-xs"
											>{tag}</span
										>
									{/each}
									{#if email.tags.length > 3}
										<span class="text-muted-foreground text-xs"
											>+{email.tags.length - 3}</span
										>
									{/if}
								</div>
							{/if}
						</Table.Cell>
						<Table.Cell>
							<EmailIdentity
								email={email.from}
								fallbackName={email.senderName}
								class="max-w-48"
							/>
						</Table.Cell>
						<Table.Cell>
							{#if Array.isArray(email.to) && email.to.length > 0}
								<div class="flex max-w-48 flex-col gap-1">
									{#each email.to.slice(0, 2) as recip (recip)}
										<EmailIdentity email={recip} />
									{/each}
									{#if email.to.length > 2}
										<span class="text-muted-foreground text-xs"
											>+{email.to.length - 2} more</span
										>
									{/if}
								</div>
							{:else}
								<span class="text-muted-foreground">—</span>
							{/if}
						</Table.Cell>
						<Table.Cell>
							<span class="block max-w-52 truncate">{email.importSource}</span>
						</Table.Cell>
					</Table.Row>
				{/each}
			{:else}
				<Table.Row>
					<Table.Cell colspan={6} class="h-24 text-center">
						{$t('app.archived_emails_page.no_emails_found')}
					</Table.Cell>
				</Table.Row>
			{/if}
		</Table.Body>
	</Table.Root>
</div>

<div class="mt-4 flex flex-wrap items-center justify-between gap-3">
	<label class="text-muted-foreground flex items-center gap-2 text-sm">
		<span>Rows per page</span>
		<Select.Root type="single" value={limit} onValueChange={changePageSize}>
			<Select.Trigger class="h-8 w-[4.5rem]">{limit}</Select.Trigger>
			<Select.Content>
				{#each limitOptions as option (option.value)}
					<Select.Item value={option.value} label={option.value}
						>{option.value}</Select.Item
					>
				{/each}
			</Select.Content>
		</Select.Root>
	</label>
	<TablePagination
		count={searchResult.total}
		perPage={searchResult.limit}
		page={searchResult.page}
		buildHref={buildArchiveUrl}
		prevLabel={$t('app.archived_emails_page.prev')}
		nextLabel={$t('app.archived_emails_page.next')}
	/>
</div>

<Dialog.Root bind:open={isDeleteDialogOpen}>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title
				>Move {selectedVisibleIds.length} selected email{selectedVisibleIds.length === 1
					? ''
					: 's'} to the Trash?</Dialog.Title
			>
			<Dialog.Description>
				This moves the selected email{selectedVisibleIds.length === 1 ? '' : 's'} to the Trash,
				where
				{selectedVisibleIds.length === 1 ? 'it' : 'they'} can be restored or permanently deleted.
			</Dialog.Description>
		</Dialog.Header>
		<Dialog.Footer class="sm:justify-start">
			<Button
				type="button"
				variant="destructive"
				disabled={isDeleting}
				onclick={deleteSelectedEmails}
			>
				{isDeleting
					? 'Deleting...'
					: `Delete ${selectedVisibleIds.length} email${selectedVisibleIds.length === 1 ? '' : 's'}`}
			</Button>
			<Dialog.Close>
				<Button type="button" variant="secondary">Cancel</Button>
			</Dialog.Close>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
