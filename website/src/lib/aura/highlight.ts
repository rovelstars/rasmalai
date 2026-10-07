// Oracles: keyword() in compiler/frontend/src/token.rs, is_type_name in compiler/frontend/src/highlight.rs.
export const AURA_KEYWORDS = new Set([
	'true', 'false', 'null', 'this', 'super', 'class', 'struct', 'record',
	'trait', 'interface', 'extension', 'enum', 'fn', 'init', 'new', 'deinit',
	'onReload', 'extends', 'with', 'let', 'const', 'static', 'if', 'else',
	'for', 'while', 'in', 'return', 'break', 'continue', 'try', 'catch',
	'finally', 'throw', 'throws', 'defer', 'guard', 'switch', 'case', 'default',
	'do', 'fallthrough', 'is', 'import', 'export', 'from', 'as', 'pub',
	'public', 'private', 'unsafe', 'comptime', 'native', 'async', 'await', 'pass'
]);

export const AURA_TYPES = new Set([
	'Int', 'Float', 'FastFloat', 'Bool', 'Void', 'String', 'Any', 'Array',
	'Map', 'Set', 'GenRef', 'Vec4f', 'Vec4i', 'Vec2', 'Option', 'Result'
]);

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

export function highlightAura(src: string, links?: Map<string, string>): AuraSpan[] {
	const spans: AuraSpan[] = [];
	const re =
		/(\/\*\*(?!\/)[\s\S]*?\*\/|\/\/![^\n]*)|(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|("(?:[^"\\]|\\.)*"|\b\d[\d_]*(?:\.\d+)?\b|\btrue\b|\bfalse\b|\bnull\b)|([A-Za-z_][A-Za-z0-9_]*)/g;
	let last = 0;
	let m: RegExpExecArray | null;
	while ((m = re.exec(src)) !== null) {
		if (m.index > last) {
			spans.push({ text: src.slice(last, m.index), cls: 'text-aura-muted' });
		}
		if (m[1] !== undefined) {
			pushDocSpan(spans, m[1]);
		} else if (m[2] !== undefined) {
			spans.push({ text: m[2], cls: 'text-aura-muted italic' });
		} else if (m[3] !== undefined) {
			spans.push({ text: m[3], cls: 'text-aura-orange' });
		} else if (AURA_KEYWORDS.has(m[4])) {
			spans.push({ text: m[4], cls: 'text-aura-purple' });
		} else if (AURA_TYPES.has(m[4])) {
			spans.push({ text: m[4], cls: 'text-aura-pink', href: links?.get(m[4]) });
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
