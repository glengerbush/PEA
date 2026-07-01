<script lang="ts">
	import type { SafeIngestionSource, CreateIngestionSourceDto } from '@open-archiver/types';
	import { Button } from '$lib/components/ui/button';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import * as Select from '$lib/components/ui/select';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { api } from '$lib/api.client';
	import { Loader2, Info } from 'lucide-svelte';
	import tippy from 'tippy.js';
	import 'tippy.js/dist/tippy.css';
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
	// user-facing "provider" concept. One Import Method selector drives the form and
	// the backend provider (mbox_import / eml_import) is derived from the choice.

	/** Only show root sources (not children) in the merge dropdown */
	const mergeableRootSources = $derived(existingSources.filter((s) => !s.mergedIntoId));

	let formData: CreateIngestionSourceDto = $state({
		name: source?.name ?? '',
		provider: source?.provider ?? 'mbox_import',
		providerConfig: {
			type: source?.provider ?? 'mbox_import',
			secure: true,
			allowInsecureCert: false,
		},
	});

	let isSubmitting = $state(false);
	let fileUploading = $state(false);
	let mergeEnabled = $state(false);

	/** When merge is toggled off, clear the mergedIntoId */
	$effect(() => {
		if (!mergeEnabled) {
			delete formData.mergedIntoId;
		}
	});

	// The single source of truth for the create form. Each option maps to a backend
	// provider + input shape:
	//   mbox-files   → mbox_import, upload flat .mbox file(s)
	//   mbox-folder  → mbox_import, upload an Apple Mail .mbox package (folder)
	//   eml-zip      → eml_import, upload a .zip of .eml files
	//   local        → server path; format detected from the extension (.zip → eml)
	type ImportMethod = 'mbox-files' | 'mbox-folder' | 'eml-zip' | 'local';
	let importMethod = $state<ImportMethod>('mbox-files');

	const importMethodLabel = $derived(
		importMethod === 'mbox-files'
			? $t('app.components.import_source_form.mbox_files')
			: importMethod === 'mbox-folder'
				? $t('app.components.import_source_form.apple_mail_folder')
				: importMethod === 'eml-zip'
					? $t('app.components.import_source_form.provider_eml_import')
					: $t('app.components.import_source_form.local_path')
	);

	// Derive the backend provider from the chosen method (create mode only).
	$effect(() => {
		if (source) return;
		const isZipPath =
			importMethod === 'local' &&
			(formData.providerConfig.localFilePath ?? '').toLowerCase().endsWith('.zip');
		const provider = importMethod === 'eml-zip' || isZipPath ? 'eml_import' : 'mbox_import';
		formData.provider = provider;
		formData.providerConfig.type = provider;
	});

	// Keep only the providerConfig fields that belong to the selected method, so a
	// stale upload path is never submitted after switching. Mbox uploads accumulate
	// into uploadedFiles; eml uses a single uploaded file; local uses a path.
	$effect(() => {
		const cfg = formData.providerConfig;
		const isMbox = importMethod === 'mbox-files' || importMethod === 'mbox-folder';
		if (!isMbox && 'uploadedFiles' in cfg) delete cfg.uploadedFiles;
		if (importMethod !== 'eml-zip') {
			if ('uploadedFilePath' in cfg) delete cfg.uploadedFilePath;
			if ('uploadedFileName' in cfg) delete cfg.uploadedFileName;
		}
		if (importMethod !== 'local' && 'localFilePath' in cfg) delete cfg.localFilePath;
	});

	const handleSubmit = async (event: Event) => {
		event.preventDefault();
		isSubmitting = true;
		try {
			// Edit mode only renames the import. Send just the name so we don't overwrite
			// the stored provider credentials (file paths) with an empty stub — and so
			// editing never re-runs or alters the already-imported emails.
			const payload = source
				? ({ name: formData.name } as CreateIngestionSourceDto)
				: formData;
			await onSubmit(payload);
		} finally {
			isSubmitting = false;
		}
	};

	const handleFileChange = async (event: Event) => {
		const target = event.target as HTMLInputElement;
		const selectedFiles = Array.from(target.files ?? []);
		if (selectedFiles.length === 0) {
			return;
		}
		fileUploading = true;

		try {
			const isMboxImport = importMethod === 'mbox-files' || importMethod === 'mbox-folder';
			const compatibleFiles = isMboxImport
				? selectedFiles.filter(
						(file) =>
							file.name.toLowerCase().endsWith('.mbox') ||
							file.name.toLowerCase().endsWith('.emlx')
					)
				: selectedFiles.slice(0, 1);
			const existingFiles: Array<{
				fileName: string;
				filePath: string;
				relativePath?: string;
			}> =
				isMboxImport && Array.isArray(formData.providerConfig.uploadedFiles)
					? formData.providerConfig.uploadedFiles
					: [];
			const existingFileKeys = new Set(
				existingFiles.map((file) => file.relativePath || file.fileName)
			);
			const filesToUpload = compatibleFiles.filter(
				(file) => !existingFileKeys.has(file.webkitRelativePath || file.name)
			);

			if (filesToUpload.length === 0) {
				throw new Error($t('app.components.import_source_form.no_new_mbox_messages'));
			}

			const uploadedFiles: Array<{
				fileName: string;
				filePath: string;
				relativePath?: string;
			}> = [];

			for (const file of filesToUpload) {
				const uploadFormData = new FormData();
				uploadFormData.append('file', file);
				const response = await api('/upload', {
					method: 'POST',
					body: uploadFormData,
				});

				let result: Record<string, string>;
				try {
					result = await response.json();
				} catch {
					throw new Error(
						$t('app.components.import_source_form.upload_network_error')
					);
				}

				if (!response.ok) {
					throw new Error(
						result.message || $t('app.components.import_source_form.upload_failed')
					);
				}

				uploadedFiles.push({
					fileName: file.name,
					filePath: result.filePath,
					relativePath: file.webkitRelativePath || undefined,
				});
			}

			if (isMboxImport) {
				formData.providerConfig.uploadedFiles = [...existingFiles, ...uploadedFiles];
				delete formData.providerConfig.uploadedFilePath;
				delete formData.providerConfig.uploadedFileName;
				target.value = '';
			} else {
				formData.providerConfig.uploadedFilePath = uploadedFiles[0].filePath;
				formData.providerConfig.uploadedFileName = uploadedFiles[0].fileName;
			}
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			setAlert({
				type: 'error',
				title: $t('app.components.import_source_form.upload_failed'),
				message,
				duration: 5000,
				show: true,
			});
			// Reset file input so the user can retry with the same file
			target.value = '';
		} finally {
			fileUploading = false;
		}
	};

	const mergeTriggerContent = $derived(
		formData.mergedIntoId
			? (mergeableRootSources.find((s) => s.id === formData.mergedIntoId)?.name ??
					$t('app.components.import_source_form.merge_into_select'))
			: $t('app.components.import_source_form.merge_into_select')
	);
