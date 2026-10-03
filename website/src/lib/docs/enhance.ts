import { mount, unmount } from 'svelte';
import RunnableCode from '$lib/components/RunnableCode.svelte';

// Progressively enhances prerendered `rnx` code blocks into interactive
// RunnableCode islands (edit + inline run). Static header/copy/playground
// markup keeps working with JS disabled. Idempotent per container.
const mounted = new WeakMap<Element, object>();

export function enhanceDocCode(root: ParentNode) {
	const blocks = root.querySelectorAll('div.doc-code[data-runnable]:not([data-enhanced])');
	blocks.forEach((el) => {
		const code = el.getAttribute('data-code') ?? '';
		const lang = el.getAttribute('data-lang') || 'rnx';
		el.setAttribute('data-enhanced', '1');
		const prev = mounted.get(el);
		if (prev) {
			try {
				unmount(prev);
			} catch {
				/* already gone */
			}
		}
		el.innerHTML = '';
		try {
			const instance = mount(RunnableCode, { target: el, props: { code, lang } });
			mounted.set(el, instance);
		} catch {
			el.removeAttribute('data-enhanced');
		}
	});
}
