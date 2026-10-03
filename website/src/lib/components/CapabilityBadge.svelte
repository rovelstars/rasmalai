<script lang="ts">
	import { TriangleAlert } from 'lucide-svelte';
	import { TIER_META, type CapabilityTier } from '$lib/capabilities';

	let { tier, showBlurb = false }: { tier: CapabilityTier; showBlurb?: boolean } = $props();

	const meta = $derived(TIER_META[tier] ?? TIER_META.pure);
	const tooltip = $derived(`${meta.label}: ${meta.blurb}`);
</script>

<span
	role="status"
	aria-label={`Capability tier ${meta.label.toLowerCase()}: ${meta.blurb}`}
	title={tooltip}
	class={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-[11px] font-semibold tracking-wide ${meta.pill}`}
>
	{#if tier === 'hazard'}
		<TriangleAlert size={12} aria-hidden="true" />
	{/if}
	{tier}
	{#if showBlurb}
		<span class="font-sans font-normal normal-case tracking-normal opacity-80">{meta.blurb}</span>
	{/if}
</span>
