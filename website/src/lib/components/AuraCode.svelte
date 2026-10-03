<script lang="ts">
	import { highlightAura } from '$lib/aura/highlight';

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

	let spans = $derived(highlightAura(source, links));
</script>

<div class="relative">
	{#if showCopy}
		<button
			onclick={copy}
			class="press absolute right-2 top-2 rounded border border-aura-border px-2 py-1 font-mono text-[11px] text-aura-muted hover:border-aura-borderHover hover:text-aura-text"
			aria-label="Copy code"
		>
			{copied ? 'copied' : 'copy'}
		</button>
	{/if}
	<pre
		class="overflow-x-auto p-4 font-mono text-[13px] leading-relaxed"><code>{#each spans as s}{#if s.href}<a
					href={s.href}
					class="{s.cls} underline decoration-dotted underline-offset-2 hover:text-aura-cyan">{s.text}</a
				>{:else}<span
					class={s.cls}>{s.text}</span
				>{/if}{/each}</code></pre>
</div>
