<script lang="ts">
	import { ansiToHtml } from '$lib/ansi/ansi';

	let { output, command }: { output: string; command: string } = $props();
</script>

<div class="ansi">
	<p class="ansi-cmd"><span class="ansi-prompt">$</span> {command}</p>
	<pre class="ansi-out">{@html ansiToHtml(output.trimEnd())}</pre>
</div>

<style>
	.ansi {
		overflow: hidden;
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		background: var(--color-paper-2);
		font-family: var(--font-mono);
	}

	.ansi-cmd {
		border-bottom: 1px solid var(--color-rule);
		padding: 6px var(--space-md);
		font-size: var(--text-2xs);
		color: var(--color-muted);
	}

	.ansi-prompt {
		color: var(--color-signal);
	}

	.ansi-out {
		overflow-x: auto;
		padding: var(--space-md);
		font-size: var(--text-xs);
		line-height: 1.75;
	}
</style>
