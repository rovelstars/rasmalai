import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { detectCapabilityUse, type CapabilityEvidence } from '../src/lib/server/permissions.js';

// The sink scan runs on every published package, so it was rewritten to
// compile its patterns once per isolate and to test a whole group of
// sinks in one pass. The rewrite must not change what it finds: this
// file keeps the old scan next to the new one and compares them over the
// whole standard library and over the shapes that separate them.

type Head = 'fs' | 'net' | 'sys' | 'env' | 'term' | 'unsafe';

const DOTTED_SINKS: Array<{ head: Head; names: string[] }> = [
	{
		head: 'fs',
		names: [
			'File.open', 'File.read', 'File.readText', 'File.readBytes', 'File.lines',
			'File.exists', 'File.seek', 'File.tell', 'File.len', 'File.write',
			'File.writeText', 'File.writeBytes', 'File.append', 'File.create',
			'File.remove', 'File.truncate', 'Path.exists', 'Path.isFile',
			'Path.isDir', 'Path.remove', 'fs.stat', 'fs.exists', 'fs.isFile',
			'fs.isDir', 'fs.readDir', 'fs.glob', 'fs.readLink', 'fs.readText',
			'fs.readBytes', 'fs.mkdir', 'fs.mkdirAll', 'fs.remove', 'fs.removeAll',
			'fs.copy', 'fs.move', 'fs.rename', 'fs.truncate', 'fs.chmod',
			'fs.symlink', 'fs.fsync', 'fs.writeText', 'fs.writeBytes', 'fs.mmap'
		]
	},
	{ head: 'sys', names: ['Process.spawn', 'Process.run'] },
	{
		head: 'env',
		names: ['Env.get', 'Env.has', 'Env.all', 'Env.allEnv', 'env.get', 'env.has', 'env.all']
	},
	{
		head: 'term',
		names: [
			'File.fromHandle', 'io.write', 'io.writeRaw', 'io.writeError', 'io.clear',
			'io.stdout', 'io.stderr', 'io.read', 'io.readLine', 'io.stdin',
			'io.isTTY', 'io.width', 'io.height', 'io.colorProfile', 'io.setRawMode'
		]
	}
];

const BARE_SINKS: Array<{ head: Head; names: string[] }> = [
	{ head: 'net', names: ['fetch', 'Request', 'WebSocket'] },
	{ head: 'term', names: ['print'] }
];

const TERM_SUFFIX_RE = /\.\s*(writeText|writeBytes|readText|readBytes)\s*\(/g;

function stripToCode(source: string): string {
	let s = source.replace(/"""[\s\S]*?"""/g, '""');
	s = s.replace(/"(?:[^"\\\n]|\\.)*"/g, '""');
	s = s.replace(/'(?:[^'\\\n]|\\.)*'/g, "''");
	s = s.replace(/\/\*[\s\S]*?\*\//g, ' ');
	s = s.replace(/\/\/.*$/gm, ' ');
	s = s.replace(/\bclass\s+[A-Za-z_][A-Za-z0-9_]*/g, 'class');
	s = s.replace(/\bfn\s+[A-Za-z_][A-Za-z0-9_]*/g, 'fn');
	return s;
}

