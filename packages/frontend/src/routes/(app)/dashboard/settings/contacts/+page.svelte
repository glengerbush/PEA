<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import * as Card from '$lib/components/ui/card';
	import { api } from '$lib/api.client';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import type { ImportContactsResult, ContactImportFormat } from '@open-archiver/types';
	import Upload from 'lucide-svelte/icons/upload';

	let fileInput = $state<HTMLInputElement | null>(null);
	let selectedFile = $state<File | null>(null);
	let isImporting = $state(false);
	let lastResult = $state<ImportContactsResult | null>(null);

	function onFileChange(e: Event) {
		const input = e.currentTarget as HTMLInputElement;
		selectedFile = input.files?.[0] ?? null;
	}

	function detectFormat(name: string): ContactImportFormat | null {
		const lower = name.toLowerCase();
		if (lower.endsWith('.csv')) return 'csv';
		if (lower.endsWith('.vcf') || lower.endsWith('.vcard')) return 'vcf';
		return null;
	}

	async function importContacts() {
		if (!selectedFile) return;
		const format = detectFormat(selectedFile.name);
		if (!format) {
			setAlert({
				type: 'error',
				title: 'Unsupported file',
				message: 'Please choose a .csv or .vcf (vCard) file.',
				duration: 5000,
				show: true,
			});
			return;
		}
		isImporting = true;
		try {
			const content = await selectedFile.text();
			const response = await api('/contacts/import', {
				method: 'POST',
				body: JSON.stringify({ format, content }),
			});
			const body = await response.json();
			if (!response.ok) throw new Error(body.message || 'Import failed');
			lastResult = body as ImportContactsResult;
			setAlert({
				type: 'success',
				title: 'Contacts imported',
				message: `${lastResult.imported} added, ${lastResult.updated} updated`,
				duration: 4000,
				show: true,
			});
			selectedFile = null;
			if (fileInput) fileInput.value = '';
		} catch (error) {
			setAlert({
				type: 'error',
				title: 'Import failed',
				message: error instanceof Error ? error.message : 'Import failed',
				duration: 6000,
				show: true,
			});
		} finally {
			isImporting = false;
		}
	}
</script>

<div class="mx-auto max-w-2xl space-y-6">
	<div>
		<h1 class="text-2xl font-bold">Contacts</h1>
		<p class="text-muted-foreground text-sm">
			Import contacts from a CSV or vCard (.vcf) file. Matching names are shown next to email
			addresses across the archive (stacked above the address to save space).
		</p>
	</div>

	<Card.Root>
		<Card.Header>
			<Card.Title>Import contacts</Card.Title>
			<Card.Description>
				Accepts a <strong>.csv</strong> (with an email column — and optionally name / first
				/ last columns) or a <strong>.vcf / vCard</strong> file. Existing contacts with the
				same email are updated.
			</Card.Description>
		</Card.Header>
		<Card.Content class="space-y-4">
			<input
				bind:this={fileInput}
				type="file"
				accept=".csv,.vcf,.vcard,text/csv,text/vcard"
				onchange={onFileChange}
				class="border-input file:bg-muted file:text-foreground block w-full cursor-pointer rounded-md border text-sm file:mr-4 file:cursor-pointer file:border-0 file:px-4 file:py-2 file:text-sm"
			/>
			<Button onclick={importContacts} disabled={!selectedFile || isImporting} class="gap-2">
				<Upload class="h-4 w-4" />
				{isImporting ? 'Importing…' : 'Import'}
			</Button>
			{#if lastResult}
				<p class="text-muted-foreground text-sm">
					Parsed {lastResult.parsed}, added {lastResult.imported}, updated {lastResult.updated}{lastResult.skipped
						? `, skipped ${lastResult.skipped} duplicate${lastResult.skipped === 1 ? '' : 's'}`
						: ''}.
				</p>
			{/if}
		</Card.Content>
	</Card.Root>
</div>
