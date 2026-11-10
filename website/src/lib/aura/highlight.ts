// Keyword and type tables are generated from the tree-sitter grammar
// (scripts/build-keywords.mjs); see keywords.gen.ts for provenance.
import { RNX_KEYWORDS as AURA_KEYWORDS, RNX_TYPES as AURA_TYPES } from './keywords.gen';

export { AURA_KEYWORDS, AURA_TYPES };

export interface AuraSpan {
	text: string;
	cls: string;
	href?: string;
}

const DOC_CLASS = 'text-aura-green';
const DOC_TAG_CLASS = 'text-aura-cyan';
const DOC_TAG_RE = /@(param|returns?|throws|error|example|see|since|deprecated)\b/g;

function pushDocSpan(spans: AuraSpan[], text: string): void {
	DOC_TAG_RE.lastIndex = 0;
	let last = 0;
	let m: RegExpExecArray | null;
	while ((m = DOC_TAG_RE.exec(text)) !== null) {
		if (m.index > last) {
			spans.push({ text: text.slice(last, m.index), cls: DOC_CLASS });
		}
		spans.push({ text: m[0], cls: DOC_TAG_CLASS });
		last = m.index + m[0].length;
	}
	if (last < text.length) {
		spans.push({ text: text.slice(last), cls: DOC_CLASS });
	}
}

const RUST_KEYWORDS = new Set(
	'fn let mut const return if else for while in loop break continue match struct enum trait impl pub use mod crate self Self true false defer async await move ref dyn where loop'.split(' ')
);
const RUST_TYPES = new Set(
	'String str Vec Option Result i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 bool usize isize char Box Rc Arc Self'.split(' ')
);
const GO_KEYWORDS = new Set(
	'func var const return if else for range switch case default break continue struct interface map type package import defer go select chan true false nil'.split(' ')
);
const GO_TYPES = new Set(
	'string int int8 int16 int32 int64 uint float32 float64 bool byte rune error uintptr any'.split(' ')
);
const CPP_KEYWORDS = new Set(
	'int return if else for while do switch case default break continue struct class enum namespace template typename virtual override static const auto void true false nullptr using new delete this'.split(' ')
);
const CPP_TYPES = new Set(
	'int float double char bool void size_t string vector map auto'.split(' ')
);
const TS_KEYWORDS = new Set(
	'function return if else for while do switch case default break continue const let var import from export default extends class interface type enum async await new this true false null undefined'.split(' ')
);
const TS_TYPES = new Set('string number boolean void any unknown never object Array Record'.split(' '));
const TOML_KEYWORDS = new Set(['true', 'false']);
const SH_KEYWORDS = new Set(
	'if then else fi for while do done case esac function return exit export echo cd mkdir rm'.split(' ')
);
const LUA_KEYWORDS = new Set(
	'function end if then else elseif for while do return local true false nil require'.split(' ')
);

interface LangRules {
	keywords: Set<string>;
	types: Set<string>;
	hashComments?: boolean;
	singleQuoteStrings?: boolean;
}

const LANGS: Record<string, LangRules> = {
	rust: { keywords: RUST_KEYWORDS, types: RUST_TYPES },
	go: { keywords: GO_KEYWORDS, types: GO_TYPES },
	cpp: { keywords: CPP_KEYWORDS, types: CPP_TYPES },
	c: { keywords: CPP_KEYWORDS, types: CPP_TYPES },
	typescript: { keywords: TS_KEYWORDS, types: TS_TYPES, singleQuoteStrings: true },
	ts: { keywords: TS_KEYWORDS, types: TS_TYPES, singleQuoteStrings: true },
	js: { keywords: TS_KEYWORDS, types: TS_TYPES, singleQuoteStrings: true },
	toml: { keywords: TOML_KEYWORDS, types: new Set(), hashComments: true },
	sh: { keywords: SH_KEYWORDS, types: new Set(), hashComments: true },
	lua: { keywords: LUA_KEYWORDS, types: new Set() },
	json: { keywords: new Set(), types: new Set() }
};

