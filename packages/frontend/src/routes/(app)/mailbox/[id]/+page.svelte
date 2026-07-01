<script lang="ts">
	import type { PageData } from './$types';
	import { Button } from '$lib/components/ui/button';
	import * as Card from '$lib/components/ui/card';
	import EmailPreview from '$lib/components/custom/EmailPreview.svelte';
	import TagCombobox from '$lib/components/custom/TagCombobox.svelte';
	import { contactName } from '$lib/stores/contacts.svelte';
	import AttachmentPreview from '$lib/components/custom/AttachmentPreview.svelte';
	import EmailThread from '$lib/components/custom/EmailThread.svelte';
	import { formatDateTime, formatDate } from '$lib/stores/datetime.svelte';
	import { api } from '$lib/api.client';
	import { browser } from '$app/environment';
	import { formatBytes } from '$lib/utils';
	import { goto } from '$app/navigation';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as Select from '$lib/components/ui/select/index.js';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { t } from '$lib/translations';
	import { Badge } from '$lib/components/ui/badge';
	import * as HoverCard from '$lib/components/ui/hover-card';
	import {
		Clock,
		Trash2,
		CalendarClock,
		AlertCircle,
		Shield,
		CircleAlert,
		Archive,
		ArrowLeft,
		X,
	} from 'lucide-svelte';
	import { get } from 'svelte/store';
	import { lastMailboxListUrl } from '$lib/stores/mailbox-nav';
	import { page } from '$app/state';
	import { enhance } from '$app/forms';
	import type {
		RemoteContentAssetSummary,
		RemoteContentPreview,
		RemoteContentStatus,
		UpdateArchivedEmailTagsResult,
	} from '@open-archiver/types';
	import PostalMime, { type Attachment as PostalAttachment } from 'postal-mime';
	import { Paperclip } from 'lucide-svelte';

	let { data }: { data: PageData } = $props();
	let email = $derived(data.email);

	/** Return to the previous mailbox list view (preserving its search/filter/page
	 *  query), falling back to the mailbox root if it was opened directly. */
	function goBack() {
		goto(get(lastMailboxListUrl) ?? '/mailbox');
	}

	/** "Name <email>" when a name (contact or header) is known, else the bare address. */
	function identityLabel(addr: string | null | undefined, fallback?: string | null): string {
		const email = (addr || '').trim();
		const name = (contactName(email) || fallback || '').trim();
		return name && name.toLowerCase() !== email.toLowerCase() ? `${name} <${email}>` : email;
	}

	// --- Tag editing (add/remove on this single email) ---
	let localTags = $state<string[]>([]);
	$effect(() => {
		localTags = Array.isArray(email?.tags) ? [...email.tags] : [];
	});
	let isUpdatingTags = $state(false);
	// Existing tags across the archive, minus the ones already on this email.
	let tagSuggestions = $derived(
		(data.allTags ?? []).filter((t) => !localTags.includes(t))
	);

	async function applyTagChange(addTags: string[], removeTags: string[]) {
		if (!email) return;
		const emailId = email.id;
		isUpdatingTags = true;
		try {
			const response = await api('/archived-emails/bulk/tags', {
				method: 'POST',
				body: JSON.stringify({ emailIds: [emailId], addTags, removeTags }),
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to update tags');
			}
			const result = body as UpdateArchivedEmailTagsResult;
			const updated = result.emails.find((e) => e.id === emailId);
			if (updated) localTags = updated.tags;
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Tag update failed',
				message: error instanceof Error ? error.message : 'Failed to update tags',
				duration: 5000,
				show: true,
			});
		} finally {
			isUpdatingTags = false;
		}
	}

	function addTagByName(tag: string) {
		const t = tag.trim();
		if (!t || localTags.includes(t)) return;
		applyTagChange([t], []);
	}

	function removeTag(tag: string) {
		applyTagChange([], [tag]);
	}

	let isDeleteDialogOpen = $state(false);
	let isDeleting = $state(false);
	let isQueueingRemoteContent = $state(false);
	let remoteContentQueued = $state(false);
	let remoteContentResult = $state<RemoteContentPreview | null>(null);
	let remoteContentRefreshKey = $state(0);
	let currentRemoteContentStatus = $derived(
		remoteContentQueued
			? ('pending' as const)
			: remoteContentResult !== null && remoteContentResult.emailId === email?.id
				? remoteContentResult.status
				: email?.remoteContentStatus
	);
	let currentRemoteContentAssetCount = $derived(
		remoteContentResult !== null && remoteContentResult.emailId === email?.id
			? remoteContentResult.archivedAssetCount
			: (email?.remoteContentAssetCount ?? 0)
	);

	// Reset per-email remote-content UI state on navigation between emails.
	// `email` is $derived(data.email); same-route nav keeps these $state values,
	// so without this a queued "Processing…" indicator from a previous email
	// leaks onto the next one (the poll self-cancels via its email.id guard but
	// never clears the flag).
	$effect(() => {
		void email?.id; // re-run when the viewed email changes
		remoteContentQueued = false;
		isQueueingRemoteContent = false;
		remoteContentResult = null;
	});

	// --- Embedded attachment state (parsed from raw EML) ---
	/** Non-inline attachments parsed from the raw EML via postal-mime */
	let embeddedAttachments = $state<PostalAttachment[]>([]);
	let isEmbeddedAttachmentDialogOpen = $state(false);
	let selectedEmbeddedFilename = $state('');

	/** Parse raw EML to extract non-inline attachments for display */
	$effect(() => {
		const currentId = email?.id;
		// Clear immediately so a previous email's attachments never linger while
		// the new email parses (or if the new email has no raw content).
		embeddedAttachments = [];

		async function parseEmlAttachments() {
			const raw = email?.raw;
			if (!raw) return;

			try {
				let buffer: Uint8Array;
				if (raw && typeof raw === 'object' && 'type' in raw && raw.type === 'Buffer') {
					buffer = new Uint8Array(
						(raw as unknown as { type: 'Buffer'; data: number[] }).data
					);
				} else {
					buffer = new Uint8Array(raw as unknown as ArrayLike<number>);
				}

				const parsed = await new PostalMime().parse(buffer);
				// Ignore a stale parse that resolved after the user navigated to a
				// different email — otherwise its attachments render under the wrong one.
				if (email?.id !== currentId) return;
				// Filter to non-inline attachments (those with a filename and no contentId,
				// or with disposition=attachment)
				embeddedAttachments = parsed.attachments.filter(
					(att) => att.filename && (att.disposition === 'attachment' || !att.contentId)
				);
			} catch (error) {
				console.error('Failed to parse EML for embedded attachments:', error);
			}
		}
		parseEmlAttachments();
	});

	// --- Archived remote-content assets (shown collapsed in the right column) ---
	let remoteAssets = $state<RemoteContentAssetSummary[]>([]);
	const archivedRemoteAssets = $derived(remoteAssets.filter((a) => a.status === 'archived'));
	const failedRemoteAssets = $derived(remoteAssets.filter((a) => a.status !== 'archived'));
	$effect(() => {
		const id = email?.id;
		// Re-fetch after remote content is (re)archived.
		void remoteContentRefreshKey;
		if (!id) {
			remoteAssets = [];
			return;
		}
		void loadRemoteAssets(id);
	});

	async function loadRemoteAssets(id: string): Promise<void> {
		try {
			const response = await api(`/archived-emails/${id}/remote-assets`);
			remoteAssets = response.ok ? ((await response.json()) as RemoteContentAssetSummary[]) : [];
		} catch {
			remoteAssets = [];
		}
	}

	/** Fetches a stored attachment's bytes for inline preview / download. */
	function fetchAttachmentBlob(storagePath: string): Promise<Blob> {
		return api(`/storage/download?path=${encodeURIComponent(storagePath)}`).then((response) => {
			if (!response.ok) throw new Error('Failed to load attachment');
			return response.blob();
		});
	}

	/** Fetches an archived remote-content asset's bytes for inline preview / download. */
	function fetchRemoteAssetBlob(assetId: string): Promise<Blob> {
		const id = email?.id;
		if (!id) return Promise.reject(new Error('Email not loaded'));
		return api(`/archived-emails/${id}/remote-assets/${assetId}`).then((response) => {
			if (!response.ok) throw new Error('Failed to load remote content');
			return response.blob();
		});
	}

	/** Derives a readable title from a remote asset's source URL. */
	function remoteAssetTitle(asset: RemoteContentAssetSummary): string {
		try {
			const url = new URL(asset.originalUrl);
			const last = url.pathname.split('/').filter(Boolean).pop();
			return last || url.hostname;
		} catch {
			return asset.originalUrl;
		}
	}

	/**
	 * Opens the confirmation dialog when a user tries to download an
	 * embedded attachment. Since embedded attachments are not stored
	 * separately, the user must download the entire EML file.
	 */
	function handleEmbeddedAttachmentDownload(filename: string) {
		selectedEmbeddedFilename = filename;
		isEmbeddedAttachmentDialogOpen = true;
	}


	async function download(path: string, filename: string) {
		if (!browser) return;

		try {
			const response = await api(`/storage/download?path=${encodeURIComponent(path)}`);

			if (!response.ok) {
				throw new Error(`HTTP error! status: ${response.status}`);
			}

			const blob = await response.blob();
			const url = window.URL.createObjectURL(blob);
			const a = document.createElement('a');
			a.href = url;
			a.download = filename;
			document.body.appendChild(a);
			a.click();
			window.URL.revokeObjectURL(url);
			a.remove();
		} catch (error) {
			console.error('Download failed:', error);
		}
	}

	async function confirmDelete() {
		if (!email) return;
		try {
			isDeleting = true;
			const response = await api(`/archived-emails/${email.id}`, {
				method: 'DELETE',
			});
			if (!response.ok) {
				const errorData = await response.json().catch(() => null);
				const message = errorData?.message || 'Failed to delete email';
				console.error('Delete failed:', message);
				setAlert({
					type: 'error',
					title: 'Failed to delete archived email',
					message: message,
					duration: 5000,
					show: true,
				});
				return;
			}
			await goto('/mailbox', { invalidateAll: true });
		} catch (error) {
			console.error('Delete failed:', error);
		} finally {
			isDeleting = false;
			isDeleteDialogOpen = false;
		}
	}

	function remoteContentLabel(status: string | undefined): string {
		switch (status) {
			case 'archived':
				return 'Remote archived';
			case 'partial':
				return 'Remote partial';
			case 'failed':
				return 'Remote failed';
			case 'skipped':
				return 'No remote content';
			case 'pending':
				return 'Remote pending';
			default:
				return 'Remote not started';
		}
	}

	function assetCountLabel(count: number): string {
		return `${count} local asset${count === 1 ? '' : 's'}`;
	}

	function isRemoteContentComplete(status: RemoteContentStatus): boolean {
		return status !== 'not_started' && status !== 'pending';
	}

	function remoteContentResultMessage(preview: RemoteContentPreview): string {
		if (preview.status === 'archived') {
			return `${preview.archivedAssetCount} remote asset${preview.archivedAssetCount === 1 ? '' : 's'} archived.`;
		}
		if (preview.status === 'partial') {
			return `${preview.archivedAssetCount} archived, ${preview.failedAssetCount} failed, and ${preview.blockedAssetCount} blocked.`;
		}
		if (preview.status === 'skipped') {
			return 'This email has no remote content to archive.';
		}
		return `No assets were archived. ${preview.failedAssetCount} failed and ${preview.blockedAssetCount} were blocked.`;
	}

	async function pollRemoteContentArchive(emailId: string): Promise<void> {
		try {
			for (let attempt = 0; attempt < 60; attempt += 1) {
				await new Promise((resolve) => setTimeout(resolve, 1000));
				if (email?.id !== emailId) return;

				const response = await api(`/archived-emails/${emailId}/preview`);
				const preview = (await response.json()) as RemoteContentPreview & {
					message?: string;
				};
				if (!response.ok) {
					throw new Error(preview.message || 'Failed to check remote content status');
				}

				remoteContentResult = preview;
				if (!isRemoteContentComplete(preview.status)) continue;

				remoteContentQueued = false;
				remoteContentRefreshKey += 1;
				setAlert({
					type:
						preview.status === 'archived'
							? 'success'
							: preview.status === 'partial' || preview.status === 'skipped'
								? 'warning'
								: 'error',
					title:
						preview.status === 'failed'
							? 'Remote content archive failed'
							: 'Remote content archive complete',
					message: remoteContentResultMessage(preview),
					duration: 6000,
					show: true,
				});
				return;
			}

			remoteContentQueued = false;
			setAlert({
				type: 'warning',
				title: 'Remote content still processing',
				message:
					'The job is taking longer than expected. Its current status is shown above.',
				duration: 6000,
				show: true,
			});
		} catch (error) {
			remoteContentQueued = false;
			setAlert({
				type: 'error',
				title: 'Unable to check remote content status',
				message: error instanceof Error ? error.message : 'Status check failed',
				duration: 5000,
				show: true,
			});
		}
	}

	async function queueRemoteContentArchive() {
		if (!email) return;

		isQueueingRemoteContent = true;
		try {
			const response = await api(`/archived-emails/${email.id}/remote-content/archive`, {
				method: 'POST',
			});
			const body = await response.json();
			if (!response.ok) {
				throw new Error(body.message || 'Failed to queue remote content archive');
			}
			remoteContentQueued = true;
			setAlert({
				type: 'success',
				title: 'Remote content queued',
				message: 'The remote content worker is processing this email.',
				duration: 4000,
				show: true,
			});
			void pollRemoteContentArchive(email.id);
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Remote content archive failed',
				message:
					error instanceof Error
						? error.message
						: 'Failed to queue remote content archive',
				duration: 5000,
				show: true,
			});
		} finally {
			isQueueingRemoteContent = false;
		}
	}
