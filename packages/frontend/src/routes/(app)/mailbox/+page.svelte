<script lang="ts">
	import type { PageData } from './$types';
	import * as Table from '$lib/components/ui/table';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import * as Select from '$lib/components/ui/select';
	import * as Dialog from '$lib/components/ui/dialog';
	import SearchableSelect from '$lib/components/custom/SearchableSelect.svelte';
	import EmailIdentity from '$lib/components/custom/EmailIdentity.svelte';
	import { formatDateTime } from '$lib/stores/datetime.svelte';
	import { goto } from '$app/navigation';
	import { page as appPage } from '$app/state';
	import { lastMailboxListUrl } from '$lib/stores/mailbox-nav';
	import { api } from '$lib/api.client';
	import { t } from '$lib/translations';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import TablePagination from '$lib/components/custom/TablePagination.svelte';
	import Paperclip from 'lucide-svelte/icons/paperclip';
	import Search from 'lucide-svelte/icons/search';
	import Filter from 'lucide-svelte/icons/filter';
	import ArrowUp from 'lucide-svelte/icons/arrow-up';
	import ArrowDown from 'lucide-svelte/icons/arrow-down';
	import ChevronsUpDown from 'lucide-svelte/icons/chevrons-up-down';
	import TagsIcon from 'lucide-svelte/icons/tags';
	import Trash2 from 'lucide-svelte/icons/trash-2';
	import X from 'lucide-svelte/icons/x';
	import type {
		ArchiveSortField,
		BulkDeleteArchivedEmailsResult,
		MatchingStrategy,
		SearchHit,
		SortDirection,
		UpdateArchivedEmailTagsResult,
	} from '@open-archiver/types';

	type SelectOption = {
		value: string;
		label: string;
	};

	let { data }: { data: PageData } = $props();

	let ingestionSources = $derived(data.ingestionSources);
	let searchResult = $derived(data.searchResult);
	let filters = $derived(data.filters);

	let q = $derived(filters.q);
	let fields = $derived(filters.fields);
	let ingestionSourceId = $derived(filters.ingestionSourceId);
	let hasAttachments = $derived(filters.hasAttachments);
	let tags = $derived(filters.tags);
	let sort: ArchiveSortField = $derived(filters.sort);
	let direction: SortDirection = $derived(filters.direction);
	let limit = $derived(String(filters.limit));
	let matchingStrategy: MatchingStrategy = $derived(filters.matchingStrategy);

	// The Clear button resets to the pristine /mailbox view, so it should show
	// whenever any filter/search is active. buildArchiveUrl() only writes a query
	// param when its value deviates from the default, so "any param present that
	// isn't display/pagination" is exactly the set of active filters. Listing the
	// non-filter params (rather than the filters) means a filter added to
	// buildArchiveUrl() in the future lights up Clear automatically — only a new
	// display/pagination param would need to be added to this set.
	const DISPLAY_PARAMS = new Set(['sort', 'direction', 'limit', 'page']);
	let hasActiveFilters = $derived(
		[...appPage.url.searchParams.keys()].some((key) => !DISPLAY_PARAMS.has(key))
	);
	// Result filters (source / attachments / tags) live in a collapsible panel —
	// they narrow results even with no search term. Field/Match (which refine the
	// query itself) stay inline in the search bar; sort/order move to the table
	// headers and page size lives by the pagination. Open the panel on load if any
	// of its filters is already applied.
	let showFilters = $state(
		data.filters.ingestionSourceId !== 'all' ||
			data.filters.hasAttachments !== 'any' ||
			Boolean(data.filters.tags)
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

	const fieldOptions: SelectOption[] = [
		{ value: 'all', label: 'All indexed fields' },
		{ value: 'subject', label: 'Subject' },
		{ value: 'body', label: 'Body' },
		{ value: 'from,senderName', label: 'Sender' },
		{ value: 'to,cc,bcc', label: 'Recipients' },
		{ value: 'attachments.filename,attachments.content', label: 'Attachments' },
		{ value: 'sourcePath,sourceLabels,tags', label: 'Tags' },
	];

	const attachmentOptions: SelectOption[] = [
		{ value: 'any', label: 'Any attachments' },
		{ value: 'true', label: 'Has attachments' },
		{ value: 'false', label: 'No attachments' },
	];

	const limitOptions: SelectOption[] = [
		{ value: '25', label: '25 per page' },
		{ value: '50', label: '50 per page' },
		{ value: '100', label: '100 per page' },
	];

	const matchingOptions: SelectOption[] = [
		{ value: 'last', label: 'Fuzzy' },
		{ value: 'all', label: 'Verbatim' },
		{ value: 'frequency', label: 'Frequency' },
	];

	let sourceOptions = $derived([
		{ value: 'all', label: 'All sources' },
		...ingestionSources.map((source) => ({ value: source.id, label: source.name })),
	]);

	const sourceLabel = $derived(getOptionLabel(sourceOptions, ingestionSourceId, 'All sources'));
	const fieldLabel = $derived(getOptionLabel(fieldOptions, fields, 'All indexed fields'));
	const attachmentLabel = $derived(
		getOptionLabel(attachmentOptions, hasAttachments, 'Any attachments')
	);
	const matchingLabel = $derived(getOptionLabel(matchingOptions, matchingStrategy, 'Fuzzy'));

	// Count of active result filters (the ones in the panel), shown as a badge.
	const advancedActiveCount = $derived(
		(ingestionSourceId !== 'all' ? 1 : 0) + (hasAttachments !== 'any' ? 1 : 0) + (tags ? 1 : 0)
	);

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
	const resultStart = $derived(
		searchResult.total === 0 ? 0 : (searchResult.page - 1) * searchResult.limit + 1
	);
	const resultEnd = $derived(
		Math.min(searchResult.page * searchResult.limit, searchResult.total)
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

	function buildArchiveUrl(pageNumber = 1): string {
		const params = new URLSearchParams();
		setParam(params, 'q', q);
		setParam(params, 'fields', fields, 'all');
		setParam(params, 'ingestionSourceId', ingestionSourceId, 'all');
		setParam(params, 'hasAttachments', hasAttachments, 'any');
		setParam(params, 'tags', tags);
		setParam(params, 'sort', sort, 'sentAt');
		setParam(params, 'direction', direction, 'desc');
		setParam(params, 'limit', limit, '25');
		setParam(params, 'matchingStrategy', matchingStrategy, 'last');
		if (pageNumber > 1) {
			params.set('page', String(pageNumber));
		}
		const query = params.toString();
		return `/mailbox${query ? `?${query}` : ''}`;
	}

	function handleApplyFilters(event: SubmitEvent) {
		event.preventDefault();
		selectedIds = [];
		goto(buildArchiveUrl(1), { keepFocus: true });
	}

	function formatTimestamp(timestamp: SearchHit['timestamp']): string {
		return formatDateTime(timestamp);
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
			const deleted = new Set(result.deletedIds);
			searchResult = {
				...searchResult,
				hits: searchResult.hits.filter((email) => !deleted.has(email.id)),
				total: Math.max(0, searchResult.total - result.deletedCount),
			};
			selectedIds = selectedIds.filter((id) => !deleted.has(id));
			isDeleteDialogOpen = false;

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
	<title>{$t('app.archived_emails_page.title')} - OpenArchiver</title>
</svelte:head>

<div class="mb-4 flex flex-col gap-1">
	<h1 class="text-2xl font-bold">{$t('app.archived_emails_page.header')}</h1>
	<p class="text-muted-foreground text-sm">
		{#if searchResult.total > 0}
			Showing {resultStart}-{resultEnd} of {searchResult.total} emails in {searchResult.processingTimeMs}
			ms
		{:else}
			No emails found
		{/if}
	</p>
</div>

<form onsubmit={handleApplyFilters} class="mb-4 space-y-3">
	<!-- Search bar: query + the two search-refinement controls (field scope & match mode) -->
	<div class="flex flex-wrap items-center gap-2">
		<div class="relative min-w-[12rem] flex-1 sm:max-w-md">
			<Search
				class="text-muted-foreground pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2"
			/>
			<Input
				type="search"
				name="q"
				placeholder={$t('app.search.placeholder')}
				bind:value={q}
				class="pl-9"
			/>
		</div>
		<Select.Root type="single" name="fields" bind:value={fields}>
			<Select.Trigger class="w-[10.5rem]" title="Search scope">{fieldLabel}</Select.Trigger>
			<Select.Content>
				{#each fieldOptions as option (option.value)}
					<Select.Item value={option.value} label={option.label}
						>{option.label}</Select.Item
					>
				{/each}
			</Select.Content>
		</Select.Root>
		<Select.Root type="single" name="matchingStrategy" bind:value={matchingStrategy}>
			<Select.Trigger class="w-[8rem]" title="Match mode">{matchingLabel}</Select.Trigger>
			<Select.Content>
				{#each matchingOptions as option (option.value)}
					<Select.Item value={option.value} label={option.label}
						>{option.label}</Select.Item
					>
				{/each}
			</Select.Content>
		</Select.Root>
		<Button type="submit" class="gap-2">
			<Search class="h-4 w-4" />
			{$t('app.search.search_button')}
		</Button>
		<Button
			type="button"
			variant="outline"
			class="gap-2"
			aria-expanded={showFilters}
			onclick={() => (showFilters = !showFilters)}
		>
			<Filter class="h-4 w-4" />
			Filters
			{#if advancedActiveCount > 0}
				<span
					class="bg-primary text-primary-foreground inline-flex h-5 min-w-[1.25rem] items-center justify-center rounded-full px-1 text-xs font-semibold"
				>
					{advancedActiveCount}
				</span>
			{/if}
		</Button>
		{#if hasActiveFilters}
			<a href="/mailbox">
				<Button type="button" variant="ghost" class="gap-2">
					<X class="h-4 w-4" />
					Clear
				</Button>
			</a>
		{/if}
	</div>

	{#if showFilters}
		<div class="flex flex-wrap items-end gap-4 rounded-md border p-3">
			<label class="flex min-w-[12rem] flex-col gap-1 text-sm font-medium">
				<span>Source</span>
				<Select.Root type="single" name="ingestionSourceId" bind:value={ingestionSourceId}>
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

			<label class="flex min-w-[12rem] flex-col gap-1 text-sm font-medium">
				<span>Tags</span>
				<SearchableSelect
					name="tags"
					bind:value={tags}
					options={tagFilterOptions}
					placeholder="Any tag"
				/>
			</label>

			<div class="flex h-9 items-center gap-2">
				<Checkbox
					id="hasAttachments"
					checked={hasAttachments === 'true'}
					onCheckedChange={(checked) => (hasAttachments = checked ? 'true' : 'any')}
				/>
				<label for="hasAttachments" class="text-sm font-medium">Only with attachments</label
				>
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
				<Table.Head>{$t('app.archived_emails_page.inbox')}</Table.Head>
			</Table.Row>
		</Table.Header>
		<Table.Body class="text-sm">
			{#if searchResult.hits.length > 0}
				{#each searchResult.hits as email (email.id)}
					<Table.Row
						class="hover:bg-muted/50 cursor-pointer"
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
							{formatTimestamp(email.timestamp)}
						</Table.Cell>

						<Table.Cell>
							<div class="flex max-w-80 items-center gap-2 truncate">
								{#if email.hasAttachments}
									<Paperclip
										class="text-muted-foreground h-4 w-4 flex-shrink-0"
										aria-label="Has attachments"
									/>
								{/if}
								<a class="link truncate" href={`/mailbox/${email.id}`}>
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
							<span class="block max-w-52 truncate">{email.userEmail}</span>
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
				>Delete {selectedVisibleIds.length} selected email{selectedVisibleIds.length === 1
					? ''
					: 's'}?</Dialog.Title
			>
			<Dialog.Description>
				This permanently deletes the selected email{selectedVisibleIds.length === 1
					? ''
					: 's'} and any attachments not shared with other emails. This action cannot be undone.
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
