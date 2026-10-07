import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	emptyRoot,
	normalizePath,
	writeFile,
	readFile,
	ensureDirAll,
	listDir,
	removePath,
	renamePath,
	getNode,
	resolveImportCandidates,
	resolveImportExisting,
	flattenForEngine,
	sortedFileList,
	movePathsForRename,
	countFiles
} from '../src/lib/playground/vfs.ts';
import {
	createProject,
	createSingleFileProject,
	getActiveContent,
	setActiveContent,
	addFile,
	removeProjectFile,
	moveProjectFile,
	setProjectEntry,
	projectFilesJson,
	serializeProject,
	deserializeProject,
	migrateLegacyTabs,
	projectImportTargets,
	allRnxFiles
} from '../src/lib/playground/project-model.ts';
import {
	runShellLine,
	parseRunOutput,
	createShellState,
	SHELL_HELP,
	type ShellEngine
} from '../src/lib/playground/shell.ts';
import { pruneForSave, mergeForLoad } from '../src/lib/playground/ide-store.ts';
import { computeIndent, dedentForClosing, rnxStream } from '../src/lib/aura/rnx-mode.ts';
import type { StringStream } from '@codemirror/language';
import { AURA_KEYWORDS, AURA_TYPES } from '../src/lib/aura/highlight.ts';

describe('vfs paths', () => {
	it('normalizes relative paths against cwd', () => {
		assert.equal(normalizePath('/', 'a/b'), '/a/b');
		assert.equal(normalizePath('/a', './b'), '/a/b');
		assert.equal(normalizePath('/a/b', '../c'), '/a/c');
		assert.equal(normalizePath('/', ''), '/');
	});

	it('rejects escapes above root', () => {
		assert.equal(normalizePath('/', '..'), null);
		assert.equal(normalizePath('/a', '../../x'), null);
		assert.equal(normalizePath('/', ''), '/');
	});

	it('writes, reads, lists and removes files', () => {
		const root = emptyRoot();
		assert.ok(ensureDirAll(root, '/lib').ok);
		assert.ok(writeFile(root, '/lib/util.rnx', 'export fn v(): Int { return 1; }\n').ok);
		const r = readFile(root, '/lib/util.rnx');
		assert.ok(r.ok && r.value.content.includes('export fn v'));
		const l = listDir(root, '/lib');
		assert.ok(l.ok && l.value.entries.length === 1 && l.value.entries[0].name === 'util.rnx');
		assert.ok(removePath(root, '/lib/util.rnx', false).ok);
		assert.equal(getNode(root, '/lib/util.rnx'), null);
	});

	it('refuses to remove non-empty dirs without -r', () => {
		const root = emptyRoot();
		ensureDirAll(root, '/lib');
		writeFile(root, '/lib/a.rnx', 'x');
		const r = removePath(root, '/lib', false);
		assert.ok(!r.ok);
		assert.ok(removePath(root, '/lib', true).ok);
	});

	it('renames files and guards self-moves', () => {
		const root = emptyRoot();
		ensureDirAll(root, '/a');
		writeFile(root, '/a/x.rnx', 'x');
		assert.ok(renamePath(root, '/a/x.rnx', '/b.rnx').ok);
		assert.ok(!renamePath(root, '/nope.rnx', '/b.rnx').ok);
	});

	it('/dev/null sinks writes and reads empty', () => {
		const root = emptyRoot();
		const w = writeFile(root, '/dev/null', 'hello');
		assert.ok(w.ok && w.value.bytes === 5);
		const r = readFile(root, '/dev/null');
		assert.ok(r.ok && r.value.content === '');
		const d = listDir(root, '/dev');
		assert.ok(d.ok && d.value.entries.some((e) => e.name === 'null'));
	});

	it('/dev/stdin serves the shell stdin buffer', () => {
		const root = emptyRoot();
		const r = readFile(root, '/dev/stdin', { stdin: 'hi\n' });
		assert.ok(r.ok && r.value.content === 'hi\n');
		const w = readFile(root, '/dev/stdout');
		assert.ok(!w.ok);
	});

	it('counts files and flattens .rnx sources for the engine', () => {
		const root = emptyRoot();
		ensureDirAll(root, '/lib');
		writeFile(root, '/main.rnx', 'a');
		writeFile(root, '/lib/util.rnx', 'b');
		writeFile(root, '/notes.txt', 'c');
		assert.equal(countFiles(root), 3);
		assert.deepEqual(flattenForEngine(root), { 'main.rnx': 'a', 'lib/util.rnx': 'b' });
		assert.deepEqual(sortedFileList(root), ['/lib/', '/lib/util.rnx', '/main.rnx', '/notes.txt']);
	});

	it('rewrites renamed prefixes for open-file tracking', () => {
		assert.deepEqual(movePathsForRename(['/a/x', '/a/y', '/b'], '/a', '/c'), ['/c/x', '/c/y', '/b']);
	});
});

