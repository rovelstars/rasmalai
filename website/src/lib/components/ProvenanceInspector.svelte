<script lang="ts">
	import { ChevronDown, Lock, LockKeyholeOpen } from 'lucide-svelte';
	import CapabilityBadge from '$lib/components/CapabilityBadge.svelte';
	import {
		nodeLocation,
		type CallChain,
		type PackageCapabilityManifest
	} from '$lib/capabilities';

	let { manifest }: { manifest: PackageCapabilityManifest } = $props();

	let open = $state<boolean[]>([]);

	function toggle(i: number) {
		open[i] = !open[i];
	}

	function intrinsicId(symbol: string): string {
		return symbol.includes('::') ? symbol : `@std/${symbol}`;
	}

	function shortFile(file: string): string {
		const parts = file.split('/');
		return parts.length > 2 ? parts.slice(-2).join('/') : file;
	}
</script>

<div class="overflow-hidden rounded-lg border border-aura-border">
	<div class="flex flex-wrap items-center gap-2 border-b border-aura-border bg-aura-panel px-4 py-3">
		<CapabilityBadge tier={manifest.tier} />
		<span class="font-mono text-sm font-semibold">{manifest.name}</span>
		<span class="font-mono text-xs text-aura-muted">v{manifest.version}</span>
		{#if manifest.summary}
			<span class="w-full text-xs text-aura-muted">{manifest.summary}</span>
		{/if}
	</div>

	{#if manifest.capabilities.length === 0}
		<p class="px-4 py-3 text-sm text-aura-muted">
			No external capabilities detected. 100% verified pure computation.
		</p>
	{:else}
		<div class="flex flex-wrap gap-1.5 px-4 py-3">
			{#each manifest.capabilities as cap}
				<code class="rounded border border-aura-border bg-aura-panel px-1.5 py-0.5 font-mono text-[11px]">{cap}</code>
			{/each}
		</div>

		{#each (manifest.traces ?? []) as trace, i (trace.capability)}
			{@const chain = trace as CallChain}
			{@const last = chain.nodes[chain.nodes.length - 1]}
			<div class="border-t border-aura-border">
				<button
					type="button"
					onclick={() => toggle(i)}
					aria-expanded={!!open[i]}
					aria-controls={`prov-chain-${i}`}
					class="flex w-full items-center gap-2 px-4 py-2 text-left hover:bg-aura-panel"
				>
					<ChevronDown
						size={14}
						aria-hidden="true"
						class={`shrink-0 transition-transform ${open[i] ? '' : '-rotate-90'}`}
					/>
					<code class="font-mono text-xs font-semibold">{chain.capability}</code>
					{#if chain.is_delegated}
						<span
							title="Parameters flow to the sink without mutation"
							class="inline-flex items-center gap-1 rounded-full border border-amber-200 bg-amber-50 px-1.5 py-px font-mono text-[10px] text-amber-700"
						>
							<LockKeyholeOpen size={10} aria-hidden="true" /> delegated
						</span>
					{:else}
						<span
							title="Ambient authority: fixed endpoint, file, or variable"
							class="inline-flex items-center gap-1 rounded-full border border-orange-200 bg-orange-50 px-1.5 py-px font-mono text-[10px] text-orange-700"
						>
							<Lock size={10} aria-hidden="true" /> ambient
						</span>
					{/if}
				</button>

				{#if open[i]}
					<ol id={`prov-chain-${i}`} class="space-y-0 px-4 pb-3">
						{#each chain.nodes as node, j}
							<li class="flex gap-2 font-mono text-xs">
								<div class="flex w-24 shrink-0 flex-col items-center" aria-hidden="true">
									{#if j > 0}
										<span class="text-aura-muted">v</span>
										<span class="text-[10px] text-aura-muted">
											{j === chain.nodes.length - 1 ? 'sink' : 'calls'}
										</span>
									{/if}
								</div>
								<div class={j === chain.nodes.length - 1 ? 'font-bold' : ''}>
									{#if j === chain.nodes.length - 1 && last}
										<span title="Terminal capability sink">
											SINK: {intrinsicId(node.symbol)}
										</span>
									{:else}
										<span>{node.symbol}</span>
									{/if}
									<span class="text-aura-muted">
										({shortFile(node.file)}:{node.line}:{node.col})
									</span>
									{#if node.expression_snippet}
										<div class="mt-0.5 rounded bg-aura-panel px-1.5 py-0.5 font-normal">
											{node.expression_snippet}
										</div>
									{/if}
									<span class="sr-only">{nodeLocation(node)}</span>
								</div>
							</li>
						{/each}
					</ol>
				{/if}
			</div>
		{/each}
	{/if}
</div>
