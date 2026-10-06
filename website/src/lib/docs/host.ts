import { browser } from '$app/environment';

// Install snippets are written against the canonical host, but a preview
// or self-hosted deployment lives elsewhere. Rewrite to the current
// origin host: location.host already includes the port when one is present
// (localhost:5173) and omits it for standard domains.
//
// Code blocks are syntax-highlighted, so the URL rarely survives as one
// contiguous string in the HTML - match it with optional tags between
// every character (this also covers data-code attributes, which stay
// contiguous).
export function withHost(html: string): string {
	if (!browser) return html;
	const esc = (c: string) => c.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
	const pattern = new RegExp(
		[...'https://rasmalai.rovelstars.com'].map((c) => `${esc(c)}(?:<[^>]*>)*`).join(''),
		'g'
	);
	return html.replace(pattern, `${window.location.protocol}//${window.location.host}`);
}
