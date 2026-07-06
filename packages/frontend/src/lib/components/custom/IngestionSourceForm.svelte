<script lang="ts">
	import type { SafeIngestionSource, CreateIngestionSourceDto } from '@pea/types';
	import { Button } from '$lib/components/ui/button';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import * as Select from '$lib/components/ui/select';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { api } from '$lib/api.client';
	import FileIcon from '@lucide/svelte/icons/file';
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import Info from '@lucide/svelte/icons/info';
	import { tooltip } from '$lib/actions/tooltip';
	import { t } from '$lib/translations';
	let {
		source = null,
		existingSources = [],
		onSubmit,
	}: {
		source?: SafeIngestionSource | null;
		/** Existing root ingestion sources for the merge dropdown (create mode only) */
		existingSources?: SafeIngestionSource[];
		onSubmit: (data: CreateIngestionSourceDto) => Promise<void>;
	} = $props();

	// This fork only imports static mailbox files, so there is no live-connection or
	// user-facing "provider" concept. The form takes a single local path and the
	// backend provider (mbox_import / eml_import) is derived from its extension.

	/** Only show root sources (not children) in the merge dropdown */
	const mergeableRootSources = $derived(existingSources.filter((s) => !s.mergedIntoId));

	let formData: CreateIngestionSourceDto = $state({
		name: source?.name ?? '',
		provider: source?.provider ?? 'mbox_import',
		providerConfig: {
			type: source?.provider ?? 'mbox_import',
		},
	});

	let isSubmitting = $state(false);
	let mergeEnabled = $state(false);

	/** When merge is toggled off, clear the mergedIntoId */
	$effect(() => {
		if (!mergeEnabled) {
			delete formData.mergedIntoId;
		}
	});

	// One import method: a local path (picked natively or typed). The backend
	// detects the format from the path — a directory is scanned recursively for
	// .mbox files and Apple Mail .mbox packages (.emlx messages), a .mbox file
	// imports directly, and a .zip is treated as a zip of .eml files.
	$effect(() => {
		if (source) return;
		const isZipPath = (formData.providerConfig.localFilePath ?? '')
			.toLowerCase()
			.endsWith('.zip');
		const provider = isZipPath ? 'eml_import' : 'mbox_import';
		formData.provider = provider;
		formData.providerConfig.type = provider;
	});

	/** The last name this form filled in automatically — user edits win. */
	let lastAutoName = $state('');

	/** "…/Mail Exports/Inbox.mbox" → "Inbox"; empty for unusable paths. */
	function nameFromPath(path: string): string {
		const base = path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? '';
		return base.replace(/\.(mbox|zip|eml)$/i, '').trim();
	}

	/** Fills the Name field from the chosen path unless the user typed one. */
	function autoFillName(path: string) {
		const suggestion = nameFromPath(path);
		if (!suggestion) return;
		if (!formData.name.trim() || formData.name === lastAutoName) {
			formData.name = suggestion;
			lastAutoName = suggestion;
		}
	}

	function timestampName(): string {
		const now = new Date();
		const pad = (n: number) => String(n).padStart(2, '0');
		return `Import ${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`;
	}

	const handleSubmit = async (event: Event) => {
		event.preventDefault();
		// Never submit a blank name: fall back to the path's basename, then a
		// dated name that naturally stays unique across repeated imports.
		if (!source && !formData.name.trim()) {
			formData.name =
				nameFromPath(formData.providerConfig.localFilePath ?? '') || timestampName();
		}
		isSubmitting = true;
		try {
			// Edit mode only renames the import. Send just the name so we don't overwrite
			// the stored provider config (file paths) with an empty stub — and so
			// editing never re-runs or alters the already-imported emails.
			const payload = source
				? ({ name: formData.name } as CreateIngestionSourceDto)
				: formData;
			await onSubmit(payload);
		} finally {
			isSubmitting = false;
		}
	};

	/** Opens the OS file/folder picker (served by the desktop shell) and puts
	 *  the chosen path into the path field. */
	async function pickNative(mode: 'file' | 'folder') {
		try {
			const response = await api(`/native/pick-${mode}`, { method: 'POST' });
			if (!response.ok) {
				throw new Error($t('app.components.import_source_form.picker_unavailable'));
			}
			const result = (await response.json()) as { path: string | null };
			if (result.path) {
				formData.providerConfig.localFilePath = result.path;
				autoFillName(result.path);
			}
		} catch (error) {
			setAlert({
				type: 'error',
				title: $t('app.components.import_source_form.picker_unavailable'),
				message: error instanceof Error ? error.message : String(error),
				duration: 5000,
				show: true,
			});
		}
	}

	const mergeTriggerContent = $derived(
		formData.mergedIntoId
			? (mergeableRootSources.find((s) => s.id === formData.mergedIntoId)?.name ??
					$t('app.components.import_source_form.merge_into_select'))
			: $t('app.components.import_source_form.merge_into_select')
	);
