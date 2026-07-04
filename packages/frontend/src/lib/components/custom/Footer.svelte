<script lang="ts">
	export let currentVersion: string;
</script>

<footer class="bg-muted py-6 md:py-0">
	<div class="container mx-auto flex flex-col items-center justify-center gap-4 py-8 md:flex-row">
		<div class="flex flex-col items-center gap-2">
			<p class="text-balance text-center text-xs font-medium leading-loose">
				© {new Date().getFullYear()} PEA — Personal Email Archive
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
