export interface RevealOptions {
	delay?: number;
	y?: number;
}

export function reveal(node: HTMLElement, opts: RevealOptions = {}) {
	node.classList.add('rv');
	if (opts.delay) node.style.setProperty('--rv-delay', `${opts.delay}ms`);
	if (opts.y) node.style.setProperty('--rv-y', `${opts.y}px`);
	if (typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches) {
		node.classList.add('is-in');
		return {};
	}
	const io = new IntersectionObserver(
		(entries) => {
			for (const e of entries) {
				if (e.isIntersecting) {
					node.classList.add('is-in');
					io.disconnect();
				}
			}
		},
		{ threshold: 0.12, rootMargin: '0px 0px -6% 0px' }
	);
	io.observe(node);
	return {
		destroy() {
			io.disconnect();
		}
	};
}
