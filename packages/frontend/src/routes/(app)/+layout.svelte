<script lang="ts">
	import * as NavigationMenu from '$lib/components/ui/navigation-menu/index.js';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu/index.js';
	import Button from '$lib/components/ui/button/button.svelte';
	import Ellipsis from '@lucide/svelte/icons/ellipsis';
	import Menu from '@lucide/svelte/icons/menu';
	import { page } from '$app/state';
	import ThemeSwitcher from '$lib/components/custom/ThemeSwitcher.svelte';
	import { t } from '$lib/translations';
	import { setContacts } from '$lib/stores/contacts.svelte';
	let { data, children } = $props();

	// Make the imported-contacts map available to name-resolution everywhere.
	$effect(() => {
		setContacts(data.contacts);
	});

	interface NavItem {
		href?: string;
		label: string;
		subMenu?: {
			href: string;
			label: string;
		}[];
		position: number; // represents the position of the item in the navigation menu
	}

	const baseNavItems: NavItem[] = [
		{ href: '/mailbox', label: $t('app.layout.mailbox'), position: 0 },
		{ href: '/dashboard', label: $t('app.layout.dashboard'), position: 1 },
		{ href: '/dashboard/imports', label: $t('app.layout.imports'), position: 2 },
		{ href: '/dashboard/duplicates', label: 'Duplicates', position: 3 },
		{ href: '/trash', label: 'Trash', position: 4 },
		{
			label: $t('app.layout.settings'),
			subMenu: [
				{
					href: '/dashboard/settings/system',
					label: $t('app.layout.system'),
				},
				{
					href: '/dashboard/settings/contacts',
					label: 'Contacts',
				},
				{
					href: '/dashboard/admin/jobs',
					label: $t('app.jobs.jobs'),
				},
			],
			position: 9,
		},
	];

	const navItems: NavItem[] = baseNavItems;

	// --- Priority+ navigation ---------------------------------------------
	// Items stay in the bar until the rightmost one would be cut off; whatever
	// doesn't fit moves into a trailing "more" menu. An invisible copy of the
	// full bar is measured (per-item widths + the more-button width), and the
	// visible prefix is recomputed whenever the available space changes.
	const GAP = 16; // matches the list's space-x-4

	let available = $state(0);
	let widths = $state<number[]>(Array(navItems.length).fill(0));
	let moreWidth = $state(48);

	const observeWidth = (assign: (width: number) => void) => (el: HTMLElement) => {
		const update = () => assign(el.offsetWidth);
		const observer = new ResizeObserver(update);
		observer.observe(el);
		update();
		return () => observer.disconnect();
	};

	const visibleCount = $derived.by(() => {
		// Until everything is measured, keep the full bar (no flash of "more").
		if (available <= 0 || widths.some((w) => w === 0)) return navItems.length;
		const total = widths.reduce((sum, w, i) => sum + w + (i > 0 ? GAP : 0), 0);
		if (total <= available) return navItems.length;
		// Something overflows: reserve room for the more-button, then take the
		// longest prefix of items that still fits in front of it.
		let used = moreWidth;
		let count = 0;
		for (const w of widths) {
			if (used + GAP + w > available) break;
			used += GAP + w;
			count++;
		}
		return count;
	});
	const visibleItems = $derived(navItems.slice(0, visibleCount));
	const overflowItems = $derived(navItems.slice(visibleCount));

	const isItemActive = (item: NavItem) =>
		item.subMenu && item.subMenu.length > 0
			? item.subMenu.some((sub) =>
					page.url.pathname.startsWith(sub.href.substring(0, sub.href.lastIndexOf('/')))
				)
			: page.url.pathname === item.href;
	const overflowActive = $derived(overflowItems.some(isItemActive));
</script>

