<script lang="ts">
	import type { PageData } from './$types';
	import * as Table from '$lib/components/ui/table';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import * as Select from '$lib/components/ui/select';
	import { goto } from '$app/navigation';
	import { api } from '$lib/api.client';
	import { t } from '$lib/translations';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import * as Pagination from '$lib/components/ui/pagination/index.js';
	import ChevronLeft from 'lucide-svelte/icons/chevron-left';
	import ChevronRight from 'lucide-svelte/icons/chevron-right';
	import Paperclip from 'lucide-svelte/icons/paperclip';
	import FolderInput from 'lucide-svelte/icons/folder-input';
	import Search from 'lucide-svelte/icons/search';
	import TagsIcon from 'lucide-svelte/icons/tags';
	import X from 'lucide-svelte/icons/x';
	import type {
		ArchiveSortField,
		MatchingStrategy,
		MoveArchivedEmailsResult,
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
	let folders = $derived(data.folders);
	let searchResult = $derived(data.searchResult);
	let filters = $derived(data.filters);

	let q = $derived(filters.q);
	let fields = $derived(filters.fields);
	let ingestionSourceId = $derived(filters.ingestionSourceId);
	let hasAttachments = $derived(filters.hasAttachments);
	let sourcePath = $derived(filters.sourcePath);
	let localFolderPath = $derived(filters.localFolderPath);
	let tags = $derived(filters.tags);
	let sort: ArchiveSortField = $derived(filters.sort);
	let direction: SortDirection = $derived(filters.direction);
	let limit = $derived(String(filters.limit));
	let matchingStrategy: MatchingStrategy = $derived(filters.matchingStrategy);
	let selectedIds = $state<string[]>([]);
	let moveFolderPath = $state('');
	let isMoving = $state(false);
	let tagInput = $state('');
	let removeTagInput = $state('');
	let isUpdatingTags = $state(false);

	const fieldOptions: SelectOption[] = [
		{ value: 'all', label: 'All indexed fields' },
		{ value: 'subject', label: 'Subject' },
		{ value: 'body', label: 'Body' },
		{ value: 'from,senderName', label: 'Sender' },
		{ value: 'to,cc,bcc', label: 'Recipients' },
		{ value: 'attachments.filename,attachments.content', label: 'Attachments' },
		{ value: 'sourcePath,sourceLabels,localFolderPath,tags', label: 'Folders, labels, tags' },
	];

	const attachmentOptions: SelectOption[] = [
		{ value: 'any', label: 'Any attachments' },
		{ value: 'true', label: 'Has attachments' },
		{ value: 'false', label: 'No attachments' },
	];

	const sortOptions: SelectOption[] = [
		{ value: 'sentAt', label: 'Sent date' },
		{ value: 'archivedAt', label: 'Archived date' },
		{ value: 'sender', label: 'Sender' },
		{ value: 'subject', label: 'Subject' },
		{ value: 'sizeBytes', label: 'Size' },
	];

	const directionOptions: SelectOption[] = [
		{ value: 'desc', label: 'Newest first' },
		{ value: 'asc', label: 'Oldest first' },
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
	const sortLabel = $derived(getOptionLabel(sortOptions, sort, 'Sent date'));
	const directionLabel = $derived(getOptionLabel(directionOptions, direction, 'Newest first'));
	const limitLabel = $derived(getOptionLabel(limitOptions, limit, '25 per page'));
	const matchingLabel = $derived(getOptionLabel(matchingOptions, matchingStrategy, 'Fuzzy'));
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
		setParam(params, 'sourcePath', sourcePath);
		setParam(params, 'localFolderPath', localFolderPath);
		setParam(params, 'tags', tags);
		setParam(params, 'sort', sort, 'sentAt');
		setParam(params, 'direction', direction, 'desc');
		setParam(params, 'limit', limit, '25');
		setParam(params, 'matchingStrategy', matchingStrategy, 'last');
		if (pageNumber > 1) {
			params.set('page', String(pageNumber));
		}
		const query = params.toString();
		return `/dashboard/archived-emails${query ? `?${query}` : ''}`;
	}

	function handleApplyFilters(event: SubmitEvent) {
		event.preventDefault();
		selectedIds = [];
		goto(buildArchiveUrl(1), { keepFocus: true });
	}

	function formatTimestamp(timestamp: SearchHit['timestamp']): string {
		return new Date(timestamp).toLocaleString();
	}

	function formatSender(hit: SearchHit): string {
		return hit.senderName || hit.from || '';
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

	async function moveSelectedEmails() {
		const emailIds = selectedVisibleIds;
		const localFolderPath = moveFolderPath.trim();
		if (emailIds.length === 0 || !localFolderPath) return;

		isMoving = true;
		try {
			const response = await api('/archived-emails/bulk/move', {
				method: 'POST',
				body: JSON.stringify({ emailIds, localFolderPath }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to move emails');
			}

			const result = body as MoveArchivedEmailsResult;
			searchResult = {
				...searchResult,
				hits: searchResult.hits.map((email) =>
					emailIds.includes(email.id)
						? {
								...email,
								localFolderId: result.folder.id,
								localFolderPath: result.folder.path,
							}
						: email
				),
			};

			if (!folders.some((folder) => folder.id === result.folder.id)) {
				folders = [...folders, result.folder].sort((a, b) => a.path.localeCompare(b.path));
			}

			selectedIds = [];
			moveFolderPath = '';
			setAlert({
				type: 'success',
				title: 'Emails moved',
				message: `${result.movedCount} email${result.movedCount === 1 ? '' : 's'} moved to ${result.folder.path}`,
				duration: 3000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Move failed',
				message: error instanceof Error ? error.message : 'Failed to move emails',
				duration: 5000,
				show: true,
			});
		} finally {
			isMoving = false;
		}
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

<form onsubmit={handleApplyFilters} class="mb-4 rounded-md border p-3">
	<div class="grid gap-3 md:grid-cols-2 xl:grid-cols-6">
		<label class="flex flex-col gap-1 text-sm font-medium xl:col-span-2">
			<span>Search</span>
			<Input
				type="search"
				name="q"
				placeholder={$t('app.search.placeholder')}
				bind:value={q}
			/>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium">
			<span>Field</span>
			<Select.Root type="single" name="fields" bind:value={fields}>
				<Select.Trigger class="w-full">
					{fieldLabel}
				</Select.Trigger>
				<Select.Content>
					{#each fieldOptions as option (option.value)}
						<Select.Item value={option.value} label={option.label}>
							{option.label}
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium">
			<span>Source</span>
			<Select.Root type="single" name="ingestionSourceId" bind:value={ingestionSourceId}>
				<Select.Trigger class="w-full">
					{sourceLabel}
				</Select.Trigger>
				<Select.Content>
					{#each sourceOptions as option (option.value)}
						<Select.Item value={option.value} label={option.label}>
							{option.label}
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium">
			<span>Attachments</span>
			<Select.Root type="single" name="hasAttachments" bind:value={hasAttachments}>
				<Select.Trigger class="w-full">
					{attachmentLabel}
				</Select.Trigger>
				<Select.Content>
					{#each attachmentOptions as option (option.value)}
						<Select.Item value={option.value} label={option.label}>
							{option.label}
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium">
			<span>Match</span>
			<Select.Root type="single" name="matchingStrategy" bind:value={matchingStrategy}>
				<Select.Trigger class="w-full">
					{matchingLabel}
				</Select.Trigger>
				<Select.Content>
					{#each matchingOptions as option (option.value)}
						<Select.Item value={option.value} label={option.label}>
							{option.label}
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium xl:col-span-2">
			<span>Imported folder</span>
			<Input
				type="search"
				name="sourcePath"
				placeholder="INBOX/Receipts"
				bind:value={sourcePath}
			/>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium xl:col-span-2">
			<span>Local folder</span>
			<Input
				type="search"
				name="localFolderPath"
				placeholder="Imports/account/INBOX"
				bind:value={localFolderPath}
			/>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium xl:col-span-2">
			<span>Tags</span>
			<Input type="search" name="tags" placeholder="taxes, receipts" bind:value={tags} />
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium">
			<span>Sort</span>
			<Select.Root type="single" name="sort" bind:value={sort}>
				<Select.Trigger class="w-full">
					{sortLabel}
				</Select.Trigger>
				<Select.Content>
					{#each sortOptions as option (option.value)}
						<Select.Item value={option.value} label={option.label}>
							{option.label}
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium">
			<span>Order</span>
			<Select.Root type="single" name="direction" bind:value={direction}>
				<Select.Trigger class="w-full">
					{directionLabel}
				</Select.Trigger>
				<Select.Content>
					{#each directionOptions as option (option.value)}
						<Select.Item value={option.value} label={option.label}>
							{option.label}
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>

		<label class="flex flex-col gap-1 text-sm font-medium">
			<span>Page size</span>
			<Select.Root type="single" name="limit" bind:value={limit}>
				<Select.Trigger class="w-full">
					{limitLabel}
				</Select.Trigger>
				<Select.Content>
					{#each limitOptions as option (option.value)}
						<Select.Item value={option.value} label={option.label}>
							{option.label}
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>
	</div>

	<div class="mt-3 flex flex-wrap items-center gap-2">
		<Button type="submit" class="gap-2">
			<Search class="h-4 w-4" />
			{$t('app.search.search_button')}
		</Button>
		<a href="/dashboard/archived-emails">
			<Button type="button" variant="outline" class="gap-2">
				<X class="h-4 w-4" />
				Clear
			</Button>
		</a>
	</div>
</form>

{#if selectedVisibleIds.length > 0}
	<div
		class="mb-4 flex flex-col gap-3 rounded-md border p-3 xl:flex-row xl:items-end xl:justify-between"
	>
		<div class="flex flex-col gap-1">
			<div class="text-sm font-medium">{selectedVisibleIds.length} selected</div>
		</div>
		<div class="flex flex-col gap-2 lg:flex-row lg:items-end">
			<label class="flex min-w-72 flex-col gap-1 text-sm font-medium">
				<span>Move to folder</span>
				<Input
					type="text"
					list="archive-folder-paths"
					placeholder="Projects/Receipts"
					bind:value={moveFolderPath}
				/>
			</label>
			<datalist id="archive-folder-paths">
				{#each folders as folder (folder.id)}
					<option value={folder.path}></option>
				{/each}
			</datalist>
			<Button
				type="button"
				class="gap-2"
				disabled={isMoving || !moveFolderPath.trim()}
				onclick={moveSelectedEmails}
			>
				<FolderInput class="h-4 w-4" />
				{isMoving ? 'Moving...' : 'Move'}
			</Button>
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
		</div>
	</div>
{/if}

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
				<Table.Head>{$t('app.archived_emails_page.date')}</Table.Head>
				<Table.Head>{$t('app.archived_emails_page.subject')}</Table.Head>
				<Table.Head>{$t('app.archived_emails_page.sender')}</Table.Head>
				<Table.Head>{$t('app.archived_emails_page.inbox')}</Table.Head>
				<Table.Head>Local folder</Table.Head>
				<Table.Head>{$t('app.archived_emails_page.path')}</Table.Head>
				<Table.Head class="text-right">{$t('app.archived_emails_page.actions')}</Table.Head>
			</Table.Row>
		</Table.Header>
		<Table.Body class="text-sm">
			{#if searchResult.hits.length > 0}
				{#each searchResult.hits as email (email.id)}
					<Table.Row>
						<Table.Cell>
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
								<a
									class="link truncate"
									href={`/dashboard/archived-emails/${email.id}`}
								>
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
							<span class="block max-w-48 truncate">{formatSender(email)}</span>
						</Table.Cell>
						<Table.Cell>
							<span class="block max-w-52 truncate">{email.userEmail}</span>
						</Table.Cell>
						<Table.Cell>
							{#if email.localFolderPath}
								<span
									class="bg-muted block max-w-56 truncate rounded p-1.5 text-xs"
								>
									{email.localFolderPath}
								</span>
							{/if}
						</Table.Cell>
						<Table.Cell>
							{#if email.sourcePath}
								<span
									class="bg-muted block max-w-52 truncate rounded p-1.5 text-xs"
								>
									{email.sourcePath}
								</span>
							{/if}
						</Table.Cell>
						<Table.Cell class="text-right">
							<a href={`/dashboard/archived-emails/${email.id}`}>
								<Button variant="outline"
									>{$t('app.archived_emails_page.view')}</Button
								>
							</a>
						</Table.Cell>
					</Table.Row>
				{/each}
			{:else}
				<Table.Row>
					<Table.Cell colspan={8} class="h-24 text-center">
						{$t('app.archived_emails_page.no_emails_found')}
					</Table.Cell>
				</Table.Row>
			{/if}
		</Table.Body>
	</Table.Root>
</div>

{#if searchResult.total > searchResult.limit}
	<div class="mt-8">
		<Pagination.Root
			count={searchResult.total}
			perPage={searchResult.limit}
			page={searchResult.page}
		>
			{#snippet children({ pages, currentPage })}
				<Pagination.Content>
					<Pagination.Item>
						<a href={buildArchiveUrl(currentPage - 1)}>
							<Pagination.PrevButton>
								<ChevronLeft class="h-4 w-4" />
								<span class="hidden sm:block"
									>{$t('app.archived_emails_page.prev')}</span
								>
							</Pagination.PrevButton>
						</a>
					</Pagination.Item>
					{#each pages as page (page.key)}
						{#if page.type === 'ellipsis'}
							<Pagination.Item>
								<Pagination.Ellipsis />
							</Pagination.Item>
						{:else}
							<Pagination.Item>
								<a href={buildArchiveUrl(page.value)}>
									<Pagination.Link {page} isActive={currentPage === page.value}>
										{page.value}
									</Pagination.Link>
								</a>
							</Pagination.Item>
						{/if}
					{/each}
					<Pagination.Item>
						<a href={buildArchiveUrl(currentPage + 1)}>
							<Pagination.NextButton>
								<span class="hidden sm:block"
									>{$t('app.archived_emails_page.next')}</span
								>
								<ChevronRight class="h-4 w-4" />
							</Pagination.NextButton>
						</a>
					</Pagination.Item>
				</Pagination.Content>
			{/snippet}
		</Pagination.Root>
	</div>
{/if}
