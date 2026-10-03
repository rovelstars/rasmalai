<script lang="ts">
	import { onMount } from 'svelte';
	import { STAGES } from './stages';
	import Artifact from './Artifact.svelte';

	let active = $state(0);
	let motionOK = $state(true);
	let root = $state<HTMLElement | null>(null);
	let stageEls: HTMLElement[] = [];

		function pickClosest() {
			const maxScroll = document.body.scrollHeight - window.innerHeight;
			if (maxScroll > 100 && window.scrollY >= maxScroll - 2) {
				active = stageEls.length - 1;
				return;
			}
		const mid = window.innerHeight / 2;
		let best = -1;
		let bestD = Infinity;
		for (const s of stageEls) {
			const r = s.getBoundingClientRect();
			if (r.top > mid || r.bottom < mid) continue;
			const d = Math.abs(r.top + r.height / 2 - mid);
			if (d < bestD) {
				bestD = d;
				best = Number(s.dataset.stage);
			}
		}
		if (best >= 0) active = best;
	}

	onMount(() => {
		motionOK = !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		stageEls = root ? Array.from(root.querySelectorAll<HTMLElement>('[data-stage]')) : [];
		let atEnd = false;
		const io = new IntersectionObserver(() => {
			if (!atEnd) pickClosest();
		}, {
			threshold: [0, 0.25, 0.5, 0.75, 1]
		});
		stageEls.forEach((s) => io.observe(s));
		const end = root?.querySelector<HTMLElement>('[data-journey-end]');
		const endIo = new IntersectionObserver(
			(entries) => {
				atEnd = entries.some((e) => e.isIntersecting);
				if (atEnd) {
					active = stageEls.length - 1;
				} else {
					pickClosest();
				}
			},
			{ rootMargin: '0px' }
		);
		if (end) endIo.observe(end);
		pickClosest();
		return () => {
			io.disconnect();
			endIo.disconnect();
		};
	});

	const showAct = $derived(
		STAGES.map((s, i) => (i === 0 ? s.act : s.act !== STAGES[i - 1].act ? s.act : null))
	);
	const focused = $derived(STAGES.map((_, i) => !motionOK || active === i));
</script>

<div bind:this={root} class="relative">
	<div
		class="absolute top-0 bottom-0 left-[7px] hidden w-px bg-aura-surfaceElevated md:block"
		aria-hidden="true"
	>
		<div
			class="w-px bg-aura-purple transition-all duration-500"
			style="height:{((active + 1) / STAGES.length) * 100}%"
		></div>
	</div>

	<div class="space-y-14 md:space-y-20">
		{#each STAGES as s, i}
			{#if showAct[i]}
				<p class="font-mono text-[11px] tracking-[0.2em] text-aura-purple md:pl-8">{showAct[i]}</p>
			{/if}
			<section
				data-stage={i}
				aria-label={s.title}
				class="grid items-start gap-5 md:grid-cols-2 md:gap-8 md:pl-8 {i % 2 === 1
					? 'md:[&>*:first-child]:order-2'
					: ''}"
			>
				<div
					class="min-w-0 transition-all duration-500 {active === i || !motionOK
						? 'translate-y-0 opacity-100 blur-0'
						: 'translate-y-3 opacity-60 blur-[1.5px]'}"
				>
					<Artifact def={s.artifact} active={active === i} {motionOK} />
				</div>
				<div class="min-w-0 {i % 2 === 1 ? 'md:order-1' : ''}">
					<div
						class="transition-all duration-500 {focused[i]
							? 'translate-y-0 opacity-100 blur-0'
							: 'translate-y-1 opacity-60 blur-[1.5px]'}"
					>
						<p
							class="font-mono text-[11px] transition-colors duration-500 {focused[i]
								? 'text-aura-purple'
								: 'text-aura-muted/50'}"
						>
							{s.kicker}
						</p>
						<h3
							class="mt-1 text-xl font-bold transition-colors duration-500 md:text-2xl {focused[i]
								? 'text-aura-text'
								: 'text-aura-muted/60'}"
						>
							{s.title}
						</h3>
						<p
							class="mt-2 max-w-[52ch] text-[15px] leading-relaxed transition-colors duration-500 {focused[i]
								? 'text-aura-muted'
								: 'text-aura-muted/50'}"
						>
							{s.body}
						</p>
					</div>
				</div>
			</section>
		{/each}
	</div>

	<div data-journey-end class="h-px" aria-hidden="true"></div>

</div>

<style>
	@media (prefers-reduced-motion: reduce) {
		div {
			transition: none !important;
		}
	}
</style>
