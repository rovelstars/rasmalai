export interface AuraSpan {
	text: string;
	cls: string;
	href?: string;
}

export interface RnxCapture {
	name: string;
	start: number;
	end: number;
}

const CAPTURE_CLASSES: Record<string, string> = {
	keyword: 'text-aura-purple',
	'keyword.control': 'text-aura-purple',
	'keyword.control.import': 'text-aura-purple',
	'keyword.special': 'text-aura-purple',
	type: 'text-aura-pink',
	'type.builtin': 'text-aura-pink',
	constructor: 'text-aura-pink',
	function: 'text-aura-cyan',
	'function.call': 'text-aura-cyan',
	'function.method': 'text-aura-cyan',
	'function.macro': 'text-aura-cyan',
	property: 'text-aura-cyan',
	attribute: 'text-aura-cyan',
	'variable.builtin': 'text-aura-purple',
	'variable.parameter': 'text-aura-text',
	variable: 'text-aura-text',
	string: 'text-aura-orange',
	'string.special': 'text-aura-orange',
	number: 'text-aura-orange',
	boolean: 'text-aura-orange',
	'constant.builtin': 'text-aura-orange',
	'comment.documentation': 'text-aura-green',
	comment: 'text-aura-muted italic',
	operator: 'text-aura-purple',
	punctuation: 'text-aura-muted',
	'punctuation.bracket': 'text-aura-muted',
	'punctuation.delimiter': 'text-aura-muted',
	'punctuation.special': 'text-aura-muted'
};

export function captureClass(name: string): string {
	let current = name;
	for (;;) {
		const cls = CAPTURE_CLASSES[current];
		if (cls) return cls;
		const dot = current.lastIndexOf('.');
		if (dot < 0) return 'text-aura-text';
		current = current.slice(0, dot);
	}
}

export function spansFromCaptures(
	src: string,
	captures: readonly RnxCapture[],
	links?: Map<string, string>
): AuraSpan[] {
	if (src.length === 0) return [];
	const classes: Array<string | null> = new Array(src.length).fill(null);
	for (const capture of captures) {
		const start = Math.max(0, Math.min(src.length, capture.start));
		const end = Math.max(start, Math.min(src.length, capture.end));
		if (start === end) continue;
		const cls = captureClass(capture.name);
		for (let i = start; i < end; i += 1) classes[i] = cls;
	}
	const spans: AuraSpan[] = [];
	let start = 0;
	while (start < src.length) {
		const cls = classes[start] ?? 'text-aura-text';
		let end = start + 1;
		while (end < src.length && (classes[end] ?? 'text-aura-text') === cls) end += 1;
		const text = src.slice(start, end);
		const href = cls === 'text-aura-pink' ? links?.get(text) : undefined;
		const last = spans[spans.length - 1];
		if (last && last.cls === cls && last.href === href) last.text += text;
		else spans.push(href ? { text, cls, href } : { text, cls });
		start = end;
	}
	return spans;
}

export function escapeRnxHtml(text: string): string {
	return text
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;');
}

export function rnxSpansToHtml(spans: readonly AuraSpan[]): string {
	return spans
		.map((span) =>
			span.href
				? `<a href="${escapeRnxHtml(span.href)}" class="${span.cls} doc-symlink">${escapeRnxHtml(span.text)}</a>`
				: `<span class="${span.cls}">${escapeRnxHtml(span.text)}</span>`
		)
		.join('');
}
