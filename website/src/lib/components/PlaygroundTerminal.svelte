<script lang="ts">
	import { Check, Circle, LoaderCircle, Timer, X } from 'lucide-svelte';
	export type TermStatus = 'idle' | 'checking' | 'running' | 'done' | 'timeout' | 'error';

	let {
		status,
		diagnostics,
		stdout,
		retval,
		duration,
		notice,
		stale
	}: {
		status: TermStatus;
		diagnostics: string;
		stdout: string[];
		retval: string;
		duration: number | null;
		notice: string;
		stale: boolean;
	} = $props();

	const statusLabel: Record<TermStatus, string> = {
		idle: 'idle',
		checking: 'compiling...',
		running: 'running...',
		done: 'done',
		timeout: 'timeout',
		error: 'error'
	};

	function fmtDuration(ms: number): string {
		return ms.toFixed(2).padStart(8) + ' ms';
	}
</script>

<div class="panel flex min-h-[320px] flex-col overflow-hidden">
	<div class="panel-title flex items-center justify-between">
		<span class="inline-flex items-center gap-1.5 {status === 'timeout' || status === 'error' ? 'text-aura-red' : ''} {(status === 'running' || status === 'checking') ? 'animate-pulse' : ''}">
			{#if status === 'done'}<Check size={13} />
			{:else if status === 'timeout'}<Timer size={13} />
			{:else if status === 'error'}<X size={13} />
			{:else if status === 'idle'}<Circle size={13} />
			{:else}<LoaderCircle size={13} class="animate-spin" />{/if}
			{statusLabel[status]}
		</span>
		<span class="flex items-center gap-2">
			{#if stale && (stdout.length > 0 || retval)}
				<span class="rounded border border-aura-orange/40 px-1.5 py-px font-mono text-[10px] text-aura-orange">stale</span>
			{/if}
			{#if duration !== null}
				<span class="tabular text-aura-orange">{fmtDuration(duration)}</span>
			{/if}
		</span>
	</div>
	<div class="flex-1 overflow-auto p-4 font-mono text-[13px] leading-relaxed">
		{#if notice}
			<p class="text-aura-muted">{notice}</p>
		{/if}
		{#if diagnostics}
			<pre class="whitespace-pre-wrap text-aura-red">{diagnostics}</pre>
		{:else}
			<div class={stale && (stdout.length > 0 || retval) ? 'opacity-50' : ''}>
				{#each stdout as line}
					<p class="whitespace-pre-wrap text-aura-text">{line}</p>
				{/each}
				{#if retval}
					<p class="text-aura-green">{retval}</p>
				{/if}
			</div>
		{/if}
		{#if status === 'checking' || status === 'running'}
			<p class="animate-pulse text-aura-muted">|</p>
		{/if}
	</div>
</div>
