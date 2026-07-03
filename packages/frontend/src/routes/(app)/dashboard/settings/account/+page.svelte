<script lang="ts">
	import { t } from '$lib/translations';
	import { invalidateAll } from '$app/navigation';
	import { Button } from '$lib/components/ui/button';
	import * as Card from '$lib/components/ui/card';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import * as Dialog from '$lib/components/ui/dialog';
	import { setAlert } from '$lib/components/custom/alert/alert-state.svelte';
	import { api } from '$lib/api.client';

	let { data } = $props();
	let user = $derived(data.user);

	let isProfileDialogOpen = $state(false);
	let isSubmitting = $state(false);

	// Profile form state
	let profileFirstName = $state('');
	let profileLastName = $state('');
	let profileEmail = $state('');

	function openProfileDialog() {
		profileFirstName = user?.first_name || '';
		profileLastName = user?.last_name || '';
		profileEmail = user?.email || '';
		isProfileDialogOpen = true;
	}

	async function updateProfile(event: SubmitEvent) {
		event.preventDefault();
		isSubmitting = true;
		try {
			const response = await api('/users/profile', {
				method: 'PATCH',
				body: JSON.stringify({
					first_name: profileFirstName,
					last_name: profileLastName,
					email: profileEmail,
				}),
			});
			if (response.ok) {
				isProfileDialogOpen = false;
				setAlert({
					type: 'success',
					title: $t('app.account.operation_successful'),
					message: $t('app.account.operation_successful'),
					duration: 3000,
					show: true,
				});
				await invalidateAll();
			} else {
				const body = await response.json().catch(() => ({}) as { message?: string });
				setAlert({
					type: 'error',
					title: $t('app.search.error'),
					message: body.message || 'Failed to update profile',
					duration: 3000,
					show: true,
				});
			}
		} finally {
			isSubmitting = false;
		}
	}
</script>

<svelte:head>
	<title>{$t('app.account.title')} - PEA</title>
</svelte:head>

<div class="space-y-6">
	<div>
		<h1 class="text-2xl font-bold">{$t('app.account.title')}</h1>
		<p class="text-muted-foreground">{$t('app.account.description')}</p>
	</div>

	<!-- Personal Information -->
	<Card.Root>
		<Card.Header>
			<Card.Title>{$t('app.account.personal_info')}</Card.Title>
		</Card.Header>
		<Card.Content class="space-y-4">
			<div class="grid grid-cols-2 gap-4">
				<div>
					<Label class="text-muted-foreground">{$t('app.users.name')}</Label>
					<p class="text-sm font-medium">{user?.first_name} {user?.last_name}</p>
				</div>
				<div>
					<Label class="text-muted-foreground">{$t('app.users.email')}</Label>
					<p class="text-sm font-medium">{user?.email}</p>
				</div>
			</div>
		</Card.Content>
		<Card.Footer>
			<Button variant="outline" onclick={openProfileDialog}
				>{$t('app.account.edit_profile')}</Button
			>
		</Card.Footer>
	</Card.Root>
</div>

<!-- Profile Edit Dialog -->
<Dialog.Root bind:open={isProfileDialogOpen}>
	<Dialog.Content class="sm:max-w-[425px]">
		<Dialog.Header>
			<Dialog.Title>{$t('app.account.edit_profile')}</Dialog.Title>
			<Dialog.Description>{$t('app.account.edit_profile_desc')}</Dialog.Description>
		</Dialog.Header>
		<form onsubmit={updateProfile} class="grid gap-4 py-4">
			<div class="grid grid-cols-4 items-center gap-4">
				<Label for="first_name" class="text-right">{$t('app.setup.first_name')}</Label>
				<Input
					id="first_name"
					name="first_name"
					bind:value={profileFirstName}
					class="col-span-3"
				/>
			</div>
			<div class="grid grid-cols-4 items-center gap-4">
				<Label for="last_name" class="text-right">{$t('app.setup.last_name')}</Label>
				<Input
					id="last_name"
					name="last_name"
					bind:value={profileLastName}
					class="col-span-3"
				/>
			</div>
			<div class="grid grid-cols-4 items-center gap-4">
				<Label for="email" class="text-right">{$t('app.users.email')}</Label>
				<Input
					id="email"
					name="email"
					type="email"
					bind:value={profileEmail}
					class="col-span-3"
				/>
			</div>
			<Dialog.Footer>
				<Button type="submit" disabled={isSubmitting}>
					{#if isSubmitting}
						{$t('app.components.common.submitting')}
					{:else}
						{$t('app.components.common.save')}
					{/if}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
