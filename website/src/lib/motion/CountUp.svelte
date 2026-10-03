<script lang="ts">
	let {
		value,
		decimals = 1,
		prefix = '',
		suffix = '',
		duration = 1100,
		class: cls = ''
	}: {
		value: number;
		decimals?: number;
		prefix?: string;
		suffix?: string;
		duration?: number;
		class?: string;
	} = $props();

	let shown = $state(0);
	let el: HTMLElement | undefined = $state();

	$effect(() => {
		const node = el;
		if (!node) return;
		if (typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches) {
			shown = value;
			return;
		}
		let raf = 0;
		const io = new IntersectionObserver(
			(entries) => {
				if (!entries[0].isIntersecting) return;
				io.disconnect();
				const t0 = performance.now();
				const tick = (t: number) => {
					const p = Math.min(1, (t - t0) / duration);
					const eased = 1 - Math.pow(2, -10 * p);
					shown = value * (p === 1 ? 1 : eased);
					if (p < 1) raf = requestAnimationFrame(tick);
				};
				raf = requestAnimationFrame(tick);
			},
			{ threshold: 0.4 }
		);
		io.observe(node);
		return () => {
			io.disconnect();
			cancelAnimationFrame(raf);
		};
	});
</script>

<span bind:this={el} class="tabular {cls}">{prefix}{shown.toFixed(decimals)}{suffix}</span>
