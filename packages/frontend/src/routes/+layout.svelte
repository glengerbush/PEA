<script lang="ts">
	import '../app.css';
	import { theme } from '$lib/stores/theme.store';
	import { browser } from '$app/environment';
	import Footer from '$lib/components/custom/Footer.svelte';
	import Alerts from '$lib/components/custom/alert/Alerts.svelte';
	import { setDateTimePrefs } from '$lib/stores/datetime.svelte';
	import SwipeBackTransition from '$lib/components/custom/SwipeBackTransition.svelte';
	import { swipeBackGesture as swipe } from '$lib/stores/swipe-back.svelte';
	import {
		captureSwipeBackSnapshot,
		invalidateSwipeBackSnapshots,
	} from '$lib/swipe-back-snapshot';
	import { afterNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import { SwipeBackSnapshotScheduler } from '$lib/swipe-back-snapshot-scheduler';

	let { data, children } = $props();
	let swipeSurface: HTMLElement;
	const snapshotScheduler = new SwipeBackSnapshotScheduler();

	function currentUrl(): string {
		return page.url.pathname + page.url.search;
	}

	function canBecomeSwipeDestination(pathname: string): boolean {
		return (
			pathname === '/mailbox' ||
			pathname === '/dashboard' ||
			pathname === '/dashboard/duplicates' ||
			pathname === '/dashboard/imports' ||
			pathname === '/dashboard/remote-content-issues'
		);
	}

	function captureCurrentSnapshot(): void {
		if (!swipeSurface || swipe.active || !canBecomeSwipeDestination(page.url.pathname)) return;
		void captureSwipeBackSnapshot(swipeSurface, currentUrl()).catch(() => undefined);
	}

	function scheduleSnapshot(delay?: number): void {
		if (!browser || !swipeSurface || !canBecomeSwipeDestination(page.url.pathname)) {
			snapshotScheduler.cancel();
			return;
		}
		snapshotScheduler.schedule(captureCurrentSnapshot, delay);
	}

	function handleResize(): void {
		invalidateSwipeBackSnapshots();
		scheduleSnapshot();
	}

	afterNavigate(() => scheduleSnapshot());
	onMount(() => {
		const motionPreference = window.matchMedia('(prefers-reduced-motion: reduce)');
		const syncMotionPreference = () => swipe.setReducedMotion(motionPreference.matches);
		syncMotionPreference();
		motionPreference.addEventListener('change', syncMotionPreference);

		const themeObserver = new MutationObserver(() => {
			invalidateSwipeBackSnapshots();
			scheduleSnapshot();
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});
		scheduleSnapshot();

		return () => {
			motionPreference.removeEventListener('change', syncMotionPreference);
			themeObserver.disconnect();
			snapshotScheduler.cancel();
		};
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

<svelte:window
	onwheel={swipe.handleWheel}
	onscroll={() => scheduleSnapshot()}
	onresize={handleResize}
/>
<svelte:body class:overflow-hidden={swipe.active} />

<SwipeBackTransition />
<div
	bind:this={swipeSurface}
	data-swipe-surface
	class="bg-background relative z-10 min-h-screen"
	style:transform={swipe.active ? `translate3d(${swipe.visualDistance}px, 0, 0)` : 'none'}
	style:transition={swipe.transitionDurationMs > 0
		? `transform ${swipe.transitionDurationMs}ms cubic-bezier(0.22, 0.8, 0.2, 1)`
		: 'none'}
	style:box-shadow={swipe.active ? '-18px 0 42px rgb(0 0 0 / 0.24)' : 'none'}
>
	<Alerts />
	<div class="flex min-h-screen flex-col">
		<main class="flex-1">
			{@render children()}
		</main>
		<Footer currentVersion={data.currentVersion} />
	</div>
</div>