</script>

<svelte:head>
	<title>{email?.subject} | {$t('app.archive.title')} - OpenArchiver</title>
</svelte:head>

{#if email}
	<div class="mb-4">
		<Button variant="ghost" size="sm" class="gap-2" onclick={goBack}>
			<ArrowLeft class="h-4 w-4" />
			{$t('app.archive.back_to_mailbox')}
		</Button>
	</div>
	<div class="grid grid-cols-3 gap-6">
		<div class="col-span-3 md:col-span-2">
			<Card.Root>
				<Card.Header>
					<Card.Title>{email.subject || $t('app.archive.no_subject')}</Card.Title>
					<Card.Description>
						{$t('app.archive.from')}: {identityLabel(email.senderEmail, email.senderName)} | {$t(
							'app.archive.sent'
						)}: {formatDateTime(email.sentAt)}
					</Card.Description>
				</Card.Header>
				<Card.Content>
					<div class="space-y-4">
						<div class="space-y-1">
							<h3 class="font-semibold">{$t('app.archive.recipients')}</h3>
							<p class="text-muted-foreground text-sm">
								{$t('app.archive.to')}: {email.recipients
									.map((r) => identityLabel(r.email, r.name))
									.join(', ')}
							</p>
						</div>
						<div class="space-y-1">
							<h3 class="font-semibold">{$t('app.archive.meta_data')}</h3>
							<div class="text-muted-foreground space-y-2 text-sm">
								<div class="flex flex-wrap items-center gap-2">
									<span>{$t('app.archived_emails_page.inbox')}:</span>
									<span class="bg-muted truncate rounded p-1.5 text-xs"
										>{email.userEmail}</span
									>
								</div>
								<div class="flex flex-wrap items-center gap-2">
									<span>{$t('app.archive.tags')}:</span>
									{#each localTags as tag (tag)}
										<span
											class="bg-muted flex items-center gap-1 rounded py-1 pl-1.5 pr-1 text-xs"
										>
											{tag}
											<button
												type="button"
												class="text-muted-foreground hover:text-foreground disabled:opacity-50"
												disabled={isUpdatingTags}
												onclick={() => removeTag(tag)}
												aria-label={`Remove tag ${tag}`}
											>
												<X class="h-3 w-3" />
											</button>
										</span>
									{/each}
									<TagCombobox
										existingTags={tagSuggestions}
										disabled={isUpdatingTags}
										onSelect={addTagByName}
									/>
								</div>
								<div class="flex flex-wrap items-center gap-2">
									<span>{$t('app.archive.size')}:</span>
									<span class="bg-muted truncate rounded p-1.5 text-xs"
										>{formatBytes(email.sizeBytes)}</span
									>
								</div>
							</div>
						</div>
						<div>
							<div class="flex flex-wrap items-center justify-between gap-2">
								<div class="flex flex-wrap items-center gap-2">
									<h3 class="font-semibold">{$t('app.archive.email_preview')}</h3>
									<Badge variant="secondary">
										{remoteContentLabel(currentRemoteContentStatus)}
									</Badge>
									{#if currentRemoteContentAssetCount > 0}
										<Badge variant="outline">
											{assetCountLabel(currentRemoteContentAssetCount)}
										</Badge>
									{/if}
								</div>
								{#if currentRemoteContentStatus !== 'archived' && currentRemoteContentStatus !== 'skipped'}
									<Button
										type="button"
										variant="outline"
										size="sm"
										class="gap-2 text-xs"
										disabled={isQueueingRemoteContent || remoteContentQueued}
										onclick={queueRemoteContentArchive}
									>
										<Archive class="h-3.5 w-3.5" />
										{#if isQueueingRemoteContent}
											Queueing...
										{:else if remoteContentQueued}
											Processing...
										{:else if currentRemoteContentStatus === 'failed'}
											Retry Remote Content
										{:else}
											Archive Remote Content
										{/if}
									</Button>
								{/if}
							</div>
							<EmailPreview emailId={email.id} refreshKey={remoteContentRefreshKey} />
						</div>
					</div>
				</Card.Content>
			</Card.Root>
		</div>
		<div class="col-span-3 space-y-6 md:col-span-1">
			<Card.Root>
				<Card.Header>
					<Card.Title>{$t('app.archive.actions')}</Card.Title>
				</Card.Header>
				<Card.Content class="space-y-2">
					<Button
						class="text-xs"
						onclick={() =>
							download(email.storagePath, `${email.subject || 'email'}.eml`)}
						>{$t('app.archive.download_eml')}</Button
					>
					<Button
						variant="destructive"
						class="text-xs"
						onclick={() => (isDeleteDialogOpen = true)}
					>
						{$t('app.archive.delete_email')}
					</Button>
				</Card.Content>
			</Card.Root>


			<!-- Attachments (collapsed, with inline preview where possible) -->
			{#if email.attachments && email.attachments.length > 0}
				<Card.Root>
					<Card.Header>
						<Card.Title>{$t('app.archive.attachments')}</Card.Title>
					</Card.Header>
					<Card.Content class="space-y-2">
						{#each email.attachments as attachment (attachment.id)}
							<AttachmentPreview
								title={attachment.filename}
								sizeBytes={attachment.sizeBytes}
								mimeType={attachment.mimeType}
								fetchBlob={() => fetchAttachmentBlob(attachment.storagePath)}
							/>
						{/each}
					</Card.Content>
				</Card.Root>
			{:else if embeddedAttachments.length > 0}
				<Card.Root>
					<Card.Header>
						<Card.Title>{$t('app.archive.attachments')}</Card.Title>
					</Card.Header>
					<Card.Content class="space-y-2">
						{#each embeddedAttachments as attachment, i (attachment.filename || i)}
							<AttachmentPreview
								title={attachment.filename || 'attachment'}
								sizeBytes={typeof attachment.content === 'string'
									? attachment.content.length
									: (attachment.content?.byteLength ?? null)}
								mimeType={attachment.mimeType}
								fetchBlob={() =>
									Promise.resolve(
										new Blob([attachment.content], {
											type: attachment.mimeType || 'application/octet-stream'
										})
									)}
							/>
						{/each}
					</Card.Content>
				</Card.Root>
			{/if}

			<!-- Remote content: archived (collapsed previews) + failed/blocked (with reason) -->
			{#if remoteAssets.length > 0}
				<Card.Root>
					<Card.Header>
						<Card.Title>{$t('app.archive.remote_content')}</Card.Title>
					</Card.Header>
					<Card.Content class="space-y-2">
						{#each archivedRemoteAssets as asset (asset.id)}
							<AttachmentPreview
								title={remoteAssetTitle(asset)}
								sizeBytes={asset.sizeBytes}
								mimeType={asset.contentType}
								canPreview={asset.previewable}
								fetchBlob={() => fetchRemoteAssetBlob(asset.id)}
							/>
						{/each}

						{#if failedRemoteAssets.length > 0}
							<div class="space-y-2 pt-1">
								<p class="text-muted-foreground text-xs font-medium">
									{$t('app.archive.remote_content_failed', {
										count: failedRemoteAssets.length
									} as any)}
								</p>
								{#each failedRemoteAssets as asset (asset.id)}
									<div class="rounded-md border p-2 text-xs">
										<div class="flex items-center justify-between gap-2">
											<a
												href={asset.originalUrl}
												target="_blank"
												rel="noreferrer"
												class="truncate hover:underline"
												title={asset.originalUrl}>{remoteAssetTitle(asset)}</a
											>
											<Badge
												variant={asset.status === 'blocked'
													? 'secondary'
													: 'destructive'}
												class="flex-shrink-0 capitalize">{asset.status}</Badge
											>
										</div>
										{#if asset.failureReason}
											<p class="text-muted-foreground mt-1 break-words font-mono">
												{asset.failureReason}
											</p>
										{/if}
									</div>
								{/each}
							</div>
						{/if}
					</Card.Content>
				</Card.Root>
			{/if}

			<!-- Thread discovery -->
			{#if email.thread && email.thread.length > 1}
				<Card.Root>
					<Card.Header>
						<Card.Title>{$t('app.archive.email_thread')}</Card.Title>
					</Card.Header>
					<Card.Content>
						<EmailThread thread={email.thread} currentEmailId={email.id} />
					</Card.Content>
				</Card.Root>
			{/if}
		</div>
	</div>

	<Dialog.Root bind:open={isDeleteDialogOpen}>
		<Dialog.Content class="sm:max-w-lg">
			<Dialog.Header>
				<Dialog.Title>{$t('app.archive.delete_confirmation_title')}</Dialog.Title>
				<Dialog.Description>
					{$t('app.archive.delete_confirmation_description')}
				</Dialog.Description>
			</Dialog.Header>
			<Dialog.Footer class="sm:justify-start">
				<Button
					type="button"
					variant="destructive"
					onclick={confirmDelete}
					disabled={isDeleting}
				>
					{#if isDeleting}
						{$t('app.archive.deleting')}...
					{:else}
						{$t('app.archive.confirm')}
					{/if}
				</Button>
				<Dialog.Close>
					<Button type="button" variant="secondary">{$t('app.archive.cancel')}</Button>
				</Dialog.Close>
			</Dialog.Footer>
		</Dialog.Content>
	</Dialog.Root>

	<!-- Embedded attachment download confirmation modal -->
	<Dialog.Root bind:open={isEmbeddedAttachmentDialogOpen}>
		<Dialog.Content class="sm:max-w-lg">
			<Dialog.Header>
				<Dialog.Title>
					{$t('app.archive.embedded_attachment_title')}
				</Dialog.Title>
				<Dialog.Description>
					<span class="font-medium">{selectedEmbeddedFilename}</span>
					<br /><br />
					{$t('app.archive.embedded_attachment_description')}
				</Dialog.Description>
			</Dialog.Header>
			<Dialog.Footer class="sm:justify-start">
				<Button
					type="button"
					onclick={() => {
						download(email.storagePath, `${email.subject || 'email'}.eml`);
						isEmbeddedAttachmentDialogOpen = false;
					}}
				>
					{$t('app.archive.download_eml')}
				</Button>
				<Dialog.Close>
					<Button type="button" variant="secondary">
						{$t('app.archive.cancel')}
					</Button>
				</Dialog.Close>
			</Dialog.Footer>
		</Dialog.Content>
	</Dialog.Root>
{:else}
	<p>{$t('app.archive.not_found')}</p>
{/if}
