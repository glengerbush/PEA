<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import { Download, FileText, ChevronRight } from 'lucide-svelte';
	import { formatBytes } from '$lib/utils';

	let {
		title,
		sizeBytes = null,
		mimeType = null,
		canPreview = true,
		fetchBlob
	}: {
		title: string;
		sizeBytes?: number | null;
		mimeType?: string | null;
		/** Set false to never attempt an inline preview (e.g. server can't serve it). */
		canPreview?: boolean;
		/** Lazily fetches the file bytes; only called when expanded or downloaded. */
		fetchBlob: () => Promise<Blob>;
	} = $props();

	let objectUrl = $state<string | null>(null);
	let textContent = $state<string | null>(null);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let loaded = $state(false);

	const kind = $derived.by(() => {
		const type = (mimeType || '').toLowerCase();
		if (type.startsWith('image/')) return 'image';
		if (type === 'application/pdf') return 'pdf';
		if (type.startsWith('text/') || type === 'application/json') return 'text';
		return 'other';
	});
	const previewable = $derived(canPreview && kind !== 'other');

	async function ensureLoaded(): Promise<string | null> {
		if (loaded) return objectUrl;
		if (loading) return null;
		loading = true;
		error = null;
		try {
			const blob = await fetchBlob();
			objectUrl = URL.createObjectURL(blob);
			if (kind === 'text') textContent = await blob.text();
			loaded = true;
			return objectUrl;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load file';
			return null;
		} finally {
			loading = false;
		}
	}

	function onToggle(event: Event) {
		const el = event.currentTarget as HTMLDetailsElement;
		if (el.open && previewable) void ensureLoaded();
	}

	async function downloadFile() {
		const url = await ensureLoaded();
		if (!url) return;
		const a = document.createElement('a');
		a.href = url;
		a.download = title;
		document.body.appendChild(a);
		a.click();
		a.remove();
	}

	$effect(() => {
		return () => {
			if (objectUrl) URL.revokeObjectURL(objectUrl);
		};
	});
</script>

<details class="group rounded-md border" ontoggle={onToggle}>
	<summary
		class="hover:bg-muted/50 flex cursor-pointer list-none items-center justify-between gap-2 p-2 text-sm"
	>
		<span class="flex min-w-0 items-center gap-2">
			<ChevronRight
				class="text-muted-foreground h-4 w-4 flex-shrink-0 transition-transform group-open:rotate-90"
			/>
			<FileText class="text-muted-foreground h-4 w-4 flex-shrink-0" />
			<span class="truncate" {title}>{title}</span>
			{#if sizeBytes != null}
				<span class="text-muted-foreground flex-shrink-0 text-xs">{formatBytes(sizeBytes)}</span>
			{/if}
		</span>
		<button
			type="button"
			class="text-muted-foreground hover:text-foreground flex-shrink-0 rounded p-1"
			aria-label={`Download ${title}`}
			onclick={(e) => {
				e.preventDefault();
				e.stopPropagation();
				void downloadFile();
			}}
		>
			<Download class="h-4 w-4" />
		</button>
	</summary>
	<div class="border-t p-2">
		{#if !previewable}
			<div class="text-muted-foreground flex items-center justify-between gap-2 text-xs">
				<span>Preview not available for this file type.</span>
				<Button variant="outline" size="sm" class="gap-1 text-xs" onclick={downloadFile}>
					<Download class="h-3.5 w-3.5" /> Download
				</Button>
			</div>
		{:else if loading}
			<p class="text-muted-foreground text-xs">Loading preview…</p>
		{:else if error}
			<p class="text-destructive text-xs">{error}</p>
		{:else if kind === 'image' && objectUrl}
			<img src={objectUrl} alt={title} class="max-h-96 w-auto rounded" />
		{:else if kind === 'pdf' && objectUrl}
			<iframe src={objectUrl} {title} class="h-96 w-full rounded border"></iframe>
		{:else if kind === 'text' && textContent != null}
			<pre
				class="max-h-96 overflow-auto whitespace-pre-wrap break-words text-xs">{textContent}</pre>
		{/if}
	</div>
</details>
