<script lang="ts">
	import type { PageData } from './$types';
	import * as Table from '$lib/components/ui/table';
	import { Button } from '$lib/components/ui/button';
	import { Badge } from '$lib/components/ui/badge';
	import TablePagination from '$lib/components/custom/TablePagination.svelte';
	import { api } from '$lib/api.client';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import Check from 'lucide-svelte/icons/check';
	import CheckCheck from 'lucide-svelte/icons/check-check';
	import type {
		ApproveExactDuplicateGroupDto,
		ApproveExactDuplicatesResult,
		ApproveFuzzyDuplicateGroupDto,
		ApproveFuzzyDuplicatesResult,
		ExactDuplicateGroup,
		ExactDuplicateReason,
		FuzzyDuplicateGroup,
		IgnoreFuzzyDuplicateGroupsResult,
		ScanFuzzyDuplicatesResult,
	} from '@open-archiver/types';

	let { data }: { data: PageData } = $props();

	let duplicateGroups = $derived(data.duplicateGroups);
	let fuzzyDuplicateGroups = $derived(data.fuzzyDuplicateGroups);
	let activeReason = $derived(data.activeReason || '');
	let activeTab = $state<'exact' | 'fuzzy'>('exact');
	let keeperOverrides = $state<Record<string, string>>({});
	let fuzzyKeeperOverrides = $state<Record<string, string>>({});
	let approvingKey = $state<string | null>(null);
	let scanningFuzzy = $state(false);

	const reasonFilters: { value: string; label: string }[] = [
		{ value: '', label: 'All' },
		{ value: 'message_id', label: 'Message-ID' },
		{ value: 'storage_hash', label: 'Raw hash' },
		{ value: 'attachment_hash_set', label: 'Attachment set' },
		{ value: 'sender_recipients_sent', label: 'Sender + recipients + time' },
	];

	function reasonLabel(reason: ExactDuplicateReason): string {
		switch (reason) {
			case 'message_id':
				return 'Message-ID';
			case 'storage_hash':
				return 'Raw hash';
			case 'attachment_hash_set':
				return 'Attachment set';
			case 'sender_recipients_sent':
				return 'Sender + recipients + time';
		}
	}

	function shortFingerprint(fingerprint: string): string {
		return fingerprint.length > 96 ? `${fingerprint.slice(0, 96)}...` : fingerprint;
	}

	function formatDate(value: string | Date): string {
		return new Date(value).toLocaleString();
	}

	function setKeeper(groupKey: string, emailId: string) {
		keeperOverrides = {
			...keeperOverrides,
			[groupKey]: emailId,
		};
	}

	function buildDecision(group: ExactDuplicateGroup): ApproveExactDuplicateGroupDto | null {
		const keeperEmailId = keeperOverrides[group.groupKey] || group.keeperEmailId;
		const duplicateEmailIds = group.emails
			.map((email) => email.id)
			.filter((emailId) => emailId !== keeperEmailId);

		if (!keeperEmailId || duplicateEmailIds.length === 0) {
			return null;
		}

		return {
			groupKey: group.groupKey,
			keeperEmailId,
			duplicateEmailIds,
		};
	}

	function setFuzzyKeeper(groupId: string, emailId: string) {
		fuzzyKeeperOverrides = {
			...fuzzyKeeperOverrides,
			[groupId]: emailId,
		};
	}

	function buildFuzzyDecision(group: FuzzyDuplicateGroup): ApproveFuzzyDuplicateGroupDto | null {
		const keeperEmailId = fuzzyKeeperOverrides[group.id] || group.keeperEmailId;
		const duplicateEmailIds = group.emails
			.map((email) => email.id)
			.filter((emailId) => emailId !== keeperEmailId);

		if (!keeperEmailId || duplicateEmailIds.length === 0) {
			return null;
		}

		return {
			groupId: group.id,
			keeperEmailId,
			duplicateEmailIds,
		};
	}

	async function approveGroups(groups: ExactDuplicateGroup[], actionKey: string) {
		const decisions = groups
			.map(buildDecision)
			.filter((decision): decision is ApproveExactDuplicateGroupDto => Boolean(decision));
		if (decisions.length === 0) return;

		approvingKey = actionKey;
		try {
			const response = await api('/archived-emails/duplicates/exact/approve', {
				method: 'POST',
				body: JSON.stringify({ groups: decisions }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to approve duplicates');
			}

			const result = body as ApproveExactDuplicatesResult;
			const approvedKeys = new Set(decisions.map((decision) => decision.groupKey));
			duplicateGroups = {
				...duplicateGroups,
				groups: duplicateGroups.groups.filter((group) => !approvedKeys.has(group.groupKey)),
				totalGroups: Math.max(0, duplicateGroups.totalGroups - result.approvedGroups),
			};
			setAlert({
				type: 'success',
				title: 'Duplicates deleted',
				message: `${result.deletedEmails} duplicate email${result.deletedEmails === 1 ? '' : 's'} deleted`,
				duration: 3000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Approval failed',
				message: error instanceof Error ? error.message : 'Failed to approve duplicates',
				duration: 5000,
				show: true,
			});
		} finally {
			approvingKey = null;
		}
	}

	async function approveFuzzyGroups(groups: FuzzyDuplicateGroup[], actionKey: string) {
		const decisions = groups
			.map(buildFuzzyDecision)
			.filter((decision): decision is ApproveFuzzyDuplicateGroupDto => Boolean(decision));
		if (decisions.length === 0) return;

		approvingKey = actionKey;
		try {
			const response = await api('/archived-emails/duplicates/fuzzy/approve', {
				method: 'POST',
				body: JSON.stringify({ groups: decisions }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to approve fuzzy duplicates');
			}

			const result = body as ApproveFuzzyDuplicatesResult;
			const approvedIds = new Set(decisions.map((decision) => decision.groupId));
			fuzzyDuplicateGroups = {
				...fuzzyDuplicateGroups,
				groups: fuzzyDuplicateGroups.groups.filter((group) => !approvedIds.has(group.id)),
				totalGroups: Math.max(0, fuzzyDuplicateGroups.totalGroups - result.approvedGroups),
			};
			setAlert({
				type: 'success',
				title: 'Fuzzy duplicates deleted',
				message: `${result.deletedEmails} duplicate email${result.deletedEmails === 1 ? '' : 's'} deleted`,
				duration: 3000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Approval failed',
				message:
					error instanceof Error ? error.message : 'Failed to approve fuzzy duplicates',
				duration: 5000,
				show: true,
			});
		} finally {
			approvingKey = null;
		}
	}

	async function ignoreFuzzyGroup(groupId: string) {
		approvingKey = `ignore-${groupId}`;
		try {
			const response = await api('/archived-emails/duplicates/fuzzy/ignore', {
				method: 'POST',
				body: JSON.stringify({ groupIds: [groupId] }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to ignore fuzzy group');
			}

			const result = body as IgnoreFuzzyDuplicateGroupsResult;
			fuzzyDuplicateGroups = {
				...fuzzyDuplicateGroups,
				groups: fuzzyDuplicateGroups.groups.filter((group) => group.id !== groupId),
				totalGroups: Math.max(0, fuzzyDuplicateGroups.totalGroups - result.ignoredGroups),
			};
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Ignore failed',
				message: error instanceof Error ? error.message : 'Failed to ignore fuzzy group',
				duration: 5000,
				show: true,
			});
		} finally {
			approvingKey = null;
		}
	}

	async function scanFuzzyBatch() {
		scanningFuzzy = true;
		try {
			const response = await api('/archived-emails/duplicates/fuzzy/scan', {
				method: 'POST',
				body: JSON.stringify({ batchSize: 100 }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to enqueue fuzzy scan');
			}

			const result = body as ScanFuzzyDuplicatesResult;
			setAlert({
				type: 'success',
				title: 'Fuzzy scan queued',
				message: `Indexing job ${result.jobId} will scan up to ${result.batchSize} groups.`,
				duration: 5000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Scan failed',
				message: error instanceof Error ? error.message : 'Failed to enqueue fuzzy scan',
				duration: 5000,
				show: true,
			});
		} finally {
			scanningFuzzy = false;
		}
	}

	function buildExactPageUrl(page: number): string {
		const params = new URLSearchParams();
		if (activeReason) params.set('reason', activeReason);
		if (page > 1) params.set('exactPage', String(page));
		const query = params.toString();
		return `/dashboard/duplicates${query ? `?${query}` : ''}`;
	}

	function buildFuzzyPageUrl(page: number): string {
		const params = new URLSearchParams();
		if (activeReason) params.set('reason', activeReason);
		if (page > 1) params.set('fuzzyPage', String(page));
		const query = params.toString();
		return `/dashboard/duplicates${query ? `?${query}` : ''}`;
	}

	/** URL that applies a reason filter (resets exact pagination to page 1). */
	function reasonUrl(reason: string): string {
		const query = reason ? `?reason=${encodeURIComponent(reason)}` : '';
		return `/dashboard/duplicates${query}`;
	}
</script>

<svelte:head>
	<title>Duplicate Review - OpenArchiver</title>
</svelte:head>

<div class="mb-4">
	<h1 class="text-2xl font-bold">Duplicate Review</h1>
	<p class="text-muted-foreground text-sm">
		{duplicateGroups.totalGroups} exact groups, {fuzzyDuplicateGroups.totalGroups} fuzzy groups
	</p>
</div>

<!-- Tabs: exact vs fuzzy -->
<div class="mb-4 flex gap-4 border-b">
	<button
		type="button"
		class="border-b-2 px-1 pb-2 text-sm font-medium {activeTab === 'exact'
			? 'border-primary text-foreground'
			: 'text-muted-foreground border-transparent'}"
		onclick={() => (activeTab = 'exact')}
	>
		Exact matches ({duplicateGroups.totalGroups})
	</button>
	<button
		type="button"
		class="border-b-2 px-1 pb-2 text-sm font-medium {activeTab === 'fuzzy'
			? 'border-primary text-foreground'
			: 'text-muted-foreground border-transparent'}"
		onclick={() => (activeTab = 'fuzzy')}
	>
		Fuzzy matches ({fuzzyDuplicateGroups.totalGroups})
	</button>
</div>

{#if activeTab === 'exact'}
	<div class="mb-3 flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between">
		<div class="flex flex-wrap items-center gap-1">
			<span class="text-muted-foreground mr-1 text-xs">Filter:</span>
			{#each reasonFilters as rf (rf.value)}
				<a
					href={reasonUrl(rf.value)}
					data-sveltekit-noscroll
					class="rounded-full border px-2.5 py-1 text-xs {activeReason === rf.value
						? 'bg-primary text-primary-foreground border-primary'
						: 'text-muted-foreground hover:bg-muted'}"
				>
					{rf.label}
				</a>
			{/each}
		</div>
		{#if duplicateGroups.groups.length > 0}
			<Button
				type="button"
				class="gap-2"
				disabled={approvingKey !== null}
				onclick={() => approveGroups(duplicateGroups.groups, 'page')}
			>
				<CheckCheck class="h-4 w-4" />
				{approvingKey === 'page' ? 'Deleting…' : 'Delete all duplicates on this page'}
			</Button>
		{/if}
	</div>

	{#if duplicateGroups.groups.length > 0}
	<div class="space-y-4">
		{#each duplicateGroups.groups as group (group.groupKey)}
			<section class="rounded-md border">
				<div
					class="flex flex-col gap-2 border-b p-3 lg:flex-row lg:items-center lg:justify-between"
				>
					<div class="min-w-0">
						<div class="flex flex-wrap items-center gap-2">
							{#each group.reasons as r (r)}
								<Badge variant="secondary">{reasonLabel(r)}</Badge>
							{/each}
							<span class="text-sm font-medium">{group.count} emails</span>
						</div>
						<div
							class="text-muted-foreground mt-1 max-w-full truncate font-mono text-xs"
						>
							{shortFingerprint(group.fingerprint)}
						</div>
					</div>
					<Button
						type="button"
						variant="outline"
						class="gap-2"
						disabled={approvingKey !== null}
						onclick={() => approveGroups([group], group.groupKey)}
					>
						<Check class="h-4 w-4" />
						{approvingKey === group.groupKey ? 'Deleting…' : 'Delete duplicates'}
					</Button>
				</div>

				<Table.Root>
					<Table.Header>
						<Table.Row>
							<Table.Head class="w-16">Keep</Table.Head>
							<Table.Head>Sent</Table.Head>
							<Table.Head>Subject</Table.Head>
							<Table.Head>Sender</Table.Head>
							<Table.Head>Inbox</Table.Head>
							<Table.Head>Folder</Table.Head>
							<Table.Head class="text-right">Open</Table.Head>
						</Table.Row>
					</Table.Header>
					<Table.Body class="text-sm">
						{#each group.emails as email (email.id)}
							<Table.Row>
								<Table.Cell>
									<input
										type="radio"
										name={`keeper-${group.groupKey}`}
										checked={(keeperOverrides[group.groupKey] ||
											group.keeperEmailId) === email.id}
										onchange={() => setKeeper(group.groupKey, email.id)}
										aria-label={`Keep ${email.subject || 'email'}`}
									/>
								</Table.Cell>
								<Table.Cell class="whitespace-nowrap"
									>{formatDate(email.sentAt)}</Table.Cell
								>
								<Table.Cell>
									<span class="block max-w-80 truncate"
										>{email.subject || '(no subject)'}</span
									>
								</Table.Cell>
								<Table.Cell>
									<span class="block max-w-48 truncate">
										{email.senderName || email.senderEmail}
									</span>
								</Table.Cell>
								<Table.Cell>
									<span class="block max-w-52 truncate">{email.userEmail}</span>
								</Table.Cell>
								<Table.Cell>
									{#if email.sourcePath}
										<span
											class="bg-muted block max-w-56 truncate rounded p-1.5 text-xs"
										>
											{email.sourcePath}
										</span>
									{/if}
								</Table.Cell>
								<Table.Cell class="text-right">
									<a href={`/mailbox/${email.id}`}>
										<Button variant="outline">View</Button>
									</a>
								</Table.Cell>
							</Table.Row>
						{/each}
					</Table.Body>
				</Table.Root>
			</section>
		{/each}
	</div>
	{:else}
		<div class="rounded-md border p-8 text-center text-sm">
			No exact duplicates{activeReason ? ' for this filter' : ''} found.
		</div>
	{/if}

	<TablePagination
		count={duplicateGroups.totalGroups}
		perPage={duplicateGroups.limit}
		page={duplicateGroups.page}
		buildHref={buildExactPageUrl}
		prevLabel="Prev"
		nextLabel="Next"
	/>
{:else}
	<div class="mb-3 flex flex-wrap items-center justify-end gap-2">
		<Button type="button" variant="outline" disabled={scanningFuzzy} onclick={scanFuzzyBatch}>
			{scanningFuzzy ? 'Queueing...' : 'Scan Fuzzy Batch'}
		</Button>
		{#if fuzzyDuplicateGroups.groups.length > 0}
			<Button
				type="button"
				class="gap-2"
				disabled={approvingKey !== null}
				onclick={() => approveFuzzyGroups(fuzzyDuplicateGroups.groups, 'fuzzy-page')}
			>
				<CheckCheck class="h-4 w-4" />
				{approvingKey === 'fuzzy-page' ? 'Deleting…' : 'Delete all duplicates on this page'}
			</Button>
		{/if}
	</div>

	{#if fuzzyDuplicateGroups.groups.length > 0}
	<div class="space-y-4">
		{#each fuzzyDuplicateGroups.groups as group (group.id)}
			<section class="rounded-md border">
				<div
					class="flex flex-col gap-2 border-b p-3 lg:flex-row lg:items-center lg:justify-between"
				>
					<div class="min-w-0">
						<div class="flex flex-wrap items-center gap-2">
							<Badge variant="secondary">Score {group.score}</Badge>
							<span class="text-sm font-medium">{group.emails.length} emails</span>
							{#if group.signals.matchingBodyHash}
								<Badge variant="outline">Body</Badge>
							{/if}
							{#if group.signals.matchingRecipients}
								<Badge variant="outline">Recipients</Badge>
							{/if}
							{#if group.signals.matchingAttachments}
								<Badge variant="outline">Attachments</Badge>
							{/if}
						</div>
						<div
							class="text-muted-foreground mt-1 max-w-full truncate font-mono text-xs"
						>
							{group.signals.senderEmail} · {shortFingerprint(
								group.signals.subjectHash
							)}
						</div>
					</div>
					<div class="flex flex-wrap gap-2">
						<Button
							type="button"
							variant="outline"
							class="gap-2"
							disabled={approvingKey !== null}
							onclick={() => approveFuzzyGroups([group], group.id)}
						>
							<Check class="h-4 w-4" />
							{approvingKey === group.id ? 'Deleting…' : 'Delete duplicates'}
						</Button>
						<Button
							type="button"
							variant="outline"
							disabled={approvingKey !== null}
							onclick={() => ignoreFuzzyGroup(group.id)}
						>
							{approvingKey === `ignore-${group.id}` ? 'Ignoring...' : 'Ignore'}
						</Button>
					</div>
				</div>

				<Table.Root>
					<Table.Header>
						<Table.Row>
							<Table.Head class="w-16">Keep</Table.Head>
							<Table.Head>Sent</Table.Head>
							<Table.Head>Subject</Table.Head>
							<Table.Head>Sender</Table.Head>
							<Table.Head>Inbox</Table.Head>
							<Table.Head>Folder</Table.Head>
							<Table.Head class="text-right">Open</Table.Head>
						</Table.Row>
					</Table.Header>
					<Table.Body class="text-sm">
						{#each group.emails as email (email.id)}
							<Table.Row>
								<Table.Cell>
									<input
										type="radio"
										name={`fuzzy-keeper-${group.id}`}
										checked={(fuzzyKeeperOverrides[group.id] ||
											group.keeperEmailId) === email.id}
										onchange={() => setFuzzyKeeper(group.id, email.id)}
										aria-label={`Keep ${email.subject || 'email'}`}
									/>
								</Table.Cell>
								<Table.Cell class="whitespace-nowrap"
									>{formatDate(email.sentAt)}</Table.Cell
								>
								<Table.Cell>
									<span class="block max-w-80 truncate"
										>{email.subject || '(no subject)'}</span
									>
								</Table.Cell>
								<Table.Cell>
									<span class="block max-w-48 truncate">
										{email.senderName || email.senderEmail}
									</span>
								</Table.Cell>
								<Table.Cell>
									<span class="block max-w-52 truncate">{email.userEmail}</span>
								</Table.Cell>
								<Table.Cell>
									{#if email.sourcePath}
										<span
											class="bg-muted block max-w-56 truncate rounded p-1.5 text-xs"
										>
											{email.sourcePath}
										</span>
									{/if}
								</Table.Cell>
								<Table.Cell class="text-right">
									<a href={`/mailbox/${email.id}`}>
										<Button variant="outline">View</Button>
									</a>
								</Table.Cell>
							</Table.Row>
						{/each}
					</Table.Body>
				</Table.Root>
			</section>
		{/each}
	</div>
	{:else}
		<div class="rounded-md border p-8 text-center text-sm">
			No fuzzy candidates yet. Queue a scan to build the next bounded batch.
		</div>
	{/if}

	<TablePagination
		count={fuzzyDuplicateGroups.totalGroups}
		perPage={fuzzyDuplicateGroups.limit}
		page={fuzzyDuplicateGroups.page}
		buildHref={buildFuzzyPageUrl}
		prevLabel="Prev"
		nextLabel="Next"
	/>
{/if}
