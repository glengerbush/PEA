<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import type { ArchivedEmail } from '@pea/types';
	import { ScrollArea } from '$lib/components/ui/scroll-area/index.js';
	import { t } from '$lib/translations';
	import { describeDate } from '$lib/stores/datetime.svelte';
	import Paperclip from '@lucide/svelte/icons/paperclip';

	let {
		thread,
		currentEmailId,
	}: {
		thread: ArchivedEmail['thread'];
		currentEmailId: string;
	} = $props();

	// Keep the ?from= origin (e.g. the Duplicates page) when hopping between
	// thread emails, so the Back button still returns to where the user started.
	const fromQuery = $derived.by(() => {
		const from = page.url.searchParams.get('from');
		return from ? `?from=${encodeURIComponent(from)}` : '';
	});
</script>

<div>
	<ScrollArea class="max-h-120 -ml-3 overflow-y-scroll">
		<div class="relative ml-3 border-l-2 border-gray-200 pl-6">
			{#if thread}
				{#each thread as item, i (item.id)}
					{@const d = describeDate(item.sentAt, item.sentAtKind)}
					<div class="mb-8">
						<span
							class=" ring-sidebar absolute -left-3 flex h-6 w-6 items-center justify-center rounded-full bg-gray-200 ring-8"
						>
							<svg
								class="h-3 w-3 text-gray-600"
								fill="currentColor"
								viewBox="0 0 20 20"
								xmlns="http://www.w3.org/2000/svg"
								><path
									fill-rule="evenodd"
									d="M6 2a1 1 0 00-1 1v1H4a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V6a2 2 0 00-2-2h-1V3a1 1 0 10-2 0v1H7V3a1 1 0 00-1-1zm0 5a1 1 0 000 2h8a1 1 0 100-2H6z"
									clip-rule="evenodd"
								></path></svg
							>
						</span>
						<div class="mb-2 flex items-center gap-2">
							{#if item.hasAttachments}
								<Paperclip
									class="text-muted-foreground h-4 w-4 flex-shrink-0"
									aria-label="Has attachments"
								/>
							{/if}
							<h4
								class:font-bold={item.id === currentEmailId}
								class="text-md {item.id !== currentEmailId
									? 'text-blue-500 hover:underline'
									: 'text-gray-900'}"
							>
								{#if item.id !== currentEmailId}
									<a
										href="/mailbox/{item.id}{fromQuery}"
										onclick={(e) => {
											e.preventDefault();
											goto(`/mailbox/${item.id}${fromQuery}`, {
												invalidateAll: true,
											});
										}}>{item.subject || $t('app.archive.no_subject')}</a
									>
								{:else}
									{item.subject || $t('app.archive.no_subject')}
								{/if}
							</h4>
						</div>
						<div
							class="flex flex-col space-y-2 text-sm font-normal leading-none text-gray-400"
						>
							<span>{$t('app.archive.from')}: {item.senderEmail || '(no sender)'}</span>
							<time>{d.label === 'Received' ? 'Received ' : ''}{d.text}{d.qualifier
									? ` (${d.qualifier})`
									: ''}</time>
						</div>
					</div>
				{/each}
			{/if}
		</div>
	</ScrollArea>
</div>
