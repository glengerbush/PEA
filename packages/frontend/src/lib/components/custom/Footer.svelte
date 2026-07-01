<script lang="ts">
	import { t } from '$lib/translations';
	import * as Alert from '$lib/components/ui/alert';
	import { Info } from 'lucide-svelte';

	export let currentVersion: string;
	export let newVersionInfo: { version: string; description: string; url: string } | null = null;
</script>

<footer class="bg-muted py-6 md:py-0">
	<div class="container mx-auto flex flex-col items-center justify-center gap-4 py-8 md:flex-row">
		<div class="flex flex-col items-center gap-2">
			{#if newVersionInfo}
				<Alert.Root>
					<Alert.Title class="flex items-center gap-2">
						<Info class="h-4 w-4" />
						{$t('app.components.footer.new_version_available')}
						<a
							href={newVersionInfo.url}
							target="_blank"
							class=" text-muted-foreground underline"
						>
							{newVersionInfo.description}
						</a>
					</Alert.Title>
				</Alert.Root>
			{/if}
			<p class="text-balance text-center text-xs font-medium leading-loose">
				© {new Date().getFullYear()} Open Archiver — a personal fork
			</p>
			<p class="text-muted-foreground text-balance text-center text-xs leading-loose">
				Based on the upstream
				<a
					href="https://github.com/LogicLabs-OU/OpenArchiver"
					target="_blank"
					rel="noopener noreferrer"
					class="underline hover:text-foreground">Open Archiver</a>, licensed under
				AGPL-3.0.
			</p>
			<p class="text-balance text-center text-xs font-medium leading-loose">
				Version: {currentVersion}
			</p>
		</div>
	</div>
</footer>
