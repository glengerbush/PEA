<script lang="ts">
	import type { PageData } from './$types';
	import { Button, buttonVariants } from '$lib/components/ui/button';
	import * as Tooltip from '$lib/components/ui/tooltip';
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
	import Clock from '@lucide/svelte/icons/clock';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import CalendarClock from '@lucide/svelte/icons/calendar-clock';
	import AlertCircle from '@lucide/svelte/icons/alert-circle';
	import Shield from '@lucide/svelte/icons/shield';
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import RotateCw from '@lucide/svelte/icons/rotate-cw';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Download from '@lucide/svelte/icons/download';
	import Reply from '@lucide/svelte/icons/reply';
	import ScrollText from '@lucide/svelte/icons/scroll-text';
	import X from '@lucide/svelte/icons/x';
	import { get } from 'svelte/store';
	import { lastMailboxListUrl } from '$lib/stores/mailbox-nav';
	import { disableTwoFingerSwipe } from '$lib/stores/swipe.store';
	import { page } from '$app/state';
	import { enhance } from '$app/forms';
	import type {
		RemoteContentAssetSummary,
		RemoteContentPreview,
		RemoteContentStatus,
		UpdateArchivedEmailTagsResult,
	} from '@pea/types';
	import PostalMime, { type Attachment as PostalAttachment } from 'postal-mime';
	import Paperclip from '@lucide/svelte/icons/paperclip';

	let { data }: { data: PageData } = $props();
	let email = $derived(data.email);

	/** Where "back" should go: the explicit origin passed in `?from=` (e.g. the
	 *  duplicates page with its filters/page), else the last mailbox list view,
	 *  else the mailbox root when the email was opened directly. */
	function backTarget(): string {
		return page.url.searchParams.get('from') || get(lastMailboxListUrl) || '/mailbox';
	}

	function goBack() {
		goto(backTarget());
	}

	/** "Name <email>" when a name (contact or header) is known, else the bare address. */
	function identityLabel(addr: string | null | undefined, fallback?: string | null): string {
		const email = (addr || '').trim();
		const name = (contactName(email) || fallback || '').trim();
		return name && name.toLowerCase() !== email.toLowerCase() ? `${name} <${email}>` : email;
	}

	function formatRecipients(list: { email: string; name?: string }[]): string {
		return list.map((r) => identityLabel(r.email, r.name)).join(', ');
	}

	// Recipients split by header, so the view can show separate To/Cc/Bcc lines.
	// `kind` is optional for older records — treat a missing kind as "to".
	let toRecipients = $derived((email?.recipients ?? []).filter((r) => (r.kind ?? 'to') === 'to'));
	let ccRecipients = $derived((email?.recipients ?? []).filter((r) => r.kind === 'cc'));
	let bccRecipients = $derived((email?.recipients ?? []).filter((r) => r.kind === 'bcc'));

	// --- Tag editing (add/remove on this single email) ---
	let localTags = $state<string[]>([]);
	$effect(() => {
		localTags = Array.isArray(email?.tags) ? [...email.tags] : [];
	});
	let isUpdatingTags = $state(false);
	// Existing tags across the archive, minus the ones already on this email.
	let tagSuggestions = $derived((data.allTags ?? []).filter((t) => !localTags.includes(t)));

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
			// Ignore a response that resolved after the user navigated to another
			// email — otherwise the previous email's tags render under the new one.
			if (updated && email?.id === emailId) localTags = updated.tags;
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
	let metadataOpen = $state(false);
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

	// The raw .eml is fetched lazily from /raw (not embedded in the detail JSON)
	// and cached per email so the three consumers below share one download.
	let rawBytesCache = $state<{ id: string; bytes: Uint8Array } | null>(null);
	async function getRawBytes(): Promise<Uint8Array | null> {
		const id = email?.id;
		if (!id) return null;
		if (rawBytesCache?.id === id) return rawBytesCache.bytes;
		const response = await api(`/archived-emails/${id}/raw`);
		if (!response.ok) return null;
		const bytes = new Uint8Array(await response.arrayBuffer());
		if (email?.id !== id) return null; // navigated away mid-fetch
		rawBytesCache = { id, bytes };
		return bytes;
	}

	/** Parse raw EML to extract non-inline attachments for display */
	$effect(() => {
		const currentId = email?.id;
		// Clear immediately so a previous email's attachments never linger while
		// the new email parses (or if the new email has no raw content).
		embeddedAttachments = [];

		async function parseEmlAttachments() {
			const bytes = await getRawBytes();
			if (!bytes || email?.id !== currentId) return;

			try {
				const parsed = await new PostalMime().parse(bytes);
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

	// Retry a single failed/blocked remote asset without touching the others.
	let retryingAssetId = $state<string | null>(null);
	async function retryAsset(assetId: string) {
		const id = email?.id;
		if (!id) return;
		retryingAssetId = assetId;
		try {
			const response = await api(`/archived-emails/${id}/remote-assets/${assetId}/retry`, {
				method: 'POST'
			});
			if (!response.ok) throw new Error('Retry failed');
			// Reflect the new status and refresh the preview in case it now renders.
			if (email?.id === id) {
				await loadRemoteAssets(id);
				remoteContentRefreshKey += 1;
			}
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Retry failed',
				message: error instanceof Error ? error.message : 'Could not retry this item',
				duration: 5000,
				show: true
			});
		} finally {
			retryingAssetId = null;
		}
	}

	async function loadRemoteAssets(id: string): Promise<void> {
		try {
			const response = await api(`/archived-emails/${id}/remote-assets`);
			const assets = response.ok
				? ((await response.json()) as RemoteContentAssetSummary[])
				: [];
			// Ignore a response that resolved after the user navigated away, or
			// it would show email A's assets under email B (and 404 on click).
			if (email?.id !== id) return;
			remoteAssets = assets;
		} catch {
			if (email?.id === id) remoteAssets = [];
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

	/** Downloads the .eml reconstructed from storage (attachments spliced back). */
	async function downloadEml() {
		if (!email || !browser) return;
		try {
			const response = await api(`/archived-emails/${email.id}/eml`);
			if (!response.ok) {
				throw new Error(`HTTP error! status: ${response.status}`);
			}
			const blob = await response.blob();
			const url = window.URL.createObjectURL(blob);
			const a = document.createElement('a');
			a.href = url;
			a.download = `${email.subject || 'email'}.eml`;
			document.body.appendChild(a);
			a.click();
			window.URL.revokeObjectURL(url);
			a.remove();
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Download failed',
				message: error instanceof Error ? error.message : 'Failed to download email',
				duration: 5000,
				show: true,
			});
		}
	}

	// --- Download all attachments as a zip ---
	let isDownloadingAll = $state(false);
	async function downloadAllAttachments() {
		if (!email || !browser) return;
		isDownloadingAll = true;
		try {
			const response = await api(`/archived-emails/${email.id}/attachments/archive`);
			if (!response.ok) {
				throw new Error(`HTTP error! status: ${response.status}`);
			}
			const blob = await response.blob();
			const url = window.URL.createObjectURL(blob);
			const a = document.createElement('a');
			a.href = url;
			a.download = `${email.subject || 'email'} - attachments.zip`;
			document.body.appendChild(a);
			a.click();
			window.URL.revokeObjectURL(url);
			a.remove();
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Download failed',
				message: error instanceof Error ? error.message : 'Failed to download attachments',
				duration: 5000,
				show: true,
			});
		} finally {
			isDownloadingAll = false;
		}
	}

	/** Opens an attachment in the OS quick-look previewer (qlmanage / sushi). */
	async function quickLook(storagePath: string) {
		try {
			const response = await api('/attachments/quicklook', {
				method: 'POST',
				body: JSON.stringify({ path: storagePath }),
			});
			if (!response.ok) {
				throw new Error((await response.text()) || 'Failed to open preview');
			}
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Quick Look failed',
				message: error instanceof Error ? error.message : 'Failed to open preview',
				duration: 5000,
				show: true,
			});
		}
	}

	// --- Copy a quoted reply for pasting into an email client ---
	let isCopyingReply = $state(false);

	function escapeHtml(value: string): string {
		return value
			.replaceAll('&', '&amp;')
			.replaceAll('<', '&lt;')
			.replaceAll('>', '&gt;')
			.replaceAll('"', '&quot;');
	}

	/** Plain-text rendering of an html fragment, keeping line breaks. */
	function htmlToPlainText(html: string): string {
		const withBreaks = html
			.replace(/<\s*br[^>]*>/gi, '\n')
			.replace(/<\/(p|div|tr|li|h[1-6]|blockquote)\s*>/gi, '\n');
		const doc = new DOMParser().parseFromString(withBreaks, 'text/html');
		return (doc.body.textContent ?? '').replace(/\n{3,}/g, '\n\n').trim();
	}

	async function buildReply(): Promise<{ text: string; html: string }> {
		const current = email;
		if (!current) return { text: '', html: '' };

		let text = '';
		let html = '';
		const rawBytes = await getRawBytes();
		if (rawBytes) {
			try {
				const parsed = await new PostalMime().parse(rawBytes);
				text = parsed.text ?? '';
			} catch {
				// fall through to the preview below
			}
		}
		// The rendered preview is what the reader actually sees (sanitized html,
		// inline images as data URIs) — quote that, not the raw mime parts.
		try {
			const response = await api(`/archived-emails/${current.id}/preview`);
			if (response.ok) {
				const preview = (await response.json()) as RemoteContentPreview;
				const match = /<body>([\s\S]*)<\/body>/i.exec(preview.html ?? '');
				html = (match ? match[1] : '').trim();
			}
		} catch {
			// keep whatever the raw parse produced
		}
		if (!text.trim() && html) text = htmlToPlainText(html);
		if (!text.trim() && !html) text = current.subject || '';

		const attribution = `On ${formatDateTime(current.sentAt)}, ${identityLabel(
			current.senderEmail,
			current.senderName
		)} wrote:`;
		const quotedText =
			`${attribution}\n\n` +
			text
				.trimEnd()
				.split('\n')
				.map((line) => `> ${line}`)
				.join('\n');
		const htmlBody = html || escapeHtml(text).replaceAll('\n', '<br>');
		const quotedHtml =
			`<p>${escapeHtml(attribution)}</p>` +
			`<blockquote type="cite" style="margin:0 0 0 0.8ex;border-left:1px solid #ccc;padding-left:1ex">${htmlBody}</blockquote>`;
		return { text: quotedText, html: quotedHtml };
	}

	async function copyReplyToClipboard() {
		if (!email) return;
		isCopyingReply = true;
		try {
			const { text, html } = await buildReply();
			// The desktop shell writes both clipboard flavors natively — the
			// WebKitGTK webview rejects the async Clipboard API outside a strict
			// user-gesture window, so it can't be relied on here.
			const response = await api('/native/clipboard', {
				method: 'POST',
				body: JSON.stringify({ text, html }),
			});
			if (!response.ok) {
				// Dev in a plain browser: no shell endpoint — try the web API.
				if (typeof ClipboardItem !== 'undefined' && navigator.clipboard?.write) {
					await navigator.clipboard.write([
						new ClipboardItem({
							'text/plain': new Blob([text], { type: 'text/plain' }),
							'text/html': new Blob([html], { type: 'text/html' }),
						}),
					]);
				} else {
					await navigator.clipboard.writeText(text);
				}
			}
			setAlert({
				type: 'success',
				title: 'Reply copied',
				message: 'Paste it into a new email in your mail client.',
				duration: 4000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Copy failed',
				message: error instanceof Error ? error.message : 'Failed to copy reply',
				duration: 5000,
				show: true,
			});
		} finally {
			isCopyingReply = false;
		}
	}

	// --- Raw header viewer ---
	let isHeadersDialogOpen = $state(false);
	let emailHeaders = $state('');

	async function showHeaders() {
		const bytes = await getRawBytes();
		if (!bytes) {
			setAlert({
				type: 'error',
				title: 'Headers unavailable',
				message: 'The stored email could not be loaded.',
				duration: 5000,
				show: true,
			});
			return;
		}
		// The header block is everything before the first blank line.
		const decoded = new TextDecoder('utf-8', { fatal: false }).decode(bytes);
		const end = decoded.search(/\r?\n\r?\n/);
		emailHeaders = (end === -1 ? decoded : decoded.slice(0, end)).trimEnd();
		isHeadersDialogOpen = true;
	}

	async function copyHeaders() {
		try {
			const response = await api('/native/clipboard', {
				method: 'POST',
				body: JSON.stringify({ text: emailHeaders }),
			});
			if (!response.ok) {
				await navigator.clipboard.writeText(emailHeaders);
			}
			setAlert({
				type: 'success',
				title: 'Headers copied',
				message: 'The raw headers are on your clipboard.',
				duration: 3000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Copy failed',
				message: error instanceof Error ? error.message : 'Failed to copy headers',
				duration: 5000,
				show: true,
			});
		}
	}

	// --- Two-finger (horizontal) swipe returns to the previous screen ---
	let swipeAccum = 0;
	let swipeResetTimer: ReturnType<typeof setTimeout> | null = null;
	let swipeCooldownUntil = 0;
	/** 0→1 progress toward the swipe threshold, drives the on-screen affordance. */
	let swipeProgress = $state(0);
	function handleWheel(event: WheelEvent) {
		if (get(disableTwoFingerSwipe)) return;
		// Only count clearly-horizontal movement so vertical scrolling never triggers.
		if (Math.abs(event.deltaX) <= Math.abs(event.deltaY) * 1.5) return;
		const now = Date.now();
		if (now < swipeCooldownUntil) return;
		swipeAccum += event.deltaX;
		swipeProgress = Math.min(1, Math.abs(swipeAccum) / 300);
		if (swipeResetTimer) clearTimeout(swipeResetTimer);
		swipeResetTimer = setTimeout(() => {
			swipeAccum = 0;
			swipeProgress = 0;
		}, 400);
		if (Math.abs(swipeAccum) >= 300) {
			swipeAccum = 0;
			swipeProgress = 0;
			swipeCooldownUntil = now + 1000;
			goBack();
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
			await goto(backTarget(), { invalidateAll: true });
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
	<title>{email?.subject} | {$t('app.archive.title')} - PEA</title>
</svelte:head>

<svelte:window onwheel={handleWheel} />

<!-- Two-finger swipe affordance: a back indicator that slides in and fills as
     the gesture approaches the threshold, then completes into the navigation. -->
{#if swipeProgress > 0}
	<div
		class="pointer-events-none fixed top-1/2 left-2 z-50"
		style="opacity:{swipeProgress}; transform: translate({-44 + swipeProgress * 44}px, -50%);"
		aria-hidden="true"
	>
		<div
			class="bg-primary/90 text-primary-foreground flex h-12 w-12 items-center justify-center rounded-full shadow-lg backdrop-blur"
			style="transform: scale({0.7 + swipeProgress * 0.3});"
		>
			<ArrowLeft class="h-6 w-6" />
		</div>
	</div>
{/if}

{#if email}
	<div class="mb-4">
		<Button variant="ghost" size="sm" class="gap-2" onclick={goBack}>
			<ArrowLeft class="h-4 w-4" />
			{$t('app.archive.back_to_mailbox')}
		</Button>
	</div>
	<div class="grid grid-cols-3 gap-6">
		<div class="col-span-3 md:col-span-2">
			<Card.Root class="gap-1.5">
				<Card.Header class="relative">
					<div class="flex flex-col-reverse gap-4 sm:block">
						<div class="min-w-0 space-y-1.5 sm:pr-40">
							<p class="text-sm">
								<span class="font-semibold">{$t('app.archive.from')}:</span>
								<span class="text-muted-foreground"
									>{identityLabel(email.senderEmail, email.senderName)}</span
								>
							</p>
							<Card.Title>{email.subject || $t('app.archive.no_subject')}</Card.Title>
							<p class="text-sm">
								<span class="font-semibold">{$t('app.archive.to')}:</span>
								<span class="text-muted-foreground"
									>{formatRecipients(toRecipients)}</span
								>
							</p>
							{#if ccRecipients.length > 0}
								<p class="text-sm">
									<span class="font-semibold">Cc:</span>
									<span class="text-muted-foreground"
										>{formatRecipients(ccRecipients)}</span
									>
								</p>
							{/if}
							{#if bccRecipients.length > 0}
								<p class="text-sm">
									<span class="font-semibold">Bcc:</span>
									<span class="text-muted-foreground"
										>{formatRecipients(bccRecipients)}</span
									>
								</p>
							{/if}
							<p class="text-muted-foreground text-xs">
								{$t('app.archive.sent')}: {formatDateTime(email.sentAt)}
							</p>
						</div>
						<div
							class="flex flex-col items-stretch gap-2 sm:absolute sm:right-6 sm:top-0 sm:w-32"
						>
							<div class="flex gap-2">
								<Tooltip.Root>
									<Tooltip.Trigger
										type="button"
										class="{buttonVariants({ size: 'sm' })} flex-1"
										aria-label={$t('app.archive.download_eml')}
										onclick={downloadEml}
									>
										<Download class="h-4 w-4" />
									</Tooltip.Trigger>
									<Tooltip.Content
										>{$t('app.archive.download_eml')}</Tooltip.Content
									>
								</Tooltip.Root>
								<Tooltip.Root>
									<Tooltip.Trigger
										type="button"
										class="{buttonVariants({
											variant: 'destructive',
											size: 'sm',
										})} flex-1"
										aria-label={$t('app.archive.delete_email')}
										onclick={() => (isDeleteDialogOpen = true)}
									>
										<Trash2 class="h-4 w-4" />
									</Tooltip.Trigger>
									<Tooltip.Content
										>{$t('app.archive.delete_email')}</Tooltip.Content
									>
								</Tooltip.Root>
							</div>
							<Button
								variant="outline"
								size="sm"
								class="justify-start gap-2 text-xs"
								onclick={showHeaders}
							>
								<ScrollText class="h-3.5 w-3.5" />
								{$t('app.archive.view_headers')}
							</Button>
							<Button
								variant="outline"
								size="sm"
								class="justify-start gap-2 text-xs"
								disabled={isCopyingReply}
								onclick={copyReplyToClipboard}
							>
								<Reply class="h-3.5 w-3.5" />
								{$t('app.archive.copy_reply')}
							</Button>
						</div>
					</div>
				</Card.Header>
				<Card.Content>
					<div class="space-y-4">
						<!-- Tags stay here; the rest of the metadata is in the collapsible Metadata panel on the right. -->
						<div class="text-muted-foreground flex flex-wrap items-center gap-2 text-sm">
							<span>{$t('app.archive.tags')}:</span>
							{#each localTags as tag (tag)}
								<span class="bg-muted flex items-center gap-1 rounded py-1 pl-1.5 pr-1 text-xs">
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
						<div>
							<EmailPreview emailId={email.id} refreshKey={remoteContentRefreshKey} />
						</div>
					</div>
				</Card.Content>
			</Card.Root>
		</div>
		<div class="col-span-3 space-y-6 md:col-span-1">
			{#snippet metaRow(label: string, value: string | null | undefined, mono = false)}
				{#if value}
					<div class="flex flex-col gap-0.5 sm:flex-row sm:gap-3">
						<dt class="text-muted-foreground w-44 flex-shrink-0 font-medium">{label}</dt>
						<dd class="min-w-0 break-all {mono ? 'font-mono' : ''}">{value}</dd>
					</div>
				{/if}
			{/snippet}

			<!-- Metadata (collapsible, debug-oriented; starts collapsed) -->
			<Card.Root class="gap-0 overflow-hidden py-0">
				<button
					type="button"
					class="hover:bg-muted/50 flex w-full items-center justify-between gap-2 px-4 py-3 text-left"
					aria-expanded={metadataOpen}
					onclick={() => (metadataOpen = !metadataOpen)}
				>
					<Card.Title>{$t('app.archive.meta_data')}</Card.Title>
					<ChevronDown class="text-muted-foreground h-4 w-4 flex-shrink-0 transition-transform {metadataOpen ? 'rotate-180' : ''}" />
				</button>
				{#if metadataOpen}
					<Card.Content class="pb-4 pt-0">
						<dl class="space-y-2 text-xs">
							{@render metaRow('Size', `${formatBytes(email.sizeBytes)} (${email.sizeBytes.toLocaleString()} bytes)`)}
							{@render metaRow('Sent', formatDateTime(email.sentAt))}
							{@render metaRow('Archived', formatDateTime(email.archivedAt))}
							{@render metaRow('Import Source', email.importSource)}
							{@render metaRow('Storage path', email.storagePath, true)}
							{@render metaRow('Original Folder', email.sourcePath)}

							{@render metaRow('Has attachments', email.hasAttachments ? 'Yes' : 'No')}
							{@render metaRow('Remote content status', email.remoteContentStatus)}
							{@render metaRow('Remote assets archived', String(email.remoteContentAssetCount))}
							{@render metaRow('Remote content archived', email.remoteContentArchivedAt ? formatDateTime(email.remoteContentArchivedAt) : null)}

							{@render metaRow('Ingestion source ID', email.ingestionSourceId, true)}
							{@render metaRow('Email ID', email.id, true)}
							{@render metaRow('Thread ID', email.threadId, true)}
							{@render metaRow('Message-ID header', email.messageIdHeader, true)}
							{@render metaRow('Provider message ID', email.providerMessageId, true)}

							{@render metaRow('Storage hash (SHA-256)', email.storageHashSha256, true)}

							{@render metaRow('Duplicate: subject hash', email.duplicateSubjectHash, true)}
							{@render metaRow('Duplicate: body hash', email.duplicateBodyHash, true)}
							{@render metaRow('Duplicate: recipient fingerprint', email.duplicateRecipientFingerprint, true)}
							{@render metaRow('Duplicate: attachment fingerprint', email.duplicateAttachmentFingerprint, true)}
						</dl>
					</Card.Content>
				{/if}
			</Card.Root>

			<!-- Attachments (collapsed, with inline preview where possible) -->
			{#if email.attachments && email.attachments.length > 0}
				<Card.Root>
					<Card.Header>
						<div class="flex flex-wrap items-center justify-between gap-2">
							<Card.Title>{$t('app.archive.attachments')}</Card.Title>
							{#if email.attachments.length > 1}
								<Button
									variant="outline"
									size="sm"
									class="gap-1 text-xs"
									disabled={isDownloadingAll}
									onclick={downloadAllAttachments}
								>
									<Download class="h-3.5 w-3.5" />
									{$t('app.archive.download_all_attachments')}
								</Button>
							{/if}
						</div>
					</Card.Header>
					<Card.Content class="space-y-2">
						{#each email.attachments as attachment (attachment.id)}
							<AttachmentPreview
								title={attachment.filename}
								sizeBytes={attachment.sizeBytes}
								mimeType={attachment.mimeType}
								description={attachment.contentDescription}
								createdAt={attachment.originalCreatedAt}
								modifiedAt={attachment.originalModifiedAt}
								fetchBlob={() => fetchAttachmentBlob(attachment.storagePath)}
								onQuickLook={() => quickLook(attachment.storagePath)}
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
										new Blob([attachment.content as BlobPart], {
											type: attachment.mimeType || 'application/octet-stream',
										})
									)}
							/>
						{/each}
					</Card.Content>
				</Card.Root>
			{/if}

			<!-- Remote content: archived (collapsed previews) + failed/blocked (with reason) -->
			{#if remoteAssets.length > 0}
				<Card.Root class="gap-3">
					<Card.Header class="gap-3">
						<Card.Title>{$t('app.archive.remote_content')}</Card.Title>
						<Card.Action>
							<Tooltip.Root>
								<Tooltip.Trigger
									type="button"
									class={buttonVariants({ variant: 'ghost', size: 'icon' })}
									aria-label="Retry all remote content"
									disabled={isQueueingRemoteContent || remoteContentQueued}
									onclick={queueRemoteContentArchive}
								>
									<RotateCw
										class="h-4 w-4 {isQueueingRemoteContent ||
										remoteContentQueued
											? 'animate-spin'
											: ''}"
									/>
								</Tooltip.Trigger>
								<Tooltip.Content>Retry all remote content</Tooltip.Content>
							</Tooltip.Root>
						</Card.Action>
						<div class="flex flex-wrap items-center gap-2">
							<Badge variant="secondary"
								>{remoteContentLabel(currentRemoteContentStatus)}</Badge
							>
							{#if currentRemoteContentAssetCount > 0}
								<Badge variant="outline"
									>{assetCountLabel(currentRemoteContentAssetCount)}</Badge
								>
							{/if}
						</div>
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
										count: failedRemoteAssets.length,
									} as any)}
								</p>
								{#each failedRemoteAssets as asset (asset.id)}
									<div class="rounded-md border p-2 text-xs">
										<div class="flex items-center justify-between gap-2">
											<Tooltip.Root>
												<Tooltip.Trigger>
													{#snippet child({ props })}
														<a
															{...props}
															href={asset.originalUrl}
															target="_blank"
															rel="noreferrer"
															class="truncate hover:underline"
															>{remoteAssetTitle(asset)}</a
														>
													{/snippet}
												</Tooltip.Trigger>
												<Tooltip.Content
													>{asset.originalUrl}</Tooltip.Content
												>
											</Tooltip.Root>
											<div class="flex flex-shrink-0 items-center gap-1">
												<Badge
													variant={asset.status === 'blocked' ? 'secondary' : 'destructive'}
													class="capitalize">{asset.status}</Badge
												>
												<Tooltip.Root>
													<Tooltip.Trigger
														type="button"
														class="{buttonVariants({ variant: 'ghost', size: 'icon' })} h-6 w-6"
														disabled={retryingAssetId === asset.id}
														onclick={() => retryAsset(asset.id)}
														aria-label="Retry this item"
													>
														<RotateCw class="h-3.5 w-3.5 {retryingAssetId === asset.id ? 'animate-spin' : ''}" />
													</Tooltip.Trigger>
													<Tooltip.Content>Retry this item</Tooltip.Content>
												</Tooltip.Root>
											</div>
										</div>
										{#if asset.failureReason}
											<p
												class="text-muted-foreground mt-1 break-words font-mono"
											>
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

	<!-- Raw header viewer -->
	<Dialog.Root bind:open={isHeadersDialogOpen}>
		<Dialog.Content class="sm:max-w-3xl">
			<Dialog.Header>
				<Dialog.Title>{$t('app.archive.view_headers')}</Dialog.Title>
				<Dialog.Description
					>{email.subject || $t('app.archive.no_subject')}</Dialog.Description
				>
			</Dialog.Header>
			<pre
				class="bg-muted max-h-[60vh] overflow-auto whitespace-pre-wrap break-all rounded-md p-3 font-mono text-xs">{emailHeaders}</pre>
			<Dialog.Footer class="sm:justify-start">
				<Button type="button" variant="outline" onclick={copyHeaders}>
					{$t('app.archive.copy_headers')}
				</Button>
				<Dialog.Close>
					<Button type="button" variant="secondary">{$t('app.archive.cancel')}</Button>
				</Dialog.Close>
			</Dialog.Footer>
		</Dialog.Content>
	</Dialog.Root>
{:else}
	<p>{$t('app.archive.not_found')}</p>
{/if}
