const ALLOWED = new Set([
	'p', 'br', 'b', 'i', 'em', 'strong', 'code', 'pre',
	'ul', 'ol', 'li', 'a', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
	'blockquote', 'hr', 'table', 'tr', 'td', 'th', 'thead', 'tbody', 'tfoot', 'caption', 'img'
]);

const VOID = new Set(['br', 'hr', 'img']);

const DROP_WITH_CONTENT = new Set(['script', 'style', 'iframe', 'object', 'embed', 'noscript']);

function cleanUrl(value: string): string | null {
	const v = value.trim().replace(/[\u0000-\u0020]+/g, '');
	if (/^https?:\/\//i.test(v)) return value.trim();
	if (/^mailto:/i.test(v)) return value.trim();
	if (v.startsWith('#') || v.startsWith('/')) return value.trim();
	return null;
}

function cleanAttrs(tag: string, raw: string): string {
	const out: string[] = [];
	const re = /([a-zA-Z0-9-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'`=<>]+)))?/g;
	let m: RegExpExecArray | null;
	while ((m = re.exec(raw)) !== null) {
		const name = m[1].toLowerCase();
		const value = m[2] ?? m[3] ?? m[4] ?? '';
		if (name.startsWith('on') || name === 'style' || name === 'srcset') continue;
		if (tag === 'a' && name === 'href') {
			const clean = cleanUrl(value);
			if (clean === null) continue;
			out.push(`href="${clean.replace(/"/g, '&quot;')}"`);
		} else if (tag === 'img' && name === 'src') {
			const clean = cleanUrl(value);
			if (clean === null || /^mailto:/i.test(clean)) continue;
			out.push(`src="${clean.replace(/"/g, '&quot;')}"`);
		} else if (tag === 'img' && name === 'alt') {
			out.push(`alt="${value.replace(/"/g, '&quot;')}"`);
		}
	}
	return out.length > 0 ? ' ' + out.join(' ') : '';
}

export function sanitizeGuideHtml(html: string): string {
	let s = html.replace(/<!--[\s\S]*?-->/g, '');
	const out: string[] = [];
	const re = /<\/?([a-zA-Z0-9]+)((?:"[^"]*"|'[^']*'|[^>"'])*)>|([^<]+)|(<)/g;
	let m: RegExpExecArray | null;
	let dropDepth = 0;
	while ((m = re.exec(s)) !== null) {
		if (m[3] !== undefined) {
			if (dropDepth === 0) out.push(m[3]);
			continue;
		}
		if (m[4] !== undefined) {
			if (dropDepth === 0) out.push('<');
			continue;
		}
		const full = m[0];
		const tag = m[1].toLowerCase();
		const closing = full.startsWith('</');
		if (DROP_WITH_CONTENT.has(tag)) {
			if (!closing) dropDepth++;
			else if (dropDepth > 0) dropDepth--;
			continue;
		}
		if (dropDepth > 0) continue;
		if (!ALLOWED.has(tag)) continue;
		if (closing) {
			if (!VOID.has(tag)) out.push(`</${tag}>`);
			continue;
		}
		out.push(`<${tag}${cleanAttrs(tag, m[2])}>`);
	}
	return out.join('');
}