export function highlightCode(src: string, lang: string): AuraSpan[] | null {
	const rules = LANGS[lang.toLowerCase()];
	if (!rules) return null;
	const spans: AuraSpan[] = [];
	const comment = rules.hashComments ? '(#[^\\n]*)' : '(\\/\\/[^\\n]*|\\/\\*[\\s\\S]*?\\*\\/)';
	const str = rules.singleQuoteStrings
		? '("(?:[^"\\\\]|\\\\.)*"|\'(?:[^\'\\\\]|\\\\.)*\'|`(?:[^`\\\\]|\\\\.)*`)'
		: '("(?:[^"\\\\]|\\\\.)*")';
	const re = new RegExp(
		`${comment}|${str}|(\\b\\d[\\d_]*(?:\\.\\d+)?\\b|\\btrue\\b|\\bfalse\\b|\\bnull\\b|\\bnil\\b)|([A-Za-z_][A-Za-z0-9_]*)`,
		'g'
	);
	let last = 0;
	let m: RegExpExecArray | null;
	while ((m = re.exec(src)) !== null) {
		if (m.index > last) {
			spans.push({ text: src.slice(last, m.index), cls: 'text-aura-muted' });
		}
		if (m[1] !== undefined) {
			spans.push({ text: m[1], cls: 'text-aura-muted italic' });
		} else if (m[2] !== undefined) {
			spans.push({ text: m[2], cls: 'text-aura-orange' });
		} else if (m[3] !== undefined) {
			spans.push({ text: m[3], cls: 'text-aura-orange' });
		} else if (rules.keywords.has(m[4])) {
			spans.push({ text: m[4], cls: 'text-aura-purple' });
		} else if (rules.types.has(m[4])) {
			spans.push({ text: m[4], cls: 'text-aura-pink' });
		} else {
			spans.push({ text: m[4], cls: 'text-aura-text' });
		}
		last = m.index + m[0].length;
	}
	if (last < src.length) {
		spans.push({ text: src.slice(last), cls: 'text-aura-muted' });
	}
	return spans;
}

const MULTI_CHAR_OPERATORS = [
	'>>>=',
	'<<=',
	'>>=',
	'..=',
	'...',
	'>>>',
	'?.',
	'??',
	'=>',
	'==',
	'!=',
	'<=',
	'>=',
	'&&',
	'||',
	'<<',
	'>>',
	'+=',
	'-=',
	'*=',
	'/=',
	'%=',
	'&=',
	'|=',
	'^=',
	'++',
	'--'
];

const NUMBER_RE = /^(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|(?:\d[\d_]*(?:\.[\d_]+)?|\.[\d_]+)(?:[eE][+-]?[\d_]+)?)/;

function isIdentStart(ch: string): boolean {
	return /[A-Za-z_]/.test(ch);
}

function isIdentPart(ch: string): boolean {
	return /[A-Za-z0-9_]/.test(ch);
}

function isDigit(ch: string): boolean {
	return ch >= '0' && ch <= '9';
}

function addSpan(spans: AuraSpan[], text: string, cls: string, href?: string): void {
	if (!text) return;
	const last = spans[spans.length - 1];
	if (!href && last && last.cls === cls && !last.href) {
		last.text += text;
		return;
	}
	spans.push(href ? { text, cls, href } : { text, cls });
}

function nextNonSpace(src: string, from: number): string {
	for (let i = from; i < src.length; i += 1) {
		if (!/\s/.test(src[i])) return src[i];
	}
	return '';
}

function prevNonSpace(src: string, from: number): number {
	for (let i = from - 1; i >= 0; i -= 1) {
		if (!/\s/.test(src[i])) return i;
	}
	return -1;
}

function isMacroCall(src: string, end: number): boolean {
	const at = nextNonSpace(src, end);
	if (at !== '!') return false;
	let after = end;
	while (after < src.length && /\s/.test(src[after])) after += 1;
	return src[after + 1] !== '=';
}

function isPropertyAccess(src: string, start: number): boolean {
	const dot = prevNonSpace(src, start);
	if (dot < 0 || src[dot] !== '.') return false;
	const before = prevNonSpace(src, dot);
	return before < 0 || src[before] !== '.';
}

function isNamedArgument(src: string, start: number, end: number): boolean {
	const colon = nextNonSpace(src, end);
	if (colon !== ':') return false;
	const prev = prevNonSpace(src, start);
	if (prev < 0) return false;
	return src[prev] === '(' || src[prev] === ',' || src[prev] === '{';
}

function findClosingBrace(src: string, open: number): number {
	let depth = 0;
	for (let i = open; i < src.length; i += 1) {
		const ch = src[i];
		if (ch === '"') {
			i += 1;
			while (i < src.length) {
				if (src[i] === '\\') {
					i += 2;
					continue;
				}
				if (src[i] === '"') break;
				i += 1;
			}
			continue;
		}
		if (ch === '{') depth += 1;
		else if (ch === '}') {
			depth -= 1;
			if (depth === 0) return i;
		}
	}
	return -1;
}