<header class="bg-background sticky top-0 z-40 border-b px-4 md:px-0">
	<div class="container mx-auto flex h-16 flex-row items-center justify-between">
		<a href="/mailbox" class="flex shrink-0 flex-row items-center gap-2 font-bold">
			<img src="/logos/logo-sq.svg" alt="PEA Logo" class="h-8 w-8" />
			<span class="hidden sm:inline-block">PEA</span>
		</a>

		<!-- Navigation: as many items as fit, the rest behind the more-menu.
		     Spacing must be margin, not padding: the fit check measures this
		     element's offsetWidth, and padding would inflate it — letting the
		     centered list bleed over the brand at certain widths. -->
		<div
			class="mx-4 flex min-w-0 flex-1 justify-center overflow-x-clip"
			{@attach observeWidth((w) => (available = w))}
		>
			<NavigationMenu.Root viewport={false} delayDuration={0}>
				<NavigationMenu.List class="flex items-center space-x-4">
					{#each visibleItems as item (item.href || item.label)}
						{#if item.subMenu && item.subMenu.length > 0}
							<NavigationMenu.Item
								class={isItemActive(item) ? 'bg-accent rounded-md' : ''}
							>
								<NavigationMenu.Trigger class="cursor-pointer font-normal">
									{item.label}
								</NavigationMenu.Trigger>
								<NavigationMenu.Content>
									<ul class="grid w-fit min-w-40 gap-1 p-1">
										{#each item.subMenu as subItem (subItem.href)}
											<li>
												<NavigationMenu.Link href={subItem.href}>
													{subItem.label}
												</NavigationMenu.Link>
											</li>
										{/each}
									</ul>
								</NavigationMenu.Content>
							</NavigationMenu.Item>
						{:else if item.href}
							<NavigationMenu.Item
								class={page.url.pathname === item.href
									? 'bg-accent rounded-md'
									: ''}
							>
								<NavigationMenu.Link href={item.href}>
									{item.label}
								</NavigationMenu.Link>
							</NavigationMenu.Item>
						{/if}
					{/each}

					{#if overflowItems.length > 0}
						<NavigationMenu.Item class={overflowActive ? 'bg-accent rounded-md' : ''}>
							<DropdownMenu.Root>
								<DropdownMenu.Trigger>
									{#snippet child({ props })}
										<Button
											{...props}
											variant="ghost"
											size="icon"
											aria-label="More navigation items"
										>
											{#if visibleItems.length === 0}
												<Menu class="h-6 w-6" />
											{:else}
												<Ellipsis class="h-5 w-5" />
											{/if}
										</Button>
									{/snippet}
								</DropdownMenu.Trigger>
								<DropdownMenu.Content class="w-56" align="end">
									{#each overflowItems as item (item.href || item.label)}
										{#if item.subMenu && item.subMenu.length > 0}
											<DropdownMenu.Sub>
												<DropdownMenu.SubTrigger
													>{item.label}</DropdownMenu.SubTrigger
												>
												<DropdownMenu.SubContent>
													{#each item.subMenu as subItem (subItem.href)}
														<a href={subItem.href}>
															<DropdownMenu.Item
																>{subItem.label}</DropdownMenu.Item
															>
														</a>
													{/each}
												</DropdownMenu.SubContent>
											</DropdownMenu.Sub>
										{:else if item.href}
											<a href={item.href}>
												<DropdownMenu.Item>{item.label}</DropdownMenu.Item>
											</a>
										{/if}
									{/each}
								</DropdownMenu.Content>
							</DropdownMenu.Root>
						</NavigationMenu.Item>
					{/if}
				</NavigationMenu.List>
			</NavigationMenu.Root>
		</div>

		<div class="flex shrink-0 items-center gap-4">
			<ThemeSwitcher />
			<!-- LOCAL MODE: auth disabled — logout removed. -->
		</div>
	</div>
</header>

<!-- Invisible full copy of the bar, only for measuring item widths. -->
<div
	class="pointer-events-none fixed top-0 left-0 -z-50 h-0 overflow-hidden"
	aria-hidden="true"
	inert
>
	<NavigationMenu.Root viewport={false} delayDuration={0}>
		<NavigationMenu.List class="flex items-center space-x-4">
			{#each navItems as item, i (item.href || item.label)}
				<NavigationMenu.Item {@attach observeWidth((w) => (widths[i] = w))}>
					{#if item.subMenu && item.subMenu.length > 0}
						<NavigationMenu.Trigger class="cursor-pointer font-normal">
							{item.label}
						</NavigationMenu.Trigger>
					{:else}
						<NavigationMenu.Link href={item.href}>
							{item.label}
						</NavigationMenu.Link>
					{/if}
				</NavigationMenu.Item>
			{/each}
			<NavigationMenu.Item {@attach observeWidth((w) => (moreWidth = w))}>
				<Button variant="ghost" size="icon" aria-label="">
					<Ellipsis class="h-5 w-5" />
				</Button>
			</NavigationMenu.Item>
		</NavigationMenu.List>
	</NavigationMenu.Root>
</div>

<main class="container mx-auto my-10 px-4 md:px-0">
	{@render children()}
</main>
