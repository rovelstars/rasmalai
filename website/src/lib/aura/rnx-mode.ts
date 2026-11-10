import { StreamLanguage, syntaxHighlighting, HighlightStyle, type StringStream } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { AURA_KEYWORDS, AURA_TYPES } from '$lib/aura/highlight';

interface RnxState {
	inBlockComment: boolean;
	inString: boolean;
	attrDepth: number;
	expectFunctionName: boolean;
	prev: string;
}

function startState(): RnxState {
	return { inBlockComment: false, inString: false, attrDepth: 0, expectFunctionName: false, prev: '' };
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
		if (stream.match(/#!?\[/, true)) {
			state.attrDepth = 1;
			state.prev = '';
			return 'meta';
		}
		if (state.attrDepth > 0 && stream.match('[', true)) {
			state.attrDepth += 1;
			return 'punctuation';
		}
		if (state.attrDepth > 0 && stream.match(']', true)) {
			state.attrDepth -= 1;
			return 'punctuation';
		}
		if (stream.match('"', true)) {
			state.inString = true;
			state.prev = '"';
			return readString(stream, state);
		}
		if (stream.match(/0[xX][0-9a-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|\d[\d_]*(?:\.\d[\d_]*)?/, true)) {
			state.prev = '0';
			return 'number';
		}
		if (stream.match(/[A-Za-z_][A-Za-z0-9_]*/, true)) {
			const word = stream.current();
			const prev = state.prev;
			const next = stream.peek();
			const expectName = state.expectFunctionName;
			state.prev = word.charAt(word.length - 1);
			state.expectFunctionName = false;
			if (word === 'true' || word === 'false') return 'bool';
			if (word === 'null') return 'null';
			if (state.attrDepth > 0) return 'attributeName';
			if (AURA_KEYWORDS.has(word)) {
				if (word === 'fn') state.expectFunctionName = true;
				return 'keyword';
			}
			if (next === '!' && !stream.match('!=', false)) return 'macroName';
			if (AURA_TYPES.has(word)) return 'typeName';
			if (expectName) return 'variableName.function';
			if (/^[A-Z]/.test(word)) return 'typeName';
			if (prev === '.') return 'propertyName';
			if (next === '(') return 'variableName.function';
			if (next === ':' && (prev === '(' || prev === ',' || prev === '{')) return 'propertyName';
			return 'variableName';
		}
		if (stream.match(/>>>=|<<=|>>=|\.\.=|\.\.\.|>>>|\?\.|\?\?|=>|==|!=|<=|>=|&&|\|\||<<|>>|\+=|-=|\*=|\/=|%=|&=|\|=|\^=|\+\+|--/, true)) {
			state.prev = stream.current().charAt(stream.current().length - 1);
			return 'operator';
		}
		const ch = stream.next();
		if (ch === undefined) return null;
		state.prev = ch === '.' ? '.' : ch;
		return 'punctuation';
	},
	languageData: {
		commentTokens: { line: '//', block: { open: '/*', close: '*/' } },
		indentOnInput: /^\s*[}\]]$/
	}
};

export const rnxLanguage = StreamLanguage.define(rnxStream);

export const auraHighlight = HighlightStyle.define([
	{ tag: tags.keyword, color: 'rgb(var(--aura-purple))' },
	{ tag: tags.typeName, color: 'rgb(var(--aura-pink))' },
	{ tag: tags.string, color: 'rgb(var(--aura-orange))' },
	{ tag: tags.number, color: 'rgb(var(--aura-orange))' },
	{ tag: tags.bool, color: 'rgb(var(--aura-orange))' },
	{ tag: tags.null, color: 'rgb(var(--aura-orange))' },
	{ tag: tags.comment, color: 'rgb(var(--aura-faint))', fontStyle: 'italic' },
	{ tag: tags.meta, color: 'rgb(var(--aura-green))' },
	{ tag: tags.attributeName, color: 'rgb(var(--aura-cyan))' },
	{ tag: tags.macroName, color: 'rgb(var(--aura-cyan))' },
	{ tag: tags.propertyName, color: 'rgb(var(--aura-cyan))' },
	{ tag: tags.function(tags.variableName), color: 'rgb(var(--aura-cyan))' },
	{ tag: tags.operator, color: 'rgb(var(--aura-purple))' },
	{ tag: tags.variableName, color: 'rgb(var(--aura-text))' }
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