function scanAura(src: string, links: Map<string, string> | undefined, depth: number, inline: boolean): AuraSpan[] {
	const spans: AuraSpan[] = [];
	const plainClass = inline ? 'text-aura-text' : 'text-aura-muted';
	const add = (text: string, cls: string, href?: string) => addSpan(spans, text, cls, href);
	let i = 0;
	let attrDepth = 0;
	let expectFunctionName = false;

	const scanString = (start: number): number => {
		let j = start + 1;
		let chunk = start;
		while (j < src.length) {
			const ch = src[j];
			if (ch === '\\') {
				j += src[j + 1] === 'u' && src[j + 2] === '{' && src.indexOf('}', j + 3) >= 0
					? src.indexOf('}', j + 3) - j + 1
					: 2;
				continue;
			}
			if (ch === '"') {
				add(src.slice(chunk, j + 1), 'text-aura-orange');
				return j + 1;
			}
			const brace = ch === '{' ? j : ch === '$' && src[j + 1] === '{' ? j + 1 : -1;
			if (brace >= 0) {
				const close = findClosingBrace(src, brace);
				if (close > brace) {
					add(src.slice(chunk, brace + 1), 'text-aura-orange');
					if (depth < 3) {
						for (const span of scanAura(src.slice(brace + 1, close), links, depth + 1, inline)) {
							if (span.href) spans.push(span);
							else add(span.text, span.cls);
						}
					} else {
						add(src.slice(brace + 1, close), 'text-aura-text');
					}
					add('}', 'text-aura-orange');
					j = close + 1;
					chunk = j;
					continue;
				}
			}
			j += 1;
		}
		add(src.slice(chunk), 'text-aura-orange');
		return src.length;
	};

	while (i < src.length) {
		const ch = src[i];

		if (src.startsWith('/**', i) && !src.startsWith('/**/', i)) {
			const end = src.indexOf('*/', i + 3);
			if (end >= 0) {
				pushDocSpan(spans, src.slice(i, end + 2));
				i = end + 2;
				continue;
			}
		}
		if (src.startsWith('//!', i)) {
			const end = src.indexOf('\n', i);
			const stop = end < 0 ? src.length : end;
			pushDocSpan(spans, src.slice(i, stop));
			i = stop;
			continue;
		}
		if (src.startsWith('//', i)) {
			const end = src.indexOf('\n', i);
			const stop = end < 0 ? src.length : end;
			add(src.slice(i, stop), 'text-aura-muted italic');
			i = stop;
			continue;
		}
		if (src.startsWith('/*', i)) {
			const end = src.indexOf('*/', i + 2);
			const stop = end < 0 ? src.length : end + 2;
			add(src.slice(i, stop), 'text-aura-muted italic');
			i = stop;
			continue;
		}
		if (ch === '"') {
			i = scanString(i);
			expectFunctionName = false;
			continue;
		}

		if (ch === '#' && attrDepth === 0) {
			let j = i + 1;
			if (src[j] === '!') j += 1;
			if (src[j] === '[') {
				add(src.slice(i, j + 1), 'text-aura-muted');
				attrDepth = 1;
				i = j + 1;
				continue;
			}
		}

		if (attrDepth > 0) {
			if (ch === '[') {
				attrDepth += 1;
				add(ch, 'text-aura-muted');
				i += 1;
				continue;
			}
			if (ch === ']') {
				attrDepth -= 1;
				add(ch, 'text-aura-muted');
				i += 1;
				continue;
			}
		}

		if (isDigit(ch) || (ch === '.' && isDigit(src[i + 1]))) {
			const match = NUMBER_RE.exec(src.slice(i));
			if (match) {
				add(match[0], 'text-aura-orange');
				i += match[0].length;
				expectFunctionName = false;
				continue;
			}
		}

		if (isIdentStart(ch)) {
			let j = i + 1;
			while (j < src.length && isIdentPart(src[j])) j += 1;
			const word = src.slice(i, j);
			let cls = 'text-aura-text';
			let href: string | undefined;
			if (attrDepth > 0) {
				cls = 'text-aura-cyan';
			} else if (word === 'true' || word === 'false' || word === 'null') {
				cls = 'text-aura-orange';
			} else if (AURA_KEYWORDS.has(word)) {
				cls = 'text-aura-purple';
				if (word === 'fn') expectFunctionName = true;
			} else if (isMacroCall(src, j)) {
				cls = 'text-aura-cyan';
			} else if (AURA_TYPES.has(word)) {
				cls = 'text-aura-pink';
				href = links?.get(word);
			} else if (expectFunctionName) {
				cls = 'text-aura-cyan';
			} else if (!inline && /^[A-Z]/.test(word)) {
				cls = 'text-aura-pink';
			} else if (isPropertyAccess(src, i)) {
				cls = 'text-aura-cyan';
			} else if (nextNonSpace(src, j) === '(') {
				cls = 'text-aura-cyan';
			} else if (isNamedArgument(src, i, j)) {
				cls = 'text-aura-cyan';
			}
			if (word !== 'fn') expectFunctionName = false;
			add(word, cls, href);
			i = j;
			continue;
		}

		const op = MULTI_CHAR_OPERATORS.find((candidate) => src.startsWith(candidate, i));
		if (op) {
			add(op, inline ? plainClass : 'text-aura-purple');
			i += op.length;
			expectFunctionName = false;
			continue;
		}

		add(ch, plainClass);
		i += 1;
	}
	return spans;
}

export function highlightAura(src: string, links?: Map<string, string>): AuraSpan[] {
	return scanAura(src, links, 0, false);
}

export function highlightInlineCode(src: string, links?: Map<string, string>): AuraSpan[] {
	return scanAura(src, links, 0, true);
}
