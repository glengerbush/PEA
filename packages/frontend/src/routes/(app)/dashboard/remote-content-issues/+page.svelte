<script lang="ts">
	import type { PageData } from './$types';
	import * as Table from '$lib/components/ui/table';
	import * as Select from '$lib/components/ui/select';
	import { Badge } from '$lib/components/ui/badge';
	import { Button, buttonVariants } from '$lib/components/ui/button';
	import * as Tooltip from '$lib/components/ui/tooltip';
	import { goto } from '$app/navigation';
	import { api } from '$lib/api.client';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { t } from '$lib/translations';
	import { formatDateTime } from '$lib/stores/datetime.svelte';
	import { disableTwoFingerSwipe } from '$lib/stores/swipe.store';
	import { get } from 'svelte/store';
	import TablePagination from '$lib/components/custom/TablePagination.svelte';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import RotateCw from '@lucide/svelte/icons/rotate-cw';

	let { data }: { data: PageData } = $props();
	const result = $derived(data.result);
	const status = $derived(data.filters.status);
	const sort = $derived(data.filters.sort);
	const direction = $derived(data.filters.direction);
	const limit = $derived(data.filters.limit);

	let retrying = $state<Record<string, boolean>>({});

	const statusOptions = [
		{ value: 'all', label: 'All issues' },
		{ value: 'failed', label: 'Failed' },
		{ value: 'partial', label: 'Partial' },
	];
	const limitOptions = ['25', '50', '100'];
	const statusLabel = $derived(
		statusOptions.find((o) => o.value === status)?.label ?? 'All issues'
	);

	function buildUrl(overrides: Record<string, string | number> = {}): string {
		const merged = { status, sort, direction, limit, page: result.page, ...overrides };
		const params = new URLSearchParams();
		if (merged.status && merged.status !== 'all') params.set('status', String(merged.status));
		if (merged.sort && merged.sort !== 'date') params.set('sort', String(merged.sort));
		if (merged.direction && merged.direction !== 'desc')
			params.set('direction', String(merged.direction));
		if (merged.limit && String(merged.limit) !== '25')
			params.set('limit', String(merged.limit));
		if (merged.page && Number(merged.page) > 1) params.set('page', String(merged.page));
		const qs = params.toString();
		return `/dashboard/remote-content-issues${qs ? `?${qs}` : ''}`;
	}

	function toggleSort(field: string) {
		const dir = sort === field && direction === 'desc' ? 'asc' : 'desc';
		goto(buildUrl({ sort: field, direction: dir, page: 1 }));
	}
	function changeStatus(value: string) {
		goto(buildUrl({ status: value, page: 1 }));
	}
	function changePageSize(value: string) {
		goto(buildUrl({ limit: value, page: 1 }));
	}

	async function retry(emailId: string) {
		retrying = { ...retrying, [emailId]: true };
		try {
			const res = await api(`/archived-emails/${emailId}/remote-content/archive`, {
				method: 'POST',
			});
			if (!res.ok) {
				const b = await res.json().catch(() => ({}));
				throw new Error(b.message || 'Failed to re-queue remote content');
			}
			setAlert({
				type: 'success',
				title: 'Retry queued',
				message: 'Remote-content archiving was re-queued for this email.',
				duration: 3000,
				show: true,
			});
		} catch (e) {
			setAlert({
				type: 'error',
				title: 'Retry failed',
				message: e instanceof Error ? e.message : 'Unknown error',
				duration: 5000,
				show: true,
			});
		} finally {
			retrying = { ...retrying, [emailId]: false };
		}
	}

	function goBack() {
		goto('/dashboard');
	}

	// --- Two-finger (horizontal) swipe returns to the dashboard ---
	let swipeAccum = 0;
	let swipeResetTimer: ReturnType<typeof setTimeout> | null = null;
	let swipeCooldownUntil = 0;
	/** 0→1 progress toward the swipe threshold, drives the on-screen affordance. */
	let swipeProgress = $state(0);
	function handleWheel(event: WheelEvent) {
		if (get(disableTwoFingerSwipe)) return;
		// Only count clearly-horizontal movement so vertical scrolling never triggers.
		if (Math.abs(event.deltaX) <= Math.abs(event.deltaY) * 1.5) return;
		const now = Date.now();
		if (now < swipeCooldownUntil) return;
		swipeAccum += event.deltaX;
		swipeProgress = Math.min(1, Math.abs(swipeAccum) / 300);
		if (swipeResetTimer) clearTimeout(swipeResetTimer);
		swipeResetTimer = setTimeout(() => {
			swipeAccum = 0;
			swipeProgress = 0;
		}, 400);
		if (Math.abs(swipeAccum) >= 300) {
			swipeAccum = 0;
			swipeProgress = 0;
			swipeCooldownUntil = now + 1000;
			goBack();
		}
	}
</script>

