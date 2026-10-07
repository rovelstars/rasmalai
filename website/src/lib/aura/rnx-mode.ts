import { StreamLanguage, syntaxHighlighting, HighlightStyle, type StringStream } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { AURA_KEYWORDS, AURA_TYPES } from '$lib/aura/highlight';

interface RnxState {
	inBlockComment: boolean;
	inString: boolean;
}

function startState(): RnxState {
	return { inBlockComment: false, inString: false };
}

function readString(stream: StringStream, state: RnxState): string {
	let escaped = false;
	for (;;) {
		const c = stream.next();
		if (c === undefined || c === '') {
			state.inString = false;
			break;
		}
		if (escaped) {
			escaped = false;
			continue;
		}
		if (c === '\\') {
			escaped = true;
			continue;
		}
		if (c === '"') {
			state.inString = false;
			break;
		}
	}
	return 'string';
}

export const rnxStream = {
	name: 'rnx',
	startState,
	token(stream: StringStream, state: RnxState): string | null {
		if (state.inString) return readString(stream, state);
		if (state.inBlockComment) {
			for (;;) {
				if (stream.match('*/', true)) {
					state.inBlockComment = false;
					return 'comment';
				}
				if (stream.next() === undefined) return 'comment';
			}
		}
		if (stream.eatSpace()) return null;
		if (stream.sol() && stream.match('//!', true)) {
			stream.skipToEnd();
			return 'meta';
		}
		if (stream.match('/**', true)) {
			if (stream.match('/', false)) {
				stream.skipToEnd();
				return 'comment';
			}
			state.inBlockComment = true;
			return 'meta';
		}
		if (stream.match('//', true)) {
			if (stream.match('!', false)) stream.skipToEnd();
			else stream.skipToEnd();
			return stream.current().startsWith('//!') ? 'meta' : 'comment';
		}
		if (stream.match('/*', true)) {
			state.inBlockComment = true;
			return 'comment';
		}
		if (stream.match('"', true)) {
			state.inString = true;
			return readString(stream, state);
		}
		if (stream.match(/0[xX][0-9a-fA-F_]+|\d[\d_]*(?:\.\d[\d_]*)?/, true)) return 'number';
		if (stream.match(/[A-Za-z_][A-Za-z0-9_]*/, true)) {
			const word = stream.current();
			if (AURA_KEYWORDS.has(word)) return 'keyword';
			if (AURA_TYPES.has(word)) return 'typeName';
			return 'variableName';
		}
		stream.next();
		return null;
	},
	languageData: {
		commentTokens: { line: '//', block: { open: '/*', close: '*/' } },
		indentOnInput: /^\s*[}\]]$/
	}
};

export const rnxLanguage = StreamLanguage.define(rnxStream);

export const auraHighlight = HighlightStyle.define([
	{ tag: tags.keyword, color: '#a277ff' },
	{ tag: tags.typeName, color: '#f494ff' },
	{ tag: tags.string, color: '#ffca85' },
	{ tag: tags.number, color: '#ffca85' },
	{ tag: tags.bool, color: '#ffca85' },
	{ tag: tags.comment, color: '#6e6c7e', fontStyle: 'italic' },
	{ tag: tags.meta, color: '#61ffca' },
	{ tag: tags.variableName, color: '#edecee' }
]);

export function rnxHighlight(): ReturnType<typeof syntaxHighlighting> {
	return syntaxHighlighting(auraHighlight);
}

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
