import { encodeSnippet } from '$lib/playground/share';
import { copyText } from '$lib/docs/clipboard';

// Delegated click handler for documentation code blocks. Code blocks
// rendered by renderMarkdown carry `data-copy` (copy to clipboard) and,
// for runnable Rasmalai snippets, `data-playground` (open in playground).
// Attach to an ancestor article: onclick={onDocCodeClick}.
export function onDocCodeClick(e: MouseEvent) {
	const el = e.target as HTMLElement;
	const play = el.closest('[data-playground]') as HTMLElement | null;
	if (play) {
		const code = play.getAttribute('data-code') ?? '';
		const url = `/playground?code=${encodeSnippet(code)}`;
		window.open(url, '_blank', 'noopener');
		return;
	}
	const btn = el.closest('[data-copy]') as HTMLElement | null;
	if (!btn) return;
	const code = btn.getAttribute('data-code') ?? '';
	copyText(code).then((ok) => {
		if (!ok) return;
		btn.textContent = 'copied';
		setTimeout(() => (btn.textContent = 'copy'), 1500);
	});
}