{#snippet sortHeader(field: string, label: string)}
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

<svelte:head>
	<title>Remote content issues - PEA</title>
</svelte:head>

<svelte:window onwheel={handleWheel} />

<!-- Two-finger swipe affordance: a back indicator that slides in as the gesture
     approaches the threshold, then completes into the navigation. -->
{#if swipeProgress > 0}
	<div
		class="pointer-events-none fixed left-2 top-1/2 z-50"
		style="opacity:{swipeProgress}; transform: translate({-44 + swipeProgress * 44}px, -50%);"
		aria-hidden="true"
	>
		<div
			class="bg-primary/90 text-primary-foreground flex h-12 w-12 items-center justify-center rounded-full shadow-lg backdrop-blur"
			style="transform: scale({0.7 + swipeProgress * 0.3});"
		>
			<ArrowLeft class="h-6 w-6" />
		</div>
	</div>
{/if}

<div class="space-y-4">
	<div>
		<Button variant="ghost" size="sm" class="-ml-2 gap-2" onclick={goBack}>
			<ArrowLeft class="h-4 w-4" />
			{$t('app.archive.back_to_dashboard')}
		</Button>
	</div>
	<div class="flex flex-wrap items-center justify-between gap-3">
		<div>
			<h1 class="text-2xl font-bold">Remote content issues</h1>
			<p class="text-muted-foreground text-sm">
				Emails whose remote images/assets failed or were blocked — {result.total} total.
			</p>
		</div>
		<Select.Root type="single" value={status} onValueChange={changeStatus}>
			<Select.Trigger class="w-[10rem]">{statusLabel}</Select.Trigger>
			<Select.Content>
				{#each statusOptions as o (o.value)}
					<Select.Item value={o.value} label={o.label}>{o.label}</Select.Item>
				{/each}
			</Select.Content>
		</Select.Root>
	</div>

	<div class="rounded-md border">
		<Table.Root>
			<Table.Header>
				<Table.Row>
					<Table.Head>{@render sortHeader('subject', 'Subject')}</Table.Head>
					<Table.Head>Sender</Table.Head>
					<Table.Head>{@render sortHeader('status', 'Status')}</Table.Head>
					<Table.Head>Errors</Table.Head>
					<Table.Head>{@render sortHeader('date', 'Archived')}</Table.Head>
					<Table.Head class="text-right">Retry</Table.Head>
				</Table.Row>
			</Table.Header>
			<Table.Body class="text-sm">
				{#if result.items.length > 0}
					{#each result.items as issue (issue.emailId)}
						<Table.Row class="hover:bg-muted/50">
							<Table.Cell class="max-w-xs">
								<a
									href={`/mailbox/${issue.emailId}`}
									class="link block truncate font-medium">{issue.subject}</a
								>
							</Table.Cell>
							<Table.Cell class="text-muted-foreground max-w-[12rem] truncate"
								>{issue.sender}</Table.Cell
							>
							<Table.Cell>
								<Badge
									variant={issue.status === 'failed'
										? 'destructive'
										: 'secondary'}>{issue.status}</Badge
								>
							</Table.Cell>
							<Table.Cell class="max-w-md">
								{#if issue.assets.length > 0}
									<ul class="space-y-0.5">
										{#each issue.assets.slice(0, 3) as asset}
											<li class="text-xs">
												<span class="text-destructive">{asset.status}:</span
												>
												<span class="text-foreground/80"
													>{asset.reason || 'No reason recorded'}</span
												>
											</li>
										{/each}
										{#if issue.assets.length > 3}
											<li class="text-muted-foreground text-xs">
												+{issue.assets.length - 3} more
											</li>
										{/if}
									</ul>
								{:else}
									<span class="text-muted-foreground text-xs"
										>Failed before any assets were fetched</span
									>
								{/if}
							</Table.Cell>
							<Table.Cell class="text-muted-foreground whitespace-nowrap">
								{formatDateTime(issue.archivedAt)}
							</Table.Cell>
							<Table.Cell class="text-right">
								<Tooltip.Root>
									<Tooltip.Trigger
										type="button"
										class={buttonVariants({ variant: 'ghost', size: 'icon' })}
										disabled={retrying[issue.emailId]}
										onclick={() => retry(issue.emailId)}
										aria-label="Retry"
									>
										<RotateCw
											class="h-4 w-4 {retrying[issue.emailId]
												? 'animate-spin'
												: ''}"
										/>
									</Tooltip.Trigger>
									<Tooltip.Content>Retry</Tooltip.Content>
								</Tooltip.Root>
							</Table.Cell>
						</Table.Row>
					{/each}
				{:else}
					<Table.Row>
						<Table.Cell colspan={6} class="h-24 text-center">
							No remote-content issues to show.
						</Table.Cell>
					</Table.Row>
				{/if}
			</Table.Body>
		</Table.Root>
	</div>

	<div class="flex flex-wrap items-center justify-between gap-3">
		<label class="text-muted-foreground flex items-center gap-2 text-sm">
			<span>Rows per page</span>
			<Select.Root type="single" value={limit} onValueChange={changePageSize}>
				<Select.Trigger class="h-8 w-[4.5rem]">{limit}</Select.Trigger>
				<Select.Content>
					{#each limitOptions as opt (opt)}
						<Select.Item value={opt} label={opt}>{opt}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</label>
		<TablePagination
			count={result.total}
			perPage={result.limit}
			page={result.page}
			buildHref={(p: number) => buildUrl({ page: p })}
			prevLabel="Previous"
			nextLabel="Next"
		/>
	</div>
</div>
