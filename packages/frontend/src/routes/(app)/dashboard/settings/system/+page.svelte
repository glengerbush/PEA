<script lang="ts">
	import type { PageData } from './$types';
	import { Button } from '$lib/components/ui/button';
	import * as Card from '$lib/components/ui/card';
	import * as Label from '$lib/components/ui/label';
	import * as RadioGroup from '$lib/components/ui/radio-group';
	import * as Select from '$lib/components/ui/select';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import type { SupportedLanguage, UpdateCheckResult } from '@open-archiver/types';
	import { t } from '$lib/translations';
	import { enhance } from '$app/forms';

	let { data, form }: { data: PageData; form: any } = $props();
	let settings = $state(data.systemSettings);
	let isSaving = $state(false);
	let isCheckingUpdates = $state(false);
	let updateResult = $state<UpdateCheckResult | null>(null);

	const shortSha = (sha: string) => (sha && sha !== 'unknown' ? sha.slice(0, 7) : 'unknown');

	const languageOptions: { value: SupportedLanguage; label: string }[] = [
		{ value: 'en', label: '🇬🇧 English' },
		{ value: 'de', label: '🇩🇪 Deutsch' },
		{ value: 'fr', label: '🇫🇷 Français' },
		{ value: 'et', label: '🇪🇪 Eesti' },
		{ value: 'es', label: '🇪🇸 Español' },
		{ value: 'it', label: '🇮🇹 Italiano' },
		{ value: 'pt', label: '🇵🇹 Português' },
		{ value: 'nl', label: '🇳🇱 Nederlands' },
		{ value: 'el', label: '🇬🇷 Ελληνικά' },
		{ value: 'bg', label: '🇧🇬 български' },
		{ value: 'ja', label: '🇯🇵 日本語' },
	];

	const languageTriggerContent = $derived(
		languageOptions.find((lang) => lang.value === settings.language)?.label ??
			'Select a language'
	);

	$effect(() => {
		if (form?.success) {
			settings = form.settings;
			setAlert({
				type: 'success',
				title: $t('app.system_settings.saved_title'),
				message: $t('app.system_settings.saved_message'),
				duration: 3000,
				show: true,
			});
		} else if (form?.message) {
			setAlert({
				type: 'error',
				title: $t('app.system_settings.save_failed_title'),
				message: form.message,
				duration: 5000,
				show: true,
			});
		}
	});

	$effect(() => {
		if (form?.update) updateResult = form.update;
	});
</script>

<svelte:head>
	<title>{$t('app.system_settings.title')} - OpenArchiver</title>
</svelte:head>

<div class="space-y-6">
	<div>
		<h1 class="text-2xl font-bold">{$t('app.system_settings.system_settings')}</h1>
		<p class="text-muted-foreground">{$t('app.system_settings.description')}</p>
	</div>

	<form method="POST" action="?/save" class="space-y-8" onsubmit={() => (isSaving = true)}>
		<Card.Root>
			<Card.Content class="space-y-4">
				<!-- Hide language setting for now -->
				<div class="grid gap-2">
					<Label.Root class="mb-1" for="language"
						>{$t('app.system_settings.language')}</Label.Root
					>
					<Select.Root name="language" bind:value={settings.language} type="single">
						<Select.Trigger class="w-[280px]">
							{languageTriggerContent}
						</Select.Trigger>
						<Select.Content>
							{#each languageOptions as lang}
								<Select.Item value={lang.value}>{lang.label}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>
				</div>

				<div class="grid gap-2">
					<Label.Root class="mb-1">{$t('app.system_settings.default_theme')}</Label.Root>
					<RadioGroup.Root
						bind:value={settings.theme}
						name="theme"
						class="flex items-center gap-4"
					>
						<div class="flex items-center gap-2">
							<RadioGroup.Item value="light" id="light" />
							<Label.Root for="light">{$t('app.system_settings.light')}</Label.Root>
						</div>
						<div class="flex items-center gap-2">
							<RadioGroup.Item value="dark" id="dark" />
							<Label.Root for="dark">{$t('app.system_settings.dark')}</Label.Root>
						</div>
						<div class="flex items-center gap-2">
							<RadioGroup.Item value="system" id="system" />
							<Label.Root for="system">{$t('app.system_settings.system')}</Label.Root>
						</div>
					</RadioGroup.Root>
				</div>
			</Card.Content>
			<Card.Footer class="border-t px-6 py-4">
				<Button type="submit" disabled={isSaving}>
					{#if isSaving}
						{$t('app.system_settings.saving')}...
					{:else}
						{$t('app.system_settings.save_changes')}
					{/if}
				</Button>
			</Card.Footer>
		</Card.Root>
	</form>

	<Card.Root>
		<Card.Header>
			<Card.Title>{$t('app.system_settings.updates.title')}</Card.Title>
			<Card.Description>
				{$t('app.system_settings.updates.description')}
			</Card.Description>
		</Card.Header>
		<Card.Content class="space-y-3 text-sm">
			{#if updateResult}
				<div class="flex justify-between">
					<span class="text-muted-foreground">{$t('app.system_settings.updates.current_build')}</span>
					<span class="font-mono">{shortSha(updateResult.currentSha)}</span>
				</div>

				{#if updateResult.status === 'up_to_date'}
					<p class="font-medium text-green-600">
						✓ {$t('app.system_settings.updates.up_to_date')}
					</p>
				{:else if updateResult.status === 'behind'}
					<p class="font-medium text-yellow-600">
						{$t('app.system_settings.updates.available', {
							count: updateResult.behindBy,
						} as any)}
					</p>
					{#if updateResult.commits.length}
						<ul class="text-muted-foreground list-disc space-y-1 pl-5">
							{#each updateResult.commits.slice(0, 10) as commit}
								<li>
									<span class="font-mono">{shortSha(commit.sha)}</span>
									{commit.message}
								</li>
							{/each}
						</ul>
					{/if}
					<div>
						<p class="text-muted-foreground">
							{$t('app.system_settings.updates.run_command')}
						</p>
						<pre
							class="bg-muted mt-1 overflow-x-auto rounded p-2 font-mono text-xs">{updateResult.updateCommand}</pre>
					</div>
					{#if updateResult.compareUrl}
						<a
							href={updateResult.compareUrl}
							target="_blank"
							rel="noopener noreferrer"
							class="text-primary underline"
							>{$t('app.system_settings.updates.view_on_github')}</a
						>
					{/if}
				{:else}
					<p class="text-muted-foreground">
						{updateResult.message ?? $t('app.system_settings.updates.status_unknown')}
					</p>
				{/if}

				<p class="text-muted-foreground text-xs">
					{$t('app.system_settings.updates.last_checked', {
						date: new Date(updateResult.checkedAt).toLocaleString(),
					} as any)}
				</p>
			{:else}
				<p class="text-muted-foreground">{$t('app.system_settings.updates.prompt')}</p>
			{/if}

			{#if form?.updateError}
				<p class="text-destructive">{form.updateError}</p>
			{/if}
		</Card.Content>
		<Card.Footer class="border-t px-6 py-4">
			<form
				method="POST"
				action="?/checkUpdates"
				use:enhance={() => {
					isCheckingUpdates = true;
					return async ({ update: applyResult }) => {
						await applyResult({ reset: false });
						isCheckingUpdates = false;
					};
				}}
			>
				<Button type="submit" variant="outline" disabled={isCheckingUpdates}>
					{isCheckingUpdates
						? $t('app.system_settings.updates.checking')
						: $t('app.system_settings.updates.check')}
				</Button>
			</form>
		</Card.Footer>
	</Card.Root>
</div>
