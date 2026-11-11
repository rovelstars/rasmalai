export function computeIndent(prevLine: string, tab: string = '    '): string {
	const base = prevLine.match(/^\s*/)?.[0] ?? '';
	const trimmed = prevLine.trimEnd();
	if (trimmed.endsWith('{') || trimmed.endsWith('(') || trimmed.endsWith('[')) return base + tab;
	return base;
}

export function dedentForClosing(currentIndent: string, tab: string = '    '): string {
	if (currentIndent.endsWith(tab)) return currentIndent.slice(0, -tab.length);
	return currentIndent.replace(/ {1,4}$/, '');
}
