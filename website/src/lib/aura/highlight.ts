// Mirrors the keyword table in compiler/frontend/src/token.rs. true/false/null are
// lexed as keywords too, but they are painted as literals further down in the regex.
const KEYWORDS = new Set([
	'fn', 'let', 'const', 'static', 'new', 'return', 'defer', 'import', 'pub',
	'public', 'private', 'unsafe', 'comptime', 'native', 'if', 'else', 'for',
	'while', 'in', 'break', 'continue', 'switch', 'case', 'default', 'throw',
	'throws', 'try', 'catch', 'finally', 'guard', 'do', 'fallthrough', 'pass',
	'is', 'from', 'as', 'export', 'async', 'await', 'class', 'struct', 'record',
	'trait', 'interface', 'extension', 'enum', 'init', 'deinit', 'onReload',
	'extends', 'with', 'this', 'super'
]);

// Mirrors is_type_name in compiler/frontend/src/highlight.rs. `Option` is absent:
// no stdlib module declares it and compiler/frontend/tests/prelude.rs asserts it
// must stay removed.
const TYPES = new Set([
	'Int', 'Float', 'FastFloat', 'Bool', 'Void', 'String', 'Any', 'Array',
	'Map', 'Set', 'GenRef', 'Vec4f', 'Vec4i', 'Vec2', 'Result'
]);

export interface AuraSpan {
	text: string;
	cls: string;
	href?: string;
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
		/(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|("(?:[^"\\]|\\.)*"|\b\d[\d_]*(?:\.\d+)?\b|\btrue\b|\bfalse\b|\bnull\b)|([A-Za-z_][A-Za-z0-9_]*)/g;
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
		} else if (KEYWORDS.has(m[3])) {
			spans.push({ text: m[3], cls: 'text-aura-purple' });
		} else if (TYPES.has(m[3])) {
			spans.push({ text: m[3], cls: 'text-aura-pink', href: links?.get(m[3]) });
		} else {
			spans.push({ text: m[3], cls: 'text-aura-text' });
		}
		last = m.index + m[0].length;
	}
	if (last < src.length) {
		spans.push({ text: src.slice(last), cls: 'text-aura-muted' });
	}
	return spans;
}
