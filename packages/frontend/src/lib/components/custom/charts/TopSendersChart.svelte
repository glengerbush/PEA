<script lang="ts">
	import type { TopSender } from '@pea/types';
	import * as Tooltip from '$lib/components/ui/tooltip';

	let {
		data,
		onSelect = () => {},
	}: {
		data: TopSender[];
		onSelect?: (sender: string) => void;
	} = $props();

	const max = $derived(Math.max(1, ...data.map((d) => d.count)));
</script>

{#if data.length === 0}
	<div class="text-muted-foreground flex min-h-[300px] items-center justify-center text-sm">
		No sender data yet
	</div>
{:else}
	<div class="flex w-full flex-col gap-2">
		{#each data as d (d.sender)}
			<Tooltip.Root>
				<Tooltip.Trigger>
					{#snippet child({ props })}
						<button
							{...props}
							type="button"
							class="group flex flex-col gap-1 text-left"
							onclick={() => onSelect(d.sender)}
						>
							<div class="flex items-center justify-between gap-2 text-xs">
								<span class="truncate group-hover:underline">{d.sender}</span>
								<span class="text-muted-foreground flex-shrink-0">{d.count}</span>
							</div>
							<div class="bg-muted h-2.5 w-full overflow-hidden rounded-full">
								<div
									class="h-full rounded-full transition-all group-hover:opacity-80"
									style:width={`${(d.count / max) * 100}%`}
									style:background="var(--color-chart-1)"
								></div>
							</div>
						</button>
					{/snippet}
				</Tooltip.Trigger>
				<Tooltip.Content>{`${d.sender}: ${d.count}`}</Tooltip.Content>
			</Tooltip.Root>
		{/each}
	</div>
{/if}
