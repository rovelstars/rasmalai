<script lang="ts">
	import { cachedRnxSpans, highlightRnxSpans } from '$lib/aura/rnx-treesitter';
	import { rnxSpansToHtml, type AuraSpan } from '$lib/aura/rnx-captures';

	let { source, showCopy = false, links }: { source: string; showCopy?: boolean; links?: Map<string, string> } = $props();

	let copied = $state(false);

	async function copy() {
		try {
			await navigator.clipboard.writeText(source);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			copied = false;
		}
	}

	// svelte-ignore state_referenced_locally
	let spans = $state<AuraSpan[]>(cachedRnxSpans(source, links));

	$effect(() => {
		const src = source;
		const map = links;
		let cancelled = false;
		highlightRnxSpans(src, map).then((next) => {
			if (!cancelled) spans = next;
		});
		return () => {
			cancelled = true;
		};
	});
</script>

<div class="relative">
	{#if showCopy}
		<button
			onclick={copy}
			class="doc-copy absolute right-2 top-2"
			aria-label="Copy code"
		>
			{copied ? 'copied' : 'copy'}
		</button>
	{/if}
	<pre
		class="overflow-x-auto p-4 font-mono text-[13px] leading-relaxed"><code>{@html rnxSpansToHtml(spans)}</code></pre>
</div>
