<script lang="ts">
	import type { PageData } from './$types';
	import * as Table from '$lib/components/ui/table';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import TablePagination from '$lib/components/custom/TablePagination.svelte';
	import { api } from '$lib/api.client';
	import { goto, afterNavigate, beforeNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { formatDateTime } from '$lib/stores/datetime.svelte';
	import { saveListScroll, getListScroll, lastOpenedEmailId } from '$lib/stores/list-view-state';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import Trash2 from '@lucide/svelte/icons/trash-2';

	let { data }: { data: PageData } = $props();
	let trash = $derived(data.trash);

	let busy = $state<string | null>(null);
	let emptyDialogOpen = $state(false);
	let deleteForeverId = $state<string | null>(null);

	function buildPageUrl(p: number): string {
		const params = new URLSearchParams();
		if (p > 1) params.set('page', String(p));
		const query = params.toString();
		return `/trash${query ? `?${query}` : ''}`;
	}

	/** Open an email from the trash, remembering to return here afterwards. */
	function viewUrl(id: string): string {
		const from = page.url.pathname + page.url.search;
		return `/mailbox/${id}?from=${encodeURIComponent(from)}`;
	}

	// Restore scroll position and highlight the last-opened row on return.
	beforeNavigate(({ to }) => {
		saveListScroll(page.url.pathname + page.url.search, window.scrollY);
		const openedId = to?.url.pathname.match(/^\/mailbox\/([^/]+)$/)?.[1];
		if (openedId) lastOpenedEmailId.set(openedId);
	});
	afterNavigate(() => {
		const y = getListScroll(page.url.pathname + page.url.search);
		if (y !== undefined) requestAnimationFrame(() => window.scrollTo(0, y));
	});

	async function reload() {
		await goto(page.url.pathname + page.url.search, {
			invalidateAll: true,
			keepFocus: true,
			replaceState: true,
			noScroll: true,
		});
	}

	async function postJson(path: string, body: unknown): Promise<Response> {
		return api(path, { method: 'POST', body: JSON.stringify(body) });
	}

	async function restore(id: string) {
		busy = `restore-${id}`;
		try {
			const response = await postJson('/archived-emails/trash/restore', { emailIds: [id] });
			if (!response.ok) throw new Error((await response.json()).message || 'Failed to restore');
			await reload();
			setAlert({ type: 'success', title: 'Restored', message: 'Email restored', duration: 2500, show: true });
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Restore failed',
				message: error instanceof Error ? error.message : 'Failed to restore',
				duration: 5000,
				show: true,
			});
		} finally {
			busy = null;
		}
	}

	async function deleteForever() {
		const id = deleteForeverId;
		if (!id) return;
		busy = `delete-${id}`;
		try {
			const response = await postJson('/archived-emails/trash/delete', { emailIds: [id] });
			if (!response.ok) throw new Error((await response.json()).message || 'Failed to delete');
			deleteForeverId = null;
			await reload();
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Delete failed',
				message: error instanceof Error ? error.message : 'Failed to delete',
				duration: 5000,
				show: true,
			});
		} finally {
			busy = null;
		}
	}

	async function emptyTrash() {
		busy = 'empty';
		try {
			const response = await postJson('/archived-emails/trash/empty', {});
			const body = await response.json();
			if (!response.ok) throw new Error(body.message || 'Failed to empty trash');
			emptyDialogOpen = false;
			await reload();
			setAlert({
				type: 'success',
				title: 'Trash emptied',
				message: `${body.deletedCount} email${body.deletedCount === 1 ? '' : 's'} permanently deleted`,
				duration: 3000,
				show: true,
			});
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Empty trash failed',
				message: error instanceof Error ? error.message : 'Failed to empty trash',
				duration: 5000,
				show: true,
			});
		} finally {
			busy = null;
		}
	}
</script>

<svelte:head>
	<title>Trash - PEA</title>
</svelte:head>