describe('vfs import resolution mirrors modules.rs probes', () => {
	it('builds probe candidates in order', () => {
		assert.deepEqual(resolveImportCandidates('/main.rnx', './util'), ['/util.rnx', '/util/mod.rnx', '/util/index.rnx']);
		assert.deepEqual(resolveImportCandidates('/lib/a.rnx', '../shared/x'), ['/shared/x.rnx', '/shared/x/mod.rnx', '/shared/x/index.rnx']);
		assert.deepEqual(resolveImportCandidates('/main.rnx', './exact.rnx'), ['/exact.rnx']);
		assert.equal(resolveImportCandidates('/main.rnx', '@std/fs'), null);
		assert.equal(resolveImportCandidates('/main.rnx', 'https://x/y'), null);
		assert.equal(resolveImportCandidates('/main.rnx', '../../escape'), null);
	});

	it('resolves against existing files', () => {
		const root = emptyRoot();
		ensureDirAll(root, '/lib');
		writeFile(root, '/lib/index.rnx', 'x');
		writeFile(root, '/main.rnx', 'x');
		assert.equal(resolveImportExisting(root, '/main.rnx', './lib'), '/lib/index.rnx');
		assert.equal(resolveImportExisting(root, '/main.rnx', './missing'), null);
	});
});

describe('project model', () => {
	it('creates a runnable default project', () => {
		const p = createProject('demo');
		assert.equal(p.entry, '/main.rnx');
		assert.ok(getActiveContent(p).includes('fn Main'));
		const { json, entry } = projectFilesJson(p);
		assert.equal(entry, 'main.rnx');
		assert.ok((JSON.parse(json) as Record<string, string>)['main.rnx'].includes('fn Main'));
	});

	it('adds, moves, removes and guards the entry file', () => {
		const p = createProject('demo');
		const added = addFile(p, '/', 'lib/util.rnx');
		assert.ok(added.ok && added.path === '/lib/util.rnx');
		setActiveContent(p, 'changed');
		assert.equal(getActiveContent(p), 'changed');
		const entryBlock = removeProjectFile(p, '/main.rnx', false);
		assert.ok(!entryBlock.ok);
		assert.ok(setProjectEntry(p, '/lib/util.rnx').ok);
		assert.equal(p.entry, '/lib/util.rnx');
		assert.ok(removeProjectFile(p, '/main.rnx', false).ok);
		assert.deepEqual(allRnxFiles(p), ['/lib/util.rnx']);
	});

	it('round-trips through serialization and rejects junk', () => {
		const p = createSingleFileProject('s', 'fn Main(): Int { return 1; }\n');
		const back = deserializeProject(JSON.parse(JSON.stringify(serializeProject(p))));
		assert.ok(back && back.entry === '/main.rnx');
		assert.equal(deserializeProject(null), null);
		assert.equal(deserializeProject({ id: 'x' }), null);
	});

	it('migrates legacy snippet tabs', () => {
		const p = migrateLegacyTabs([{ name: 'Hello!', code: 'print(1);' }, { code: 'print(2);' }]);
		assert.ok(p);
		assert.ok(allRnxFiles(p).length === 2);
		assert.equal(migrateLegacyTabs([]), null);
		assert.equal(migrateLegacyTabs([{ name: 'x' }]), null);
	});

	it('finds relative import targets for the file manager', () => {
		const p = createProject('demo');
		addFile(p, '/', 'lib/util.rnx');
		writeFile(p.root, '/main.rnx', 'import { v } from "./lib/util";\nfn Main(): Int { return 0; }\n');
		assert.deepEqual(projectImportTargets(p, '/main.rnx'), ['/lib/util.rnx']);
	});
});

const fakeEngine: ShellEngine = {
	checkProject: async (json, entry) => {
		const files = JSON.parse(json) as Record<string, string>;
		return files[entry] ? '' : 'error[E108]: missing entry';
	},
	runProject: async () => ({ output: 'hello\n=> 0', timedOut: false }),
	testProject: async () => ({ output: 'ok check_math\n1 passed, 0 failed', timedOut: false }),
	formatSource: async (s) => s
};

