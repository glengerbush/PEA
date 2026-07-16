<script lang="ts">
	let { progress }: { progress: number } = $props();

	const p = $derived(Math.min(1, Math.max(0, progress)));
	const centerY = 500;
	const tipX = $derived(10 + p * 470);
	const shoulderX = $derived(4 + p * 155);
	const spread = $derived(68 + p * 320);
	const iconX = $derived(Math.max(32, tipX - 34));
	const iconRadius = $derived(19 + p * 13);
	const edgePath = $derived(
		`M 0 0 L 0 1000 ` +
			`C 0 ${centerY + spread * 1.35}, ${shoulderX * 0.25} ${centerY + spread}, ${shoulderX} ${centerY + spread * 0.58} ` +
			`C ${shoulderX * 1.4} ${centerY + spread * 0.25}, ${tipX * 0.72} ${centerY + 38}, ${tipX} ${centerY} ` +
			`C ${tipX * 0.72} ${centerY - 38}, ${shoulderX * 1.4} ${centerY - spread * 0.25}, ${shoulderX} ${centerY - spread * 0.58} ` +
			`C ${shoulderX * 0.25} ${centerY - spread}, 0 ${centerY - spread * 1.35}, 0 0 Z`
	);
</script>

{#if p > 0.001}
	<!-- Fade the viewport beneath the sticky navigation without compositing the
	     page itself. Keeping this layer pointer-transparent also prevents it from
	     changing hit testing while a gesture is cancelled or completed. -->
	<div
		class="bg-background pointer-events-none fixed inset-x-0 top-16 bottom-0 z-30"
		style:opacity={p}
		aria-hidden="true"
	></div>
	<div
		class="pointer-events-none fixed inset-y-0 left-0 z-50 w-[42vw] max-w-[520px]"
		style:opacity={Math.min(1, 0.18 + p * 1.15)}
		aria-hidden="true"
	>
		<svg
			class="h-full w-full overflow-visible"
			viewBox="0 0 520 1000"
			preserveAspectRatio="none"
		>
			<defs>
				<linearGradient id="swipe-edge-gradient" x1="0" y1="0" x2="1" y2="0">
					<stop offset="0" stop-color="var(--primary)" stop-opacity={0.28 + p * 0.18} />
					<stop offset="0.72" stop-color="var(--primary)" stop-opacity={0.68 + p * 0.2} />
					<stop offset="1" stop-color="var(--primary)" stop-opacity="0.96" />
				</linearGradient>
			</defs>
			<path
				d={edgePath}
				fill="url(#swipe-edge-gradient)"
				stroke="var(--primary)"
				stroke-opacity="0.22"
				stroke-width={3 + p * 4}
			/>
			<circle
				cx={iconX}
				cy={centerY}
				r={iconRadius}
				fill="var(--primary-foreground)"
				fill-opacity={0.82 + p * 0.18}
			/>
			<path
				d={`M ${iconX + 8} ${centerY - 9} L ${iconX - 2} ${centerY} L ${iconX + 8} ${centerY + 9} M ${iconX - 1} ${centerY} H ${iconX + 11}`}
				fill="none"
				stroke="var(--primary)"
				stroke-width="3.5"
				stroke-linecap="round"
				stroke-linejoin="round"
			/>
		</svg>
	</div>
{/if}
