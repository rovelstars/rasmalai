<script lang="ts">
	import { onMount } from 'svelte';

	interface Slot {
		name: string;
		type: string;
		from: number;
		to: number;
	}

	const VALUES: Slot[] = [
		{ name: 'sun', type: 'Vec4f', from: 0, to: 100 },
		{ name: 'acc', type: 'Vec4f', from: 3, to: 96 },
		{ name: 'v', type: 'Vec4f', from: 9, to: 88 },
		{ name: 'lit', type: 'Vec4f', from: 22, to: 91 },
		{ name: 'dot', type: 'Float', from: 78, to: 100 }
	];

	let t = $state(0.62);
	let playing = $state(false);
	let frame = 0;
	let started = 0;

	const reduced =
		typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

	function stateOf(v: Slot): 'pending' | 'live' | 'released' {
		if (t * 100 < v.from) return 'pending';
		if (t * 100 <= v.to) return 'live';
		return 'released';
	}

	let live = $derived(VALUES.filter((v) => stateOf(v) === 'live').length);
	let released = $derived(VALUES.filter((v) => stateOf(v) === 'released').length);

	function tick(now: number) {
		if (!started) started = now;
		const elapsed = (now - started) / 2400;
		if (elapsed >= 1) {
			t = 1;
			playing = false;
			return;
		}
		t = elapsed;
		frame = requestAnimationFrame(tick);
	}

	function stop() {
		playing = false;
		cancelAnimationFrame(frame);
	}

	function scrub(e: Event) {
		stop();
		t = Number((e.target as HTMLInputElement).value) / 1000;
	}

	function replay() {
		if (reduced) {
			t = 1;
			return;
		}
		stop();
		t = 0;
		playing = true;
		started = 0;
		frame = requestAnimationFrame(tick);
	}

	onMount(() => () => cancelAnimationFrame(frame));
</script>

