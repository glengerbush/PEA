<script lang="ts">
	import type { PageData } from './$types';
	import * as Card from '$lib/components/ui/card';
	import { t } from '$lib/translations';
	import * as Table from '$lib/components/ui/table';
	import { Button } from '$lib/components/ui/button';
	import { goto } from '$app/navigation';
	import type { JobStatus } from '@open-archiver/types';
	import TablePagination from '$lib/components/custom/TablePagination.svelte';
	import { onMount } from 'svelte';
	import { browser } from '$app/environment';

	let { data }: { data: PageData } = $props();
	let queue = $derived(data.queue);

	// Friendly labels for the internal queue names (queue names themselves are unchanged).
	const QUEUE_LABELS: Record<string, string> = {
		ingestion: 'Imports',
		indexing: 'Indexing',
		'remote-content': 'Remote content',
	};
	const queueLabel = (name: string) => QUEUE_LABELS[name] ?? name.split('_').join(' ');

	const jobStatuses: JobStatus[] = [
		'failed',
		'active',
		'completed',
		'delayed',
		'waiting',
		'paused',
	];

	let selectedStatus: JobStatus | undefined = $state('failed');

	onMount(() => {
		if (browser) {
			const url = new URL(window.location.href);
			const status = url.searchParams.get('status') as JobStatus;
			if (status) {
				selectedStatus = status;
			}
		}
	});

	function handleStatusChange(status: JobStatus) {
		selectedStatus = status;
		const url = new URL(window.location.href);
		url.searchParams.set('status', status);
		url.searchParams.set('page', '1');
		goto(url.toString(), { invalidateAll: true });
	}
</script>

<svelte:head>
	<title>{queueLabel(queue.name)} - {$t('app.jobs.title')} - Open Archiver</title>
</svelte:head>

<div class="space-y-4">
	<a href="/dashboard/admin/jobs" class="text-primary mb-1 text-sm hover:underline">
		&larr; {$t('app.jobs.back_to_queues')}
	</a>
	<h1 class="text-2xl font-bold capitalize">{queueLabel(queue.name)}</h1>

	<Card.Root>
		<Card.Header>
			<Card.Title>{$t('app.jobs.jobs')}</Card.Title>
			<div class="flex flex-wrap space-x-2 space-y-2">
				{#each jobStatuses as status}
					<Button
						variant={selectedStatus === status ? 'default' : 'outline'}
						onclick={() => handleStatusChange(status)}
						class="capitalize"
					>
						{status} ({queue.counts[status]})
					</Button>
				{/each}
			</div>
		</Card.Header>
		<Card.Content>
			<Table.Root>
				<Table.Header>
					<Table.Row>
						<Table.Head>{$t('app.jobs.id')}</Table.Head>
						<Table.Head>{$t('app.jobs.name')}</Table.Head>
						<Table.Head>{$t('app.jobs.state')}</Table.Head>
						<Table.Head>{$t('app.jobs.created_at')}</Table.Head>
						<Table.Head>{$t('app.jobs.processed_at')}</Table.Head>
						<Table.Head>{$t('app.jobs.finished_at')}</Table.Head>
						<Table.Head>{$t('app.jobs.import_source')}</Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each queue.jobs as job}
						<Table.Row>
							<Table.Cell>{job.id}</Table.Cell>
							<Table.Cell>{job.name}</Table.Cell>
							<Table.Cell class="capitalize">
								{#if job.error}
									<Button
										variant="secondary"
										size="sm"
										class="cursor-pointer capitalize"
										onclick={() => {
											if (job.error) {
												const el = document.getElementById(
													`error-${job.id}`
												);
												if (el) {
													el.classList.toggle('hidden');
												}
											}
										}}
									>
										{job.state}
									</Button>
								{:else}
									{job.state}
								{/if}
							</Table.Cell>
							<Table.Cell>{new Date(job.timestamp).toLocaleString()}</Table.Cell>
							<Table.Cell
								>{job.processedOn
									? new Date(job.processedOn).toLocaleString()
									: 'N/A'}</Table.Cell
							>
							<Table.Cell
								>{job.finishedOn
									? new Date(job.finishedOn).toLocaleString()
									: 'N/A'}</Table.Cell
							>
							<Table.Cell>
								<a
									href="/mailbox?ingestionSourceId={job.ingestionSourceId}"
									>{job.ingestionSourceId || 'N/A'}</a
								>
							</Table.Cell>
						</Table.Row>
						{#if job.error}
							<Table.Row id={`error-${job.id}`} class="hidden">
								<Table.Cell colspan={7} class="p-0">
									<pre
										class="bg-muted max-w-full text-wrap rounded-md p-4 text-xs">{job.error}</pre>
								</Table.Cell>
							</Table.Row>
						{/if}
					{/each}
				</Table.Body>
			</Table.Root>
		</Card.Content>
		<Card.Footer class="flex flex-col items-center justify-between gap-4 sm:flex-row">
			<div class="text-muted-foreground text-nowrap text-sm">
				{$t('app.jobs.showing')}
				{queue.jobs.length}
				{$t('app.jobs.of')}
				{queue.pagination.totalJobs}
				{$t('app.jobs.jobs')}
			</div>
			<TablePagination
				count={queue.pagination.totalJobs}
				perPage={queue.pagination.limit}
				page={queue.pagination.currentPage}
				buildHref={(p) =>
					`/dashboard/admin/jobs/${queue.name}?status=${selectedStatus}&page=${p}`}
				prevLabel={$t('app.jobs.previous')}
				nextLabel={$t('app.jobs.next')}
				class=""
			/>
		</Card.Footer>
	</Card.Root>
</div>
