<script lang="ts">
	import { tick } from 'svelte';
	import { engine } from '$lib/playground/engine.svelte';
	import { createShellState, runShellLine, type ShellState } from '$lib/playground/shell';
	import type { IdeProject } from '$lib/playground/project-model';

	let {
		project,
		onExecuted
	}: {
		project: IdeProject;
		onExecuted: () => void;
	} = $props();

	interface Line {
		text: string;
		cls: string;
	}

	let lines = $state<Line[]>([
		{ text: 'rnx shell - try `help`, `ls`, `rnx check`, `rnx run`.', cls: 'text-aura-muted' }
	]);
	let input = $state('');
	let history = $state<string[]>([]);
	let histIx = $state(-1);
	let box: HTMLDivElement | null = $state(null);
	let field: HTMLInputElement | null = $state(null);
	let busy = $state(false);
	let sh: ShellState = createShellState();

	async function scrollDown(): Promise<void> {
		await tick();
		if (box) box.scrollTop = box.scrollHeight;
	}

	async function submit(): Promise<void> {
		const cmd = input;
		input = '';
		histIx = -1;
		if (cmd.trim()) history = [...history, cmd].slice(-100);
		lines.push({ text: `$ ${cmd}`, cls: 'text-aura-purple' });
		if (busy) {
			lines.push({ text: 'shell busy - wait for the running command', cls: 'text-aura-orange' });
			await scrollDown();
			return;
		}
		if (cmd.trim().length === 0) {
			await scrollDown();
			return;
		}
		busy = true;
		try {
			await engine.ensureLoaded();
		} catch (e) {
			lines.push({ text: e instanceof Error ? e.message : 'engine failed to load', cls: 'text-aura-red' });
			busy = false;
			await scrollDown();
			return;
		}
		try {
			const res = await runShellLine(project, sh, cmd, {
				checkProject: (json, entry) => engine.checkProject(json, entry),
				runProject: (json, entry) => engine.runProject(json, entry),
				testProject: (json, entry) => engine.testProject(json, entry),
				formatSource: (src) => engine.formatSource(src)
			});
			for (const l of res.stdout.split('\n')) {
				if (l.length > 0 || res.stdout.endsWith('\n')) {
					if (l.length > 0) lines.push({ text: l, cls: 'text-aura-text' });
				}
			}
			if (res.stderr) {
				for (const l of res.stderr.split('\n')) {
					if (l.length > 0) lines.push({ text: l, cls: 'text-aura-red' });
				}
			}
			if (res.exitCode !== 0) {
				lines.push({ text: `[exit ${res.exitCode}]`, cls: 'text-aura-orange' });
			}
		} catch (e) {
			lines.push({ text: e instanceof Error ? e.message : 'command failed', cls: 'text-aura-red' });
		} finally {
			busy = false;
			onExecuted();
			await scrollDown();
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (e.key === 'Enter') {
			e.preventDefault();
			void submit();
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			if (history.length > 0) {
				histIx = histIx < 0 ? history.length - 1 : Math.max(0, histIx - 1);
				input = history[histIx];
			}
		} else if (e.key === 'ArrowDown') {
			e.preventDefault();
			if (histIx >= 0) {
				histIx = histIx + 1;
				input = histIx >= history.length ? '' : history[histIx];
				if (histIx >= history.length) histIx = -1;
			}
		}
	}

	export function focusShell(): void {
		field?.focus();
	}
</script>

<div class="panel flex min-h-[220px] flex-col overflow-hidden">
	<div class="panel-title">terminal</div>
	<div bind:this={box} class="max-h-64 min-h-32 flex-1 overflow-auto p-3 font-mono text-[12px] leading-relaxed" aria-live="polite">
		{#each lines as l}
			<p class="whitespace-pre-wrap break-words {l.cls}">{l.text}</p>
		{/each}
		{#if busy}
			<p class="animate-pulse text-aura-muted">|</p>
		{/if}
	</div>
	<div class="flex items-center gap-2 border-t border-aura-border px-3 py-2">
		<span class="shrink-0 font-mono text-[12px] text-aura-green">$</span>
		<input
			bind:this={field}
			bind:value={input}
			onkeydown={onKey}
			spellcheck={false}
			autocomplete="off"
			autocapitalize="off"
			placeholder="help, ls, rnx run"
			class="min-w-0 flex-1 bg-transparent font-mono text-[12px] outline-none placeholder:text-aura-muted/60"
			aria-label="Shell input"
		/>
	</div>
</div>
