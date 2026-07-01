<script lang="ts">
	import * as Pagination from '$lib/components/ui/pagination/index.js';
	import ChevronLeft from 'lucide-svelte/icons/chevron-left';
	import ChevronRight from 'lucide-svelte/icons/chevron-right';

	let {
		count,
		perPage,
		page,
		buildHref,
		prevLabel = 'Previous',
		nextLabel = 'Next',
		class: className = 'mt-8'
	}: {
		/** Total item count. */
		count: number;
		/** Items per page. */
		perPage: number;
		/**
		 * The current page — must come from the URL / server load, NOT from the
		 * Pagination component's internal counter. The prev/next links and the
		 * active highlight are derived from this so navigation can't double-step.
		 */
		page: number;
		/** Builds the URL for a given page number (navigation is anchor-driven). */
		buildHref: (page: number) => string;
		prevLabel?: string;
		nextLabel?: string;
		class?: string;
	} = $props();

	const totalPages = $derived(Math.max(1, Math.ceil(count / perPage)));
	const atFirst = $derived(page <= 1);
	const atLast = $derived(page >= totalPages);
</script>

{#if count > perPage}
	<div class={className}>
		<Pagination.Root {count} {perPage} {page}>
			{#snippet children({ pages })}
				<Pagination.Content>
					<Pagination.Item>
						<a
							href={buildHref(page - 1)}
							aria-disabled={atFirst}
							tabindex={atFirst ? -1 : undefined}
							class={atFirst ? 'pointer-events-none opacity-50' : ''}
						>
							<Pagination.PrevButton>
								<ChevronLeft class="h-4 w-4" />
								<span class="hidden sm:block">{prevLabel}</span>
							</Pagination.PrevButton>
						</a>
					</Pagination.Item>
					{#each pages as p (p.key)}
						{#if p.type === 'ellipsis'}
							<Pagination.Item>
								<Pagination.Ellipsis />
							</Pagination.Item>
						{:else}
							<Pagination.Item>
								<a href={buildHref(p.value)}>
									<Pagination.Link page={p} isActive={page === p.value}>
										{p.value}
									</Pagination.Link>
								</a>
							</Pagination.Item>
						{/if}
					{/each}
					<Pagination.Item>
						<a
							href={buildHref(page + 1)}
							aria-disabled={atLast}
							tabindex={atLast ? -1 : undefined}
							class={atLast ? 'pointer-events-none opacity-50' : ''}
						>
							<Pagination.NextButton>
								<span class="hidden sm:block">{nextLabel}</span>
								<ChevronRight class="h-4 w-4" />
							</Pagination.NextButton>
						</a>
					</Pagination.Item>
				</Pagination.Content>
			{/snippet}
		</Pagination.Root>
	</div>
{/if}
