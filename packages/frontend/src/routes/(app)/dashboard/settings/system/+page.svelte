<script lang="ts">
	import type { PageData } from './$types';
	import { Button } from '$lib/components/ui/button';
	import * as Card from '$lib/components/ui/card';
	import * as Label from '$lib/components/ui/label';
	import * as RadioGroup from '$lib/components/ui/radio-group';
	import * as Select from '$lib/components/ui/select';
	import { Switch } from '$lib/components/ui/switch';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import type { SupportedLanguage, NativeUpdateInfo } from '@pea/types';
	import { t } from '$lib/translations';
	import { api } from '$lib/api.client';

	let { data }: { data: PageData } = $props();
	let settings = $state(data.systemSettings);
	let isSaving = $state(false);
	let isCheckingUpdates = $state(false);
	let isInstalling = $state(false);
	let installStarted = $state(false);
	let updateInfo = $state<NativeUpdateInfo | null>(null);
	let updateError = $state<string | null>(null);

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

	// Only send the keys this form edits — the backend merges the partial body
	// into current settings (a "full" body would wipe timeZone/clockFormat).
	async function saveSettings(event: SubmitEvent) {
		event.preventDefault();
		isSaving = true;
		try {
			const response = await api('/settings/system', {
				method: 'PUT',
				body: JSON.stringify({ language: settings.language, theme: settings.theme }),
			});
			const body = await response.json();
			if (response.ok) {
				settings = body;
				setAlert({
					type: 'success',
					title: $t('app.system_settings.saved_title'),
					message: $t('app.system_settings.saved_message'),
					duration: 3000,
					show: true,
				});
			} else {
				setAlert({
					type: 'error',
					title: $t('app.system_settings.save_failed_title'),
					message: body.message || 'Failed to update settings',
					duration: 5000,
					show: true,
				});
			}
		} finally {
			isSaving = false;
		}
	}

	// The updates toggle lives outside the main settings form, so it saves on its
	// own. Optimistic: flip immediately, revert if the write fails.
	async function saveAutoCheck(value: boolean) {
		const previous = settings.autoCheckUpdates;
		settings.autoCheckUpdates = value;
		try {
			const response = await api('/settings/system', {
				method: 'PUT',
				body: JSON.stringify({ autoCheckUpdates: value }),
			});
			const body = await response.json();
			if (response.ok) {
				settings = body;
			} else {
				settings.autoCheckUpdates = previous;
				setAlert({
					type: 'error',
					title: $t('app.system_settings.save_failed_title'),
					message: body.message || 'Failed to update settings',
					duration: 5000,
					show: true,
				});
			}
		} catch {
			settings.autoCheckUpdates = previous;
			setAlert({
				type: 'error',
				title: $t('app.system_settings.save_failed_title'),
				message: 'Failed to update settings',
				duration: 5000,
				show: true,
			});
		}
	}

	async function checkUpdates() {
		isCheckingUpdates = true;
		updateError = null;
		installStarted = false;
		updateInfo = null;
		try {
			const response = await api('/native/update-check');
			if (response.ok) {
				updateInfo = await response.json();
			} else {
				// The signed updater lives in the desktop shell; outside the packaged
				// app that native route isn't served.
				updateError = $t('app.system_settings.updates.desktop_only');
			}
		} catch {
			updateError = $t('app.system_settings.updates.desktop_only');
		} finally {
			isCheckingUpdates = false;
		}
	}

	async function installUpdate() {
		isInstalling = true;
		updateError = null;
		try {
			const response = await api('/native/update-install', { method: 'POST' });
			if (response.ok) {
				// The shell downloads + installs in the background, then relaunches.
				installStarted = true;
			} else {
				updateError = $t('app.system_settings.updates.check_failed');
			}
		} catch {
			updateError = $t('app.system_settings.updates.check_failed');
		} finally {
			isInstalling = false;
		}
	}
</script>

<svelte:head>
	<title>{$t('app.system_settings.title')} - PEA</title>
</svelte:head>

<div class="space-y-6">
	<div>
		<h1 class="text-2xl font-bold">{$t('app.system_settings.system_settings')}</h1>
		<p class="text-muted-foreground">{$t('app.system_settings.description')}</p>
	</div>

	<form class="space-y-8" onsubmit={saveSettings}>
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
		<Card.Content class="space-y-4 text-sm">
			<div class="flex items-center justify-between gap-4">
				<div class="space-y-1">
					<Label.Root for="auto-check-updates"
						>{$t('app.system_settings.updates.auto_check')}</Label.Root
					>
					<p class="text-muted-foreground text-xs">
						{$t('app.system_settings.updates.auto_check_description')}
					</p>
				</div>
				<Switch
					id="auto-check-updates"
					checked={settings.autoCheckUpdates}
					onCheckedChange={saveAutoCheck}
				/>
			</div>

			{#if updateInfo}
				<div class="space-y-3 border-t pt-4">
					<div class="flex justify-between">
						<span class="text-muted-foreground"
							>{$t('app.system_settings.updates.current_version')}</span
						>
						<span class="font-mono">{updateInfo.currentVersion}</span>
					</div>

					{#if updateInfo.error}
						<p class="text-muted-foreground">
							{$t('app.system_settings.updates.check_failed')}
						</p>
					{:else if updateInfo.available}
						<p class="font-medium text-yellow-600">
							{$t('app.system_settings.updates.available', {
								version: updateInfo.version ?? '',
							} as any)}
						</p>
						{#if updateInfo.notes}
							<div>
								<p class="text-muted-foreground">
									{$t('app.system_settings.updates.release_notes')}
								</p>
								<pre
									class="bg-muted mt-1 max-h-40 overflow-auto rounded p-2 font-mono text-xs whitespace-pre-wrap">{updateInfo.notes}</pre>
							</div>
						{/if}
						{#if updateInfo.releasesUrl}
							<a
								href={updateInfo.releasesUrl}
								target="_blank"
								rel="noopener noreferrer"
								class="text-primary underline"
								>{$t('app.system_settings.updates.view_releases')}</a
							>
						{/if}
						{#if installStarted}
							<p class="text-muted-foreground">
								{$t('app.system_settings.updates.install_started')}
							</p>
						{:else}
							<div>
								<Button type="button" disabled={isInstalling} onclick={installUpdate}>
									{isInstalling
										? $t('app.system_settings.updates.installing')
										: $t('app.system_settings.updates.install')}
								</Button>
							</div>
						{/if}
					{:else}
						<p class="font-medium text-green-600">
							✓ {$t('app.system_settings.updates.up_to_date')}
						</p>
					{/if}
				</div>
			{/if}

			{#if updateError}
				<p class="text-destructive">{updateError}</p>
			{/if}
		</Card.Content>
		<Card.Footer class="border-t px-6 py-4">
			<Button
				type="button"
				variant="outline"
				disabled={isCheckingUpdates}
				onclick={checkUpdates}
			>
				{isCheckingUpdates
					? $t('app.system_settings.updates.checking')
					: $t('app.system_settings.updates.check')}
			</Button>
		</Card.Footer>
	</Card.Root>
</div>
