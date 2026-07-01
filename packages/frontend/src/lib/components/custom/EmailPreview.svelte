<script lang="ts">
	import { api } from '$lib/api.client';
	import { t } from '$lib/translations';
	import type { RemoteContentPreview } from '@open-archiver/types';

	let { emailId, refreshKey = 0 }: { emailId: string; refreshKey?: number } = $props();

	async function loadPreview(id: string, _refreshKey: number): Promise<RemoteContentPreview> {
		const response = await api(`/archived-emails/${id}/preview`);
		const body = await response.json();
		if (!response.ok) {
			throw new Error(body.message || 'Failed to load email preview');
		}
		return body as RemoteContentPreview;
	}

	let previewPromise = $derived(loadPreview(emailId, refreshKey));
</script>

<div class="mt-2 rounded-md border bg-white p-4">
	{#await previewPromise}
		<p>{$t('app.components.email_preview.loading')}</p>
	{:then preview}
		{#if preview.html}
			<iframe
				title={$t('app.archive.email_preview')}
				srcdoc={preview.html}
				sandbox="allow-popups allow-popups-to-escape-sandbox"
				referrerpolicy="no-referrer"
				class="h-[600px] w-full border-none"
			></iframe>
		{:else}
			<p class="text-gray-500">{$t('app.components.email_preview.not_available')}</p>
		{/if}
	{:catch error}
		<p>{error instanceof Error ? error.message : 'Failed to load email preview'}</p>
	{/await}
</div>
