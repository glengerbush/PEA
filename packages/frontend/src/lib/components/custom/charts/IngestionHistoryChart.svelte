<script lang="ts">
	let { data }: { data: { date: Date; count: number }[] } = $props();

	const W = 600;
	const H = 240;
	const PAD = { l: 8, r: 8, t: 10, b: 22 };

	const points = $derived.by(() => {
		if (data.length === 0) return [];
		const max = Math.max(1, ...data.map((d) => d.count));
		const iw = W - PAD.l - PAD.r;
		const ih = H - PAD.t - PAD.b;
		const n = data.length;
		return data.map((d, i) => ({
			x: PAD.l + (n === 1 ? iw / 2 : (i / (n - 1)) * iw),
			y: PAD.t + ih - (d.count / max) * ih,
			d,
		}));
	});

	const linePath = $derived(
		points.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ')
	);
	const areaPath = $derived(
		points.length
			? `${linePath} L ${points[points.length - 1].x} ${H - PAD.b} L ${points[0].x} ${H - PAD.b} Z`
			: ''
	);

	function short(dt: Date | string): string {
		return new Date(dt).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
	}
	function full(dt: Date | string): string {
		return new Date(dt).toLocaleString(undefined, {
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit',
		});
	}

	// A handful of evenly-spaced x-axis labels (plus the last point).
	const ticks = $derived.by(() => {
		const n = points.length;
		if (!n) return [];
		const step = Math.max(1, Math.ceil(n / 6));
		return points.filter((_, i) => i % step === 0 || i === n - 1);
	});
</script>

{#if data.length === 0}
	<div class="text-muted-foreground flex min-h-[300px] items-center justify-center text-sm">
		No ingestion history yet
	</div>
{:else}
	<svg viewBox={`0 0 ${W} ${H}`} class="h-auto w-full" role="img" aria-label="Ingestion history">
		<path d={areaPath} fill="var(--color-chart-1)" opacity="0.15" />
		<path
			d={linePath}
			fill="none"
			stroke="var(--color-chart-1)"
			stroke-width="2"
			vector-effect="non-scaling-stroke"
		/>
		{#each points as p (p.d.date)}
			<circle cx={p.x} cy={p.y} r="2.5" fill="var(--color-chart-1)">
				<title>{full(p.d.date)}: {p.d.count}</title>
			</circle>
		{/each}
		{#each ticks as p (p.d.date)}
			<text x={p.x} y={H - 6} text-anchor="middle" class="fill-muted-foreground text-[10px]">
				{short(p.d.date)}
			</text>
		{/each}
	</svg>
{/if}
