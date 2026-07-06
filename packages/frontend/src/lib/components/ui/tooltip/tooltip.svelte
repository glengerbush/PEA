<script lang="ts">
	import { Tooltip as TooltipPrimitive } from 'bits-ui';
	import Provider from './tooltip-provider.svelte';

	let { ...restProps }: TooltipPrimitive.RootProps = $props();
</script>

<Provider>
	<TooltipPrimitive.Root {...restProps} />
</Provider>