function escRe(s: string): string {
	return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// The scan as it was before the rewrite: one pattern per symbol, built
// again for every file.
function legacyDetect(files: Array<{ name: string; text: string }>): CapabilityEvidence[] {
	const seen = new Set<string>();
	const out: CapabilityEvidence[] = [];
	const push = (head: Head, symbol: string, file: string): void => {
		const key = `${head}:${file}`;
		if (seen.has(key)) return;
		seen.add(key);
		out.push({ head, symbol, file });
	};
	for (const f of files) {
		if (!f.name.endsWith('.rnx')) continue;
		const code = stripToCode(f.text);
		for (const group of DOTTED_SINKS) {
			for (const name of group.names) {
				const re = new RegExp(`(?<![A-Za-z0-9_.])${escRe(name)}\\s*\\(`);
				if (re.test(code)) push(group.head, name, f.name);
			}
		}
		for (const group of BARE_SINKS) {
			for (const name of group.names) {
				const re = new RegExp(`(?<![A-Za-z0-9_.])${escRe(name)}\\s*\\(`);
				if (re.test(code)) push(group.head, name, f.name);
			}
		}
		if (TERM_SUFFIX_RE.test(code)) {
			TERM_SUFFIX_RE.lastIndex = 0;
			const m = /\.\s*(writeText|writeBytes|readText|readBytes)\s*\(/.exec(code);
			push('term', m ? m[1] : 'suffix-call', f.name);
		}
		if (/\bfrom\s+native\b/.test(code)) push('unsafe', 'from native', f.name);
		if (/\bunsafe\b/.test(code)) push('unsafe', 'unsafe', f.name);
	}
	out.sort((a, b) => (a.head < b.head ? -1 : a.head > b.head ? 1 : a.file < b.file ? -1 : 1));
	return out;
}

describe('capability scan equivalence', () => {
	it('finds the same evidence over the whole standard library', () => {
		const std = join(import.meta.dirname, '..', '..', 'compiler', 'stdlib', 'src');
		const files: Array<{ name: string; text: string }> = [];
		for (const e of readdirSync(std, { withFileTypes: true })) {
			if (!e.isFile() || !e.name.endsWith('.rnx')) continue;
			files.push({ name: `src/${e.name}`, text: readFileSync(join(std, e.name), 'utf8') });
		}
		const sub = readFileSync(join(std, 'net', 'http.rnx'), 'utf8');
		files.push({ name: 'src/http.rnx', text: sub });
		assert.ok(files.length > 10);
		const legacy = legacyDetect(files);
		const current = detectCapabilityUse(files);
		assert.deepEqual(current, legacy);
	});

	it('finds the same evidence on the shapes that separate the two scans', () => {
		const cases: Array<{ name: string; text: string }> = [
			// Two sinks of one group: the first in the group order wins.
			{ name: 'src/order.rnx', text: 'let a = fs.readText("f");\nlet b = File.open("g");' },
			{ name: 'src/order2.rnx', text: 'let b = File.open("g");\nlet a = fs.readText("f");' },
			// A qualified receiver is not a sink.
			{ name: 'src/qualified.rnx', text: 'let x = doc.readText();\nlet y = obj.fetch("u");' },
			// A declaration is not a call.
			{ name: 'src/decl.rnx', text: 'fn fetch(url: String): String { throw "no"; }' },
			// A sink name in a string or a comment is not a sink.
			{ name: 'src/comment.rnx', text: '// print("x");\n/* File.open("y"); */\nlet s = "fetch(u)";' },
			// The bare sinks.
			{ name: 'src/bare.rnx', text: 'print("hi");\nlet r = await fetch("https://x");' },
			// Two heads in one file.
			{ name: 'src/two.rnx', text: 'print("hi");\nlet t = fs.readText("f");\nlet e = Env.get("HOME") ?? "";' },
			// The unsafe markers.
			{ name: 'src/unsafe.rnx', text: 'import z from "native";\nunsafe { let p = 1; }' },
			// The suffix fallback, with and without a receiver.
			{ name: 'src/suffix.rnx', text: 'let s = doc.readText();' },
			{ name: 'src/suffix2.rnx', text: 'let s = doc.writeBytes(b);' },
			// A sink with space before the parenthesis.
			{ name: 'src/space.rnx', text: 'let t = fs.readText ("f");' },
			// A sink name that is a prefix of another name.
			{ name: 'src/prefix.rnx', text: 'let t = File.readTextx("f");' },
			// A name that ends on an identifier character.
			{ name: 'src/adjacent.rnx', text: 'let t = myFile.open("f");' },
			// The term group and the bare term sink in one file.
			{ name: 'src/term.rnx', text: 'io.write("x");\nprint("y");' },
			// A file that is not rnx.
			{ name: 'README.md', text: 'print("hi");' }
		];
		assert.deepEqual(detectCapabilityUse(cases), legacyDetect(cases));
	});

	it('keeps the symbol of the first sink of the group that matches', () => {
		const out = detectCapabilityUse([
			{ name: 'src/order.rnx', text: 'let a = fs.readText("f");\nlet b = File.open("g");' }
		]);
		assert.deepEqual(out, [
			{ head: 'fs', symbol: 'File.open', file: 'src/order.rnx' },
			{ head: 'term', symbol: 'readText', file: 'src/order.rnx' }
		]);
	});
});
