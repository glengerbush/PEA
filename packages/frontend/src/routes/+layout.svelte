<script lang="ts">
	import '../app.css';
	import { authStore } from '$lib/stores/auth.store';
	import { theme } from '$lib/stores/theme.store';
	import { browser } from '$app/environment';
	import Footer from '$lib/components/custom/Footer.svelte';
	import Alerts from '$lib/components/custom/alert/Alerts.svelte';
	import { setDateTimePrefs } from '$lib/stores/datetime.svelte';

	let { data, children } = $props();

	$effect(() => {
		authStore.syncWithServer(data.user, data.accessToken);
	});

	// Apply the configured time zone / clock format to all date formatting helpers.
	$effect(() => {
		setDateTimePrefs(data.systemSettings);
	});

	$effect(() => {
		if (browser) {
			let finalTheme = $theme;

			if (finalTheme === 'system') {
				finalTheme = data.systemSettings?.theme || 'system';
			}

			const isDark =
				finalTheme === 'dark' ||
				(finalTheme === 'system' &&
					window.matchMedia('(prefers-color-scheme: dark)').matches);
			document.documentElement.classList.toggle('dark', isDark);
		}
	});
</script>

<Alerts />
<div class="flex min-h-screen flex-col">
	<main class="flex-1">
		{@render children()}
	</main>
	<Footer currentVersion={data.currentVersion} newVersionInfo={data.newVersionInfo} />
</div>