</script>

<form onsubmit={handleSubmit} class="grid gap-4">
	<div class="grid grid-cols-4 items-center gap-4">
		<Label for="name" class="text-left">{$t('app.imports.name')}</Label>
		<Input id="name" bind:value={formData.name} class="col-span-3" />
	</div>
	<!-- Import method and file inputs are create-only. Editing an import just renames
	     it; the emails are already imported and are never changed here. -->
	{#if !source}
		<div class="grid grid-cols-4 items-start gap-4">
			<Label for="import-path" class="pt-2 text-left"
				>{$t('app.components.import_source_form.import_path')}</Label
			>
			<div class="col-span-3 space-y-2">
				<Input
					id="import-path"
					bind:value={formData.providerConfig.localFilePath}
					placeholder="/home/you/Mail Exports"
				/>
				<div class="flex gap-2">
					<Button
						type="button"
						variant="outline"
						size="sm"
						class="gap-2 text-xs"
						onclick={() => pickNative('file')}
					>
						<FileIcon class="h-3.5 w-3.5" />
						{$t('app.components.import_source_form.choose_file')}
					</Button>
					<Button
						type="button"
						variant="outline"
						size="sm"
						class="gap-2 text-xs"
						onclick={() => pickNative('folder')}
					>
						<FolderOpen class="h-3.5 w-3.5" />
						{$t('app.components.import_source_form.choose_folder')}
					</Button>
				</div>
				<p class="text-muted-foreground text-xs">
					{$t('app.components.import_source_form.import_path_help')}
				</p>
			</div>
		</div>
	{/if}
	<!-- Merge into existing import — shown only when a merge target exists -->
	{#if !source && mergeableRootSources.length > 0}
		<div class="mt-2 grid gap-4 border-t pt-4">
			<div class="grid grid-cols-4 items-center gap-4">
				<div class="flex items-center gap-1 text-left">
					<Label for="mergeEnabled"
						>{$t('app.components.import_source_form.merge_into')}</Label
					>
					<span
						use:tooltip={$t('app.components.import_source_form.merge_into_tooltip')}
						class="text-muted-foreground cursor-help"
					>
						<Info class="h-4 w-4" />
					</span>
				</div>
				<Checkbox id="mergeEnabled" bind:checked={mergeEnabled} />
			</div>

			{#if mergeEnabled}
				<div class="grid grid-cols-4 items-center gap-4">
					<div class="col-span-1"></div>
					<div class="col-span-3">
						<Select.Root
							name="mergedIntoId"
							bind:value={formData.mergedIntoId}
							type="single"
						>
							<Select.Trigger class="w-full">
								{mergeTriggerContent}
							</Select.Trigger>
							<Select.Content>
								{#each mergeableRootSources as rootSource}
									<Select.Item value={rootSource.id}>
										{rootSource.name} ({rootSource.provider
											.split('_')
											.join(' ')})
									</Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>
					</div>
				</div>
			{/if}
		</div>
	{/if}

	<Dialog.Footer>
		<Button type="submit" disabled={isSubmitting}>
			{#if isSubmitting}
				{$t('app.components.common.submitting')}
			{:else}
				{$t('app.components.common.submit')}
			{/if}
		</Button>
	</Dialog.Footer>
</form>
