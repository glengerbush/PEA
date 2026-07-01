<script lang="ts">
	import * as NavigationMenu from '$lib/components/ui/navigation-menu/index.js';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu/index.js';
	import Button from '$lib/components/ui/button/button.svelte';
	import { Menu } from 'lucide-svelte';
	import { page } from '$app/state';
	import ThemeSwitcher from '$lib/components/custom/ThemeSwitcher.svelte';
	import { t } from '$lib/translations';
	import Badge from '$lib/components/ui/badge/badge.svelte';
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
		{ href: '/dashboard/admin/jobs', label: $t('app.jobs.jobs'), position: 4 },
		{
			label: $t('app.layout.settings'),
			subMenu: [
				{
					href: '/dashboard/settings/system',
					label: $t('app.layout.system'),
				},
				{
					href: '/dashboard/settings/api-keys',
					label: $t('app.layout.api_keys'),
				},
				{
					href: '/dashboard/settings/account',
					label: $t('app.layout.account'),
				},
				{
					href: '/dashboard/settings/contacts',
					label: 'Contacts',
				},
			],
			position: 9,
		},
	];

	const enterpriseNavItems: NavItem[] = [
		{
			href: '/dashboard/imports/journaling',
			label: $t('app.journaling.title'),
			position: 5,
		},
		{
			href: '/dashboard/admin/license',
			label: 'License status',
			position: 7,
		},
	];

	function mergeNavItems(baseItems: NavItem[], enterpriseItems: NavItem[]): NavItem[] {
		const mergedItems = baseItems.map((item) => ({
			...item,
			subMenu: item.subMenu ? [...item.subMenu] : undefined,
		}));

		for (const enterpriseItem of enterpriseItems) {
			const existingItem = mergedItems.find(
				(item) => item.position === enterpriseItem.position
			);

			if (existingItem) {
				if (existingItem.subMenu && enterpriseItem.subMenu) {
					existingItem.subMenu = [...existingItem.subMenu, ...enterpriseItem.subMenu];
				}
			} else {
				mergedItems.push({
					...enterpriseItem,
					subMenu: enterpriseItem.subMenu ? [...enterpriseItem.subMenu] : undefined,
				});
			}
		}

		return mergedItems.sort((a, b) => a.position - b.position);
	}

	const personalModeHiddenHrefs = new Set(['/dashboard/settings/api-keys']);

	function filterPersonalModeNavItems(items: NavItem[]): NavItem[] {
		return items
			.map((item) => ({
				...item,
				subMenu: item.subMenu?.filter(
					(subItem) => !personalModeHiddenHrefs.has(subItem.href)
				),
			}))
			.filter((item) => item.href || (item.subMenu && item.subMenu.length > 0));
	}

	let navItems: NavItem[] = $derived.by(() => {
		const items = data.enterpriseMode
			? mergeNavItems(baseNavItems, enterpriseNavItems)
			: baseNavItems;

		return data.personalMode ? filterPersonalModeNavItems(items) : items;
	});
</script>

<header class="bg-background sticky top-0 z-40 border-b px-4 md:px-0">
	<div class="container mx-auto flex h-16 flex-row items-center justify-between">
		<a href="/mailbox" class="flex flex-row items-center gap-2 font-bold">
			<img src="/logos/logo-sq.svg" alt="OpenArchiver Logo" class="h-8 w-8" />
			<span class="hidden sm:inline-block">Open Archiver</span>
			{#if data.enterpriseMode}
				<Badge class="px-1 py-0.5 text-[8px] font-bold">Enterprise</Badge>
			{/if}
		</a>

		<!-- Desktop Navigation -->
		<div class="hidden lg:flex">
			<NavigationMenu.Root viewport={false}>
				<NavigationMenu.List class="flex items-center space-x-4">
					{#each navItems as item (item.href || item.label)}
						{#if item.subMenu && item.subMenu.length > 0}
							<NavigationMenu.Item
								class={item.subMenu.some((sub) =>
									page.url.pathname.startsWith(
										sub.href.substring(0, sub.href.lastIndexOf('/'))
									)
								)
									? 'bg-accent rounded-md'
									: ''}
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
				</NavigationMenu.List>
			</NavigationMenu.Root>
		</div>

		<div class="flex items-center gap-4">
			<!-- Mobile Navigation -->
			<div class="lg:hidden">
				<DropdownMenu.Root>
					<DropdownMenu.Trigger>
						{#snippet child({ props })}
							<Button {...props} variant="ghost" size="icon">
								<Menu class="h-6 w-6" />
							</Button>
						{/snippet}
					</DropdownMenu.Trigger>
					<DropdownMenu.Content class="w-56" align="end">
						{#each navItems as item (item.href || item.label)}
							{#if item.subMenu && item.subMenu.length > 0}
								<DropdownMenu.Sub>
									<DropdownMenu.SubTrigger>{item.label}</DropdownMenu.SubTrigger>
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
			</div>
			<ThemeSwitcher />
			<!-- LOCAL MODE: auth disabled — logout removed. -->
		</div>
	</div>
</header>

<main class="container mx-auto my-10 px-4 md:px-0">
	{@render children()}
</main>
