<script lang="ts">
	let { text, class: cls = '' }: { text: string; class?: string } = $props();

	const GLYPHS = '!<>-_\\/[]{}=+*^?#01';
	let out = $state('');
	let el: HTMLElement | undefined = $state();

	$effect(() => {
		const node = el;
		if (!node) return;
		out = text;
		if (typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches) {
			out = text;
			return;
		}
		let frame = 0;
		const io = new IntersectionObserver(
			(entries) => {
				if (!entries[0].isIntersecting) return;
				io.disconnect();
				const total = text.length * 3 + 12;
				const id = setInterval(() => {
					frame++;
					const settled = Math.floor((frame / total) * (text.length + 1));
					let s = '';
					for (let i = 0; i < text.length; i++) {
						if (text[i] === ' ') {
							s += ' ';
							continue;
						}
						s += i < settled ? text[i] : GLYPHS[Math.floor(Math.random() * GLYPHS.length)];
					}
					out = s;
					if (settled > text.length) {
						out = text;
						clearInterval(id);
					}
				}, 34);
			},
			{ threshold: 0.4 }
		);
		io.observe(node);
		return () => io.disconnect();
	});
</script>

<span bind:this={el} class={cls} aria-label={text}>{out === '' ? text : out}</span>