<div class="mb-4 flex items-end justify-between gap-4">
	<div>
		<h1 class="text-2xl font-bold">Trash</h1>
		<p class="text-muted-foreground text-sm">
			{trash.total}
			{trash.total === 1 ? 'item' : 'items'} · deleted emails are kept here until you empty the trash
		</p>
	</div>
	{#if trash.hits.length > 0}
		<Button
			type="button"
			variant="destructive"
			class="gap-2"
			disabled={busy !== null}
			onclick={() => (emptyDialogOpen = true)}
		>
			<Trash2 class="h-4 w-4" />
			Empty trash
		</Button>
	{/if}
</div>

{#if trash.hits.length > 0}
	<div class="rounded-md border">
		<Table.Root>
			<Table.Header>
				<Table.Row>
					<Table.Head>Sent</Table.Head>
					<Table.Head>Subject</Table.Head>
					<Table.Head>Sender</Table.Head>
					<Table.Head>Import Source</Table.Head>
					<Table.Head class="text-right">Actions</Table.Head>
				</Table.Row>
			</Table.Header>
			<Table.Body class="text-sm">
				{#each trash.hits as email (email.id)}
					<Table.Row class={email.id === $lastOpenedEmailId ? 'bg-primary/10' : ''}>
						<Table.Cell class="whitespace-nowrap">{formatDateTime(email.timestamp)}</Table.Cell>
						<Table.Cell>
							<a href={viewUrl(email.id)} class="block max-w-80 truncate hover:underline">
								{email.subject || '(no subject)'}
							</a>
						</Table.Cell>
						<Table.Cell>
							<span class="block max-w-48 truncate">
								{email.senderName || email.from}
							</span>
						</Table.Cell>
						<Table.Cell>
							<span class="block max-w-52 truncate">{email.importSource}</span>
						</Table.Cell>
						<Table.Cell class="text-right">
							<div class="flex justify-end gap-2">
								<Button
									type="button"
									variant="outline"
									size="sm"
									class="gap-2"
									disabled={busy !== null}
									onclick={() => restore(email.id)}
								>
									<RotateCcw class="h-3.5 w-3.5" />
									{busy === `restore-${email.id}` ? 'Restoring…' : 'Restore'}
								</Button>
								<Button
									type="button"
									variant="destructive"
									size="sm"
									class="gap-2"
									disabled={busy !== null}
									onclick={() => (deleteForeverId = email.id)}
								>
									<Trash2 class="h-3.5 w-3.5" />
									Delete forever
								</Button>
							</div>
						</Table.Cell>
					</Table.Row>
				{/each}
			</Table.Body>
		</Table.Root>
	</div>
{:else}
	<div class="rounded-md border p-8 text-center text-sm">Trash is empty.</div>
{/if}

<TablePagination
	count={trash.total}
	perPage={trash.limit}
	page={trash.page}
	buildHref={buildPageUrl}
	prevLabel="Prev"
	nextLabel="Next"
/>

<!-- Empty-trash confirmation -->
<Dialog.Root bind:open={emptyDialogOpen}>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>Empty trash?</Dialog.Title>
			<Dialog.Description>
				This permanently deletes all {trash.total} email{trash.total === 1 ? '' : 's'} in the trash,
				along with any attachments no longer used by a remaining email. This cannot be undone.
			</Dialog.Description>
		</Dialog.Header>
		<Dialog.Footer class="sm:justify-start">
			<Button type="button" variant="destructive" onclick={emptyTrash} disabled={busy !== null}>
				{busy === 'empty' ? 'Emptying…' : 'Permanently delete'}
			</Button>
			<Dialog.Close>
				<Button type="button" variant="secondary">Cancel</Button>
			</Dialog.Close>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<!-- Delete-forever (single) confirmation -->
<Dialog.Root
	open={deleteForeverId !== null}
	onOpenChange={(open) => {
		if (!open) deleteForeverId = null;
	}}
>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>Delete this email forever?</Dialog.Title>
			<Dialog.Description>
				This permanently removes the email and any attachments no longer used by a remaining email.
				This cannot be undone.
			</Dialog.Description>
		</Dialog.Header>
		<Dialog.Footer class="sm:justify-start">
			<Button type="button" variant="destructive" onclick={deleteForever} disabled={busy !== null}>
				{busy?.startsWith('delete-') ? 'Deleting…' : 'Permanently delete'}
			</Button>
			<Dialog.Close>
				<Button type="button" variant="secondary">Cancel</Button>
			</Dialog.Close>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