describe('shell file ops', () => {
	it('runs ls/cat/echo/mkdir/touch/rm/mv/cd/pwd', async () => {
		const p = createProject('demo');
		const st = createShellState();
		let r = await runShellLine(p, st, 'ls', fakeEngine);
		assert.equal(r.exitCode, 0);
		assert.ok(r.stdout.includes('main.rnx'));
		r = await runShellLine(p, st, 'mkdir lib', fakeEngine);
		assert.equal(r.exitCode, 0);
		r = await runShellLine(p, st, 'cd lib', fakeEngine);
		assert.equal(r.exitCode, 0);
		assert.equal(st.cwd, '/lib');
		r = await runShellLine(p, st, 'pwd', fakeEngine);
		assert.ok(r.stdout.includes('/lib'));
		r = await runShellLine(p, st, 'touch util.rnx', fakeEngine);
		assert.equal(r.exitCode, 0);
		r = await runShellLine(p, st, 'echo hi > hello.txt', fakeEngine);
		assert.equal(r.exitCode, 0);
		r = await runShellLine(p, st, 'cat hello.txt', fakeEngine);
		assert.equal(r.stdout, 'hi\n');
		r = await runShellLine(p, st, 'cat /dev/null', fakeEngine);
		assert.equal(r.stdout, '');
		r = await runShellLine(p, st, 'cd /', fakeEngine);
		assert.equal(st.cwd, '/');
		r = await runShellLine(p, st, 'mv /lib/util.rnx /top.rnx', fakeEngine);
		assert.equal(r.exitCode, 0);
		r = await runShellLine(p, st, 'rm /top.rnx', fakeEngine);
		assert.equal(r.exitCode, 0);
	});

	it('reads herestring stdin via cat', async () => {
		const p = createProject('demo');
		const st = createShellState();
		const r = await runShellLine(p, st, 'cat <<< hello-stdin', fakeEngine);
		assert.equal(r.stdout, 'hello-stdin\n');
		const d = await runShellLine(p, st, 'cat /dev/stdin', fakeEngine);
		assert.equal(d.stdout, 'hello-stdin');
	});

	it('reports usage errors and unknown commands with codes', async () => {
		const p = createProject('demo');
		const st = createShellState();
		assert.equal((await runShellLine(p, st, 'bogus', fakeEngine)).exitCode, 127);
		assert.equal((await runShellLine(p, st, 'cat /nope', fakeEngine)).exitCode, 1);
		assert.equal((await runShellLine(p, st, 'cat a | cat b', fakeEngine)).exitCode, 2);
		assert.ok(SHELL_HELP.includes('rnx check'));
	});
});

describe('shell rnx subset against the wasm engine', () => {
	it('checks the project entry and reports failures', async () => {
		const p = createProject('demo');
		const st = createShellState();
		const ok = await runShellLine(p, st, 'rnx check', fakeEngine);
		assert.equal(ok.exitCode, 0);
		assert.ok(ok.stdout.includes('ok: main.rnx'));
		const bad = await runShellLine(p, st, 'rnx check /missing.rnx', fakeEngine);
		assert.equal(bad.exitCode, 2);
	});

	it('runs with argv echo and maps statuses to exit codes', async () => {
		const p = createProject('demo');
		const st = createShellState();
		const r = await runShellLine(p, st, 'rnx run -- a b', fakeEngine);
		assert.equal(r.exitCode, 0);
		assert.ok(r.stdout.includes('argv: ["a", "b"]'));
		assert.ok(r.stdout.includes('=> 0'));
		const threw: ShellEngine = { ...fakeEngine, runProject: async () => ({ output: 'thrown: 1', timedOut: false }) };
		const t = await runShellLine(p, st, 'rnx run', threw);
		assert.equal(t.exitCode, 1);
		const slow: ShellEngine = { ...fakeEngine, runProject: async () => ({ output: '', timedOut: true }) };
		const s = await runShellLine(p, st, 'rnx run', slow);
		assert.equal(s.exitCode, 124);
		assert.ok(s.stderr.includes('3000 ms'));
	});

	it('formats files in place', async () => {
		const p = createProject('demo');
		const st = createShellState();
		const same = await runShellLine(p, st, 'rnx fmt', fakeEngine);
		assert.ok(same.stdout.includes('unchanged'));
		const tidy: ShellEngine = { ...fakeEngine, formatSource: async (src) => `${src}\n` };
		const f = await runShellLine(p, st, 'rnx fmt main.rnx', tidy);
		assert.ok(f.stdout.includes('formatted'));
		const failing: ShellEngine = { ...fakeEngine, formatSource: async () => { throw new Error('E108: bad syntax'); } };
		const e = await runShellLine(p, st, 'rnx fmt main.rnx', failing);
		assert.equal(e.exitCode, 2);
	});

	it('tests and filters, and refuses out-of-scope commands with reasons', async () => {
		const p = createProject('demo');
		const st = createShellState();
		const t = await runShellLine(p, st, 'rnx test', fakeEngine);
		assert.equal(t.exitCode, 0);
		const f = await runShellLine(p, st, 'rnx test math', fakeEngine);
		assert.ok(f.stdout.includes('check_math'));
		const build = await runShellLine(p, st, 'rnx build', fakeEngine);
		assert.equal(build.exitCode, 2);
		assert.ok(build.stderr.includes('AOT/LLVM'));
		const pub = await runShellLine(p, st, 'rnx publish', fakeEngine);
		assert.ok(pub.stderr.includes('same-domain'));
		const url = await runShellLine(p, st, 'rnx run https://evil.example/x', fakeEngine);
		assert.ok(url.stderr.includes('E108'));
	});

	it('parses engine run output into streams', () => {
		const parsed = parseRunOutput('a\n=> 1');
		assert.deepEqual(parsed.stdoutLines, ['a']);
		assert.equal(parsed.retval, '=> 1');
		assert.equal(parsed.failed, false);
		const thrown = parseRunOutput('thrown: boom');
		assert.equal(thrown.failed, true);
		assert.equal(thrown.diagnostics, 'thrown: boom');
	});
});

