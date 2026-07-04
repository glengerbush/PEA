<script lang="ts">
	import type { IngestionSourceStats } from '@pea/types';
	import { formatBytes } from '$lib/utils';

	let {
		data,
		onSelect = () => {}
	}: {
		data: IngestionSourceStats[];
		onSelect?: (source: IngestionSourceStats) => void;
	} = $props();

	const COLORS = [
		'var(--color-chart-1)',
		'var(--color-chart-2)',
		'var(--color-chart-3)',
		'var(--color-chart-4)',
		'var(--color-chart-5)'
	];

	const R = 80;
	const r = 48;
	const C = 100;

	const slices = $derived.by(() => {
		const items = data.filter((d) => (d.storageUsed ?? 0) > 0);
		const total = items.reduce((s, d) => s + (d.storageUsed ?? 0), 0) || 1;
		let angle = -Math.PI / 2;
		return items.map((d, i) => {
			const start = angle;
			angle += ((d.storageUsed ?? 0) / total) * Math.PI * 2;
			return { d, start, end: angle, color: COLORS[i % COLORS.length] };
		});
	});

	function arcPath(start: number, endRaw: number): string {
		// A single full-circle arc can't be expressed with one A command.
		const end = endRaw - start >= Math.PI * 2 - 1e-6 ? start + Math.PI * 2 - 1e-4 : endRaw;
		const pt = (radius: number, a: number) =>
			`${C + radius * Math.cos(a)} ${C + radius * Math.sin(a)}`;
		const large = end - start > Math.PI ? 1 : 0;
		return `M ${pt(R, start)} A ${R} ${R} 0 ${large} 1 ${pt(R, end)} L ${pt(r, end)} A ${r} ${r} 0 ${large} 0 ${pt(r, start)} Z`;
	}
</script>

{#if slices.length === 0}
	<div class="text-muted-foreground flex h-full items-center justify-center text-sm">
		No storage data yet
	</div>
{:else}
	<div class="flex h-full w-full flex-col gap-3">
		<svg viewBox="0 0 200 200" class="mx-auto h-40 w-40" role="img" aria-label="Storage by source">
			{#each slices as s (s.d.id)}
				<path
					d={arcPath(s.start, s.end)}
					fill={s.color}
					class="cursor-pointer outline-none transition-opacity hover:opacity-80"
					role="button"
					tabindex="0"
					aria-label={`${s.d.name}: ${formatBytes(s.d.storageUsed ?? 0)}`}
					onclick={() => onSelect(s.d)}
					onkeydown={(e) => e.key === 'Enter' && onSelect(s.d)}
				>
					<title>{s.d.name}: {formatBytes(s.d.storageUsed ?? 0)}</title>
				</path>
			{/each}
		</svg>
		<ul class="space-y-1 overflow-y-auto text-xs">
			{#each slices as s (s.d.id)}
				<li>
					<button
						type="button"
						class="flex w-full items-center gap-2 hover:underline"
						onclick={() => onSelect(s.d)}
					>
						<span class="h-2.5 w-2.5 flex-shrink-0 rounded-sm" style:background={s.color}></span>
						<span class="truncate">{s.d.name}</span>
						<span class="text-muted-foreground ml-auto flex-shrink-0"
							>{formatBytes(s.d.storageUsed ?? 0)}</span
						>
					</button>
				</li>
			{/each}
		</ul>
	</div>
{/if}
