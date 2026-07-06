<script lang="ts">
	import type { PageData } from './$types';
	import * as Table from '$lib/components/ui/table';
	import { Button } from '$lib/components/ui/button';
	import { Badge } from '$lib/components/ui/badge';
	import TablePagination from '$lib/components/custom/TablePagination.svelte';
	import { api } from '$lib/api.client';
	import { goto, afterNavigate, beforeNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { saveListScroll, getListScroll, lastOpenedEmailId } from '$lib/stores/list-view-state';
	import Check from '@lucide/svelte/icons/check';
	import CheckCheck from '@lucide/svelte/icons/check-check';
	import type {
		ApproveExactDuplicateGroupDto,
		ApproveExactDuplicatesResult,
		ExactDuplicateGroup,
		ExactDuplicateReason,
		IgnoreExactDuplicateGroupsResult,
	} from '@pea/types';

	let { data }: { data: PageData } = $props();

	let duplicateGroups = $derived(data.duplicateGroups);
	let reasonCounts = $derived(data.duplicateGroups.reasonCounts);
	let activeReason = $derived(data.activeReason || '');
	let keeperOverrides = $state<Record<string, string>>({});
	let approvingKey = $state<string | null>(null);

	const reasonFilters: { value: string; label: string }[] = [
		{ value: '', label: 'All' },
		{ value: 'storage_hash', label: 'Raw hash' },
		{ value: 'message_id', label: 'Message-ID' },
		{ value: 'sender_recipients_sent', label: 'Sender + recipients + time' },
		{ value: 'message_body', label: 'Message body' },
		{ value: 'attachment_hash_set', label: 'Attachment set' },
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
			case 'message_body':
				return 'Message body';
		}
	}

	/** Count of groups for a filter pill (uses the `all` bucket for the empty value). */
	function reasonCount(value: string): number {
		return reasonCounts[(value || 'all') as keyof typeof reasonCounts] ?? 0;
	}

	function shortFingerprint(fingerprint: string | null | undefined): string {
		if (!fingerprint) return '';
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

	/** Re-runs the loader after groups are removed so the page refills from
	 *  the remaining ones, clamped to the last page that still exists. */
	async function reloadGroups(
		result: { totalGroups: number; page: number; limit: number },
		removedGroups: number
	) {
		const remaining = Math.max(0, result.totalGroups - removedGroups);
		const totalPages = Math.max(1, Math.ceil(remaining / Math.max(1, result.limit)));
		const target = Math.min(result.page, totalPages);
		await goto(buildExactPageUrl(target), {
			invalidateAll: true,
			keepFocus: true,
			replaceState: true,
		});
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
			await reloadGroups(duplicateGroups, result.approvedGroups);
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

	async function ignoreGroup(groupKey: string) {
		approvingKey = `ignore-${groupKey}`;
		try {
			const response = await api('/archived-emails/duplicates/exact/ignore', {
				method: 'POST',
				body: JSON.stringify({ groupKeys: [groupKey] }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to ignore group');
			}

			const result = body as IgnoreExactDuplicateGroupsResult;
			await reloadGroups(duplicateGroups, result.ignoredGroups);
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Ignore failed',
				message: error instanceof Error ? error.message : 'Failed to ignore group',
				duration: 5000,
				show: true,
			});
		} finally {
			approvingKey = null;
		}
	}

	function buildExactPageUrl(page: number): string {
		const params = new URLSearchParams();
		if (activeReason) params.set('reason', activeReason);
		if (page > 1) params.set('exactPage', String(page));
		const query = params.toString();
		return `/dashboard/duplicates${query ? `?${query}` : ''}`;
	}

	/** URL that applies a reason filter (resets pagination to page 1). */
	function reasonUrl(reason: string): string {
		const query = reason ? `?reason=${encodeURIComponent(reason)}` : '';
		return `/dashboard/duplicates${query}`;
	}

	/** Open an email, remembering this exact duplicates view (page + filter) so
	 *  Back / swipe / after-delete return here rather than to the mailbox. */
	function viewUrl(id: string): string {
		const from = page.url.pathname + page.url.search;
		return `/mailbox/${id}?from=${encodeURIComponent(from)}`;
	}

	// Restore scroll position and highlight the last-opened row on return.
	beforeNavigate(({ to }) => {
		saveListScroll(page.url.pathname + page.url.search, window.scrollY);
		const openedId = to?.url.pathname.match(/^\/mailbox\/([^/]+)$/)?.[1];
		if (openedId) lastOpenedEmailId.set(openedId);
	});
	afterNavigate(() => {
		const y = getListScroll(page.url.pathname + page.url.search);
		if (y !== undefined) requestAnimationFrame(() => window.scrollTo(0, y));
	});
</script>

<svelte:head>
	<title>Duplicate Review - PEA</title>
</svelte:head>

<div class="mb-4">
	<h1 class="text-2xl font-bold">Duplicates</h1>
	<p class="text-muted-foreground text-sm">
		{duplicateGroups.totalGroups}
		{duplicateGroups.totalGroups === 1 ? 'group' : 'groups'}{activeReason
			? ` matching ${reasonLabel(activeReason as ExactDuplicateReason)}`
			: ''}
	</p>
</div>

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
				{rf.label} ({reasonCount(rf.value)})
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
						<div class="text-muted-foreground mt-1 max-w-full truncate font-mono text-xs">
							{shortFingerprint(group.fingerprint)}
						</div>
					</div>
					<div class="flex flex-wrap gap-2">
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
						<Button
							type="button"
							variant="outline"
							disabled={approvingKey !== null}
							onclick={() => ignoreGroup(group.groupKey)}
						>
							{approvingKey === `ignore-${group.groupKey}` ? 'Ignoring...' : 'Ignore'}
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
							<Table.Head>Import Source</Table.Head>
							<Table.Head>Folder</Table.Head>
							<Table.Head class="text-right">Open</Table.Head>
						</Table.Row>
					</Table.Header>
					<Table.Body class="text-sm">
						{#each group.emails as email (email.id)}
							<Table.Row class={email.id === $lastOpenedEmailId ? 'bg-primary/10' : ''}>
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
								<Table.Cell class="whitespace-nowrap">{formatDate(email.sentAt)}</Table.Cell>
								<Table.Cell>
									<span class="block max-w-80 truncate">{email.subject || '(no subject)'}</span>
								</Table.Cell>
								<Table.Cell>
									<span class="block max-w-48 truncate">
										{email.senderName || email.senderEmail}
									</span>
								</Table.Cell>
								<Table.Cell>
									<span class="block max-w-52 truncate">{email.importSource}</span>
								</Table.Cell>
								<Table.Cell>
									{#if email.sourcePath}
										<span class="bg-muted block max-w-56 truncate rounded p-1.5 text-xs">
											{email.sourcePath}
										</span>
									{/if}
								</Table.Cell>
								<Table.Cell class="text-right">
									<a href={viewUrl(email.id)}>
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
		No duplicates{activeReason ? ' for this filter' : ''} found.
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