describe('ide store hygiene', () => {
	it('prunes stale and excess projects on save', () => {
		const now = Date.now();
		const mk = (id: string, ageMs: number) => ({
			id,
			name: id,
			root: { kind: 'dir' as const, children: {} },
			activePath: '/main.rnx',
			entry: '/main.rnx',
			updatedAt: now - ageMs
		});
		const state = {
			projects: [mk('fresh', 0), mk('stale', 200 * 24 * 3600 * 1000)],
			activeId: 'stale'
		};
		const pruned = pruneForSave(state, now);
		assert.equal(pruned.projects.length, 1);
		assert.equal(pruned.activeId, 'fresh');
		const many = { projects: Array.from({ length: 12 }, (_, i) => mk(`p${i}`, i * 1000)), activeId: 'p11' };
		assert.equal(pruneForSave(many, now).projects.length, 10);
	});

	it('loads valid projects and falls back cleanly', () => {
		const p = createSingleFileProject('s', 'fn Main(): Int { return 1; }\n');
		const loaded = mergeForLoad({ projects: [serializeProject(p)], activeId: p.id });
		assert.equal(loaded.projects.length, 1);
		assert.equal(mergeForLoad(null).projects.length, 0);
		assert.equal(mergeForLoad({ projects: [{ nope: true }], activeId: null } as unknown as { projects: never[]; activeId: null }).projects.length, 0);
	});
});

describe('rnx editor mode', () => {
	it('keeps oracle keyword and type sets in sync with highlight.ts', () => {
		for (const w of ['fn', 'let', 'import', 'export', 'pass', 'super']) assert.ok(AURA_KEYWORDS.has(w), w);
		for (const w of ['Int', 'Option', 'String', 'Vec4f']) assert.ok(AURA_TYPES.has(w), w);
		assert.ok(!AURA_KEYWORDS.has('taken'));
	});

	it('computes electric indentation deterministically', () => {
		assert.equal(computeIndent('fn Main(): Int {'), '    ');
		assert.equal(computeIndent('    return x;'), '    ');
		assert.equal(computeIndent(''), '');
		assert.equal(dedentForClosing('        '), '    ');
		assert.equal(dedentForClosing(''), '');
	});

	it('classifies tokens through the shared keyword sets', () => {
		const classify = (text: string): (string | null)[] => {
			const words = text.split(' ');
			const out: (string | null)[] = [];
			for (const w of words) {
				let at = 0;
				const fake = {
					eatSpace: () => false,
					sol: () => at === 0,
					match: (p: string | RegExp, consume?: boolean) => {
						if (typeof p === 'string') {
							if (w.slice(at).startsWith(p)) {
								if (consume !== false) at += p.length;
								return p;
							}
							return null;
						}
						const m = p.exec(w.slice(at));
						if (m && m.index === 0) {
							if (consume !== false) at += m[0].length;
							return m[0];
						}
						return null;
					},
					skipToEnd: () => {
						at = w.length;
					},
					current: () => w.slice(0, at),
					next: () => {
						if (at >= w.length) return undefined;
						return w.charAt(at++);
					},
					peek: () => (at < w.length ? w.charAt(at) : undefined),
					eat: () => undefined
				};
				out.push(rnxStream.token(fake as unknown as StringStream, { inBlockComment: false, inString: false }));
			}
			return out;
		};
		assert.deepEqual(classify('fn Int hello 42'), ['keyword', 'typeName', 'variableName', 'number']);
	});
});
