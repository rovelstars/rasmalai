import { copyText } from '$lib/docs/clipboard';

export interface SectionCopy {
	id: string;
	markdown: string;
}

// Per-page copy buttons for documentation markdown. The loaders pass the
// full raw body plus one raw-markdown chunk per `##` subtopic; this module
// injects a copy button beside each matching `<h2>` and serves the
// delegated clicks for both section buttons (`data-copy-section`) and the
// top-level full-page button (`data-copy-page`). Attach to an ancestor
// article alongside onDocCodeClick.
const pageData = new WeakMap<HTMLElement, { raw: string; sections: Map<string, string> }>();

function flash(btn: HTMLElement) {
	const prev = btn.textContent;
	btn.textContent = 'copied';
	setTimeout(() => {
		if (btn.isConnected) btn.textContent = prev;
	}, 1500);
}

export function attachSectionCopy(
	article: HTMLElement,
	raw: string | null | undefined,
	sections: SectionCopy[] | null | undefined
) {
	const byId = new Map((sections ?? []).map((s) => [s.id, s.markdown]));
	pageData.set(article, { raw: raw ?? '', sections: byId });
	for (const h of article.querySelectorAll('h2.doc-h')) {
		const id = (h as HTMLElement).id;
		if (!id || !byId.has(id)) continue;
		if (h.querySelector('[data-copy-section]')) continue;
		const btn = document.createElement('button');
		btn.type = 'button';
		btn.className = 'doc-btn doc-copy-sec';
		btn.setAttribute('data-copy-section', id);
		btn.title = 'Copy section markdown';
		btn.textContent = 'copy';
		h.appendChild(btn);
	}
}

export function onSectionCopyClick(e: MouseEvent) {
	const el = e.target as HTMLElement;
	const article = el.closest('article') as HTMLElement | null;
	if (!article) return;
	const data = pageData.get(article);
	if (!data) return;
	const sec = el.closest('[data-copy-section]') as HTMLElement | null;
	if (sec) {
		const text = data.sections.get(sec.getAttribute('data-copy-section') ?? '');
		if (text === undefined) return;
		copyText(text).then((ok) => {
			if (ok) flash(sec);
		});
		return;
	}
	const page = el.closest('[data-copy-page]') as HTMLElement | null;
	if (page && data.raw) {
		copyText(data.raw).then((ok) => {
			if (ok) flash(page);
		});
	}
}