<div class="ledger" data-testid="scope-ledger">
	<div class="ledger-head">
		<span class="mono-label">scope · fn shade()</span>
		<span class="tabular time">{(t * 0.42).toFixed(3)} ms</span>
	</div>

	<div class="ledger-body">
		<div class="track" aria-hidden="true">
			<span class="tick" style="left:0%"></span>
			<span class="tick" style="left:50%"></span>
			<span class="tick" style="left:100%"></span>
		</div>

		{#each VALUES as v}
			{@const state = stateOf(v)}
			<div class="row" class:row-live={state === 'live'} class:row-released={state === 'released'}>
				<span class="name">{v.name}</span>
				<span class="type">{v.type}</span>
				<span class="bar-track">
					<span
						class="bar"
						style="left:{v.from}%; width:{Math.max(v.to - v.from, 1)}%"
					></span>
					<span class="free" style="left:{v.to}%"></span>
				</span>
				<span class="state">{state === 'live' ? 'live' : state === 'released' ? 'freed' : ''}</span>
			</div>
		{/each}

		<div class="playhead" style="--t:{t}"></div>
	</div>

	<div class="ledger-foot">
		<div class="counts tabular">
			<span class="text-aura-green">{live} live</span>
			<span class="sep">/</span>
			<span>{released} freed</span>
			<span class="sep">/</span>
			<span>0 retained</span>
		</div>
		<button class="replay" onclick={replay} aria-label="Play the scope timeline once">
			{playing ? 'playing' : 'play'}
		</button>
	</div>

	<label class="scrubber">
		<span class="sr-only">Scope timeline</span>
		<input
			type="range"
			min="0"
			max="1000"
			value={Math.round(t * 1000)}
			oninput={scrub}
			aria-label="Scope timeline"
		/>
	</label>
</div>

<p class="ledger-note">
	Values are released at the closing brace. Nothing is traced, scanned, or collected.
</p>

<style>
	.ledger {
		border: 1px solid var(--color-rule);
		border-radius: var(--radius-sm);
		background:
			linear-gradient(rgb(var(--aura-text) / 0.02), transparent 40%),
			var(--color-paper-2);
	}

	.ledger-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		border-bottom: 1px solid var(--color-rule);
		padding: var(--space-sm) var(--space-md);
	}

	.mono-label {
		font-family: var(--font-mono);
		font-size: var(--text-xs);
		color: var(--color-muted);
	}

	.time {
		font-size: var(--text-xs);
		color: var(--color-signal);
	}

	.ledger-body {
		position: relative;
		padding: var(--space-md) var(--space-md) var(--space-sm);
		--name-w: 52px;
		--type-w: 54px;
		--state-w: 42px;
	}

	.track {
		position: absolute;
		inset: var(--space-md) var(--space-md) var(--space-sm)
			calc(var(--space-md) + var(--name-w) + var(--type-w));
		pointer-events: none;
	}

	.tick {
		position: absolute;
		top: 0;
		bottom: 0;
		width: 1px;
		background: var(--color-rule);
		opacity: 0.55;
	}

	.row {
		position: relative;
		display: grid;
		grid-template-columns: var(--name-w) var(--type-w) minmax(0, 1fr) 42px;
		align-items: center;
		height: 34px;
		gap: 0;
		color: var(--color-faint);
		transition: color var(--dur-micro) ease;
	}

	.row-live {
		color: var(--color-ink);
	}

	.name {
		font-family: var(--font-mono);
		font-size: var(--text-xs);
	}

	.type {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.bar-track {
		position: relative;
		height: 100%;
		margin-inline: 4px;
	}

	.bar {
		position: absolute;
		top: 50%;
		height: 6px;
		transform: translateY(-50%) scaleY(0.3333);
		background: var(--color-rule-strong);
		transition:
			background-color var(--dur-short) ease,
			transform var(--dur-short) ease;
	}

	.row-live .bar {
		transform: translateY(-50%) scaleY(1);
		background: rgb(var(--aura-purple) / 0.55);
	}

	.row-released .bar {
		background: rgb(var(--aura-green) / 0.45);
	}

	.free {
		position: absolute;
		top: 50%;
		width: 1px;
		height: 12px;
		transform: translate(-50%, -50%);
		background: var(--color-rule-strong);
	}

	.row-released .free {
		background: var(--color-signal);
	}

	.state {
		font-family: var(--font-body);
		font-size: var(--text-2xs);
		text-align: right;
		color: var(--color-faint);
	}

	.row-live .state {
		color: var(--color-accent);
	}

	.row-released .state {
		color: var(--color-signal);
	}

	.playhead {
		position: absolute;
		top: var(--space-md);
		bottom: var(--space-sm);
		left: calc(
			var(--space-md) + var(--name-w) + var(--type-w) +
				(100% - 2 * var(--space-md) - var(--name-w) - var(--type-w) - var(--state-w)) *
				var(--t)
		);
		width: 1px;
		background: var(--color-ink);
		opacity: 0.55;
		pointer-events: none;
	}

	.ledger-foot {
		display: flex;
		align-items: center;
		justify-content: space-between;
		border-top: 1px solid var(--color-rule);
		padding: var(--space-xs) var(--space-md);
		font-size: var(--text-2xs);
		color: var(--color-muted);
	}

	.counts {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.sep {
		color: var(--color-rule-strong);
	}

	.replay {
		font-size: var(--text-2xs);
		color: var(--color-faint);
		text-decoration: underline;
		text-underline-offset: 3px;
	}

	.replay:hover {
		color: var(--color-ink);
	}

	.scrubber {
		display: block;
		padding: 0 var(--space-md) var(--space-md);
	}

	.scrubber input {
		width: 100%;
		height: 2px;
		appearance: none;
		background: var(--color-rule);
		outline: none;
		cursor: ew-resize;
	}

	.scrubber input::-webkit-slider-thumb {
		appearance: none;
		width: 9px;
		height: 9px;
		border-radius: 50%;
		background: var(--color-ink);
	}

	.scrubber input::-moz-range-thumb {
		width: 9px;
		height: 9px;
		border: 0;
		border-radius: 50%;
		background: var(--color-ink);
	}

	.ledger-note {
		margin-top: var(--space-sm);
		font-size: var(--text-xs);
		color: var(--color-faint);
	}

	@media (max-width: 480px) {
		.ledger-body {
			--name-w: 44px;
			--type-w: 46px;
			--state-w: 0px;
		}
		.row {
			grid-template-columns: var(--name-w) var(--type-w) minmax(0, 1fr);
		}
		.state {
			display: none;
		}
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
		border: 0;
	}
</style>