</script>

<form onsubmit={handleSubmit} class="grid gap-4 py-4">
	<div class="grid grid-cols-4 items-center gap-4">
		<Label for="name" class="text-left">{$t('app.imports.name')}</Label>
		<Input id="name" bind:value={formData.name} class="col-span-3" />
	</div>
	<!-- Import method and file inputs are create-only. Editing an import just renames
	     it; the emails are already imported and are never changed here. -->
	{#if !source}
		<div class="grid grid-cols-4 items-start gap-4">
			<Label class="pt-2 text-left"
				>{$t('app.components.import_source_form.import_method')}</Label
			>
			<Select.Root name="importMethod" bind:value={importMethod} type="single">
				<Select.Trigger class="col-span-3">
					{importMethodLabel}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value="mbox-files"
						>{$t('app.components.import_source_form.mbox_files')}</Select.Item
					>
					<Select.Item value="mbox-folder"
						>{$t('app.components.import_source_form.apple_mail_folder')}</Select.Item
					>
					<Select.Item value="eml-zip"
						>{$t(
							'app.components.import_source_form.provider_eml_import'
						)}</Select.Item
					>
					<Select.Item value="local"
						>{$t('app.components.import_source_form.local_path')}</Select.Item
					>
				</Select.Content>
			</Select.Root>
		</div>

		{#if importMethod === 'mbox-files'}
			<div class="grid grid-cols-4 items-center gap-4">
				<Label for="mbox-file" class="text-left"
					>{$t('app.components.import_source_form.mbox_files')}</Label
				>
				<div class="col-span-3">
					<Input
						id="mbox-file"
						type="file"
						accept=".mbox"
						multiple
						onchange={handleFileChange}
					/>
				</div>
			</div>
		{:else if importMethod === 'mbox-folder'}
			<div class="grid grid-cols-4 items-start gap-4">
				<Label for="mbox-folder" class="pt-2 text-left"
					>{$t('app.components.import_source_form.apple_mail_folder')}</Label
				>
				<div class="col-span-3 space-y-1">
					<Input
						id="mbox-folder"
						type="file"
						accept=".emlx"
						multiple
						webkitdirectory
						onchange={handleFileChange}
					/>
					<p class="text-muted-foreground text-xs">
						{$t('app.components.import_source_form.apple_mail_folder_help')}
					</p>
				</div>
			</div>
		{:else if importMethod === 'eml-zip'}
			<div class="grid grid-cols-4 items-center gap-4">
				<Label for="eml-file" class="text-left"
					>{$t('app.components.import_source_form.eml_file')}</Label
				>
				<div class="col-span-3 flex flex-row items-center space-x-2">
					<Input id="eml-file" type="file" accept=".zip" onchange={handleFileChange} />
					{#if fileUploading}
						<span class=" text-primary animate-spin"><Loader2 /></span>
					{/if}
				</div>
			</div>
		{:else}
			<div class="grid grid-cols-4 items-start gap-4">
				<Label for="mbox-local-path" class="text-left"
					>{$t('app.components.import_source_form.local_file_path')}</Label
				>
				<div class="col-span-3 space-y-1">
					<Input
						id="mbox-local-path"
						bind:value={formData.providerConfig.localFilePath}
						placeholder="/path/inside-container"
					/>
					<p class="text-muted-foreground text-xs">
						{$t('app.components.import_source_form.local_path_container_help')}
					</p>
				</div>
			</div>
		{/if}

		{#if (importMethod === 'mbox-files' || importMethod === 'mbox-folder') && (fileUploading || formData.providerConfig.uploadedFiles?.length)}
			<div class="grid grid-cols-4 items-center gap-4">
				<div></div>
				<div class="text-muted-foreground col-span-3 flex items-center gap-2 text-sm">
					{#if fileUploading}
						<span class="text-primary animate-spin"><Loader2 /></span>
					{/if}
					{#if formData.providerConfig.uploadedFiles?.length}
						<span>
							{formData.providerConfig.uploadedFiles.length}
							{$t('app.components.import_source_form.mbox_messages_ready')}
						</span>
					{/if}
				</div>
			</div>
		{/if}
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
						use:tippy={{
							allowHTML: true,
							content: $t('app.components.import_source_form.merge_into_tooltip'),
							interactive: true,
							delay: 500,
						}}
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
		<Button type="submit" disabled={isSubmitting || fileUploading}>
			{#if isSubmitting}
				{$t('app.components.common.submitting')}
			{:else}
				{$t('app.components.common.submit')}
			{/if}
		</Button>
	</Dialog.Footer>
</form>
