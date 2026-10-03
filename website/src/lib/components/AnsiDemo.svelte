<script lang="ts">
	import { ansiToHtml } from '$lib/ansi/ansi';

	let { output, command }: { output: string; command: string } = $props();
</script>

<div>
	<p class="border-b border-aura-border px-4 py-1.5 font-mono text-[11px] text-aura-muted">
		<span class="text-aura-green">$</span> {command}
	</p>
	<pre class="overflow-x-auto p-4 font-mono text-[12px] leading-relaxed">{@html ansiToHtml(
			output.trimEnd()
		)}</pre>
</div>
