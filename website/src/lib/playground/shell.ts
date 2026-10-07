import {
	ensureDirAll,
	getNode,
	isDir,
	listDir,
	normalizePath,
	parentDir,
	readFile,
	writeFile
} from './vfs';
import {
	addFile,
	engineEntryOf,
	moveProjectFile,
	projectFilesJson,
	removeProjectFile,
	setProjectEntry,
	switchFile,
	type IdeProject
} from './project-model';

export interface ProcResult {
	stdout: string;
	stderr: string;
	exitCode: number;
}

export interface ShellEngine {
	checkProject(json: string, entry: string): Promise<string>;
	runProject(json: string, entry: string): Promise<{ output: string; timedOut: boolean }>;
	testProject(json: string, entry: string): Promise<{ output: string; timedOut: boolean }>;
	formatSource(source: string): Promise<string>;
}

export interface ShellState {
	cwd: string;
	stdin: string;
}

export function createShellState(): ShellState {
	return { cwd: '/', stdin: '' };
}

export interface ParsedRun {
	stdoutLines: string[];
	retval: string;
	diagnostics: string;
	failed: boolean;
}

export function parseRunOutput(output: string): ParsedRun {
	const stdoutLines: string[] = [];
	let retval = '';
	let diagnostics = '';
	let failed = false;
	for (const line of output.split('\n')) {
		if (line.startsWith('=> ')) retval = line;
		else if (line.startsWith('thrown: ') || line.startsWith('fatal: ')) {
			diagnostics = line;
			failed = true;
		} else if (line.length > 0) stdoutLines.push(line);
	}
	return { stdoutLines, retval, diagnostics, failed };
}

function splitArgs(input: string): string[] {
	const out: string[] = [];
	let cur = '';
	let quote: string | null = null;
	for (let i = 0; i < input.length; i++) {
		const c = input[i];
		if (quote) {
			if (c === quote) quote = null;
			else if (c === '\\' && i + 1 < input.length) {
				i += 1;
				cur += input[i];
			} else cur += c;
			continue;
		}
		if (c === '"' || c === "'") {
			quote = c;
			continue;
		}
		if (c === ' ' || c === '\t') {
			if (cur.length > 0) {
				out.push(cur);
				cur = '';
			}
			continue;
		}
		cur += c;
	}
	if (cur.length > 0) out.push(cur);
	return out;
}

interface Redirect {
	mode: 'write' | 'append';
	target: string;
}

function splitRedirect(argv: string[]): { argv: string[]; redirect: Redirect | null; heredoc: string | null } {
	const clean: string[] = [];
	let redirect: Redirect | null = null;
	let heredoc: string | null = null;
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (a === '>' || a === '>>') {
			const next = argv[i + 1];
			if (!next) break;
			redirect = { mode: a === '>' ? 'write' : 'append', target: next };
			i += 1;
			continue;
		}
		if (a === '<<<' && i + 1 < argv.length) {
			heredoc = argv[i + 1];
			i += 1;
			continue;
		}
		if (a.startsWith('<<<')) {
			heredoc = a.slice(3);
			continue;
		}
		clean.push(a);
	}
	return { argv: clean, redirect, heredoc };
}

function applyRedirect(
	p: IdeProject,
	state: ShellState,
	res: ProcResult,
	redirect: Redirect | null
): ProcResult {
	if (!redirect) return res;
	const abs = normalizePath(state.cwd, redirect.target);
	if (!abs || abs.startsWith('/dev')) return { stdout: '', stderr: `bad redirect target: ${redirect.target}\n`, exitCode: 2 };
	const dir = ensureDirAll(p.root, parentDir(abs));
	if (!dir.ok) return { stdout: '', stderr: `${dir.error}\n`, exitCode: 2 };
	let body = res.stdout;
	if (redirect.mode === 'append') {
		const prev = readFile(p.root, abs);
		if (prev.ok) body = prev.value.content + (prev.value.content.endsWith('\n') || prev.value.content.length === 0 ? '' : '\n') + body;
	}
	const w = writeFile(p.root, abs, body);
	if (!w.ok) return { stdout: '', stderr: `${w.error}\n`, exitCode: 2 };
	return { stdout: '', stderr: res.stderr, exitCode: res.exitCode };
}

function usageError(msg: string): ProcResult {
	return { stdout: '', stderr: `${msg}\n`, exitCode: 2 };
}

function resolveShellFile(p: IdeProject, state: ShellState, arg: string | undefined): { path: string } | ProcResult {
	if (arg) {
		const abs = normalizePath(state.cwd, arg);
		if (!abs) return usageError(`bad path: ${arg}`);
		const node = getNode(p.root, abs);
		if (!node || isDir(node)) return usageError(`no such file: ${arg}`);
		return { path: abs };
	}
	return { path: p.activePath };
}

export const SHELL_HELP = [
	'file ops: ls [dir]  cat <file>  echo [text]  touch <file>  mkdir <dir>',
	'          rm [-r] <path>  mv <from> <to>  cd <dir>  pwd  open <file>',
	'project:  entry [file]     show or set the run entrypoint',
	'rnx:      rnx check [file]  rnx run [file] [-- args...]  rnx fmt [file]  rnx test [filter]',
	'devices:  /dev/null sinks writes; cat /dev/stdin reads herestring (cmd <<< text)',
	'not here: rnx build needs AOT/LLVM + a linker (use rnx run); rnx publish/add/fetch',
	'          need registry writes (use rnx check/run with @std/*, fetched same-domain)'
].join('\n');

export async function runShellLine(
	p: IdeProject,
	state: ShellState,
	line: string,
	engine: ShellEngine
): Promise<ProcResult> {
	const { argv, redirect, heredoc } = splitRedirect(splitArgs(line));
	if (heredoc !== null) state.stdin = heredoc;
	if (argv.length === 0) return { stdout: '', stderr: '', exitCode: 0 };
	if (argv.includes('|')) {
		return usageError('pipes are not supported; use > file to capture output');
	}
	const [cmd, ...rest] = argv;
	let res: ProcResult;
	switch (cmd) {
		case 'help':
			res = { stdout: `${SHELL_HELP}\n`, stderr: '', exitCode: 0 };
			break;
		case 'pwd':
			res = { stdout: `${state.cwd}\n`, stderr: '', exitCode: 0 };
			break;
		case 'ls': {
			const abs = normalizePath(state.cwd, rest[0] ?? '.');
			if (!abs) {
				res = usageError(`bad path: ${rest[0]}`);
				break;
			}
			const node = getNode(p.root, abs);
			if (node && !isDir(node)) {
				res = { stdout: `${abs}\n`, stderr: '', exitCode: 0 };
				break;
			}
			const l = listDir(p.root, abs);
			if (!l.ok) {
				res = { stdout: '', stderr: `${l.error}\n`, exitCode: 1 };
				break;
			}
			res = {
				stdout: l.value.entries.map((e) => (e.kind === 'dir' ? `${e.name}/` : e.name)).join('\n') + (l.value.entries.length > 0 ? '\n' : ''),
				stderr: '',
				exitCode: 0
			};
			break;
		}
		case 'cd': {
			const abs = normalizePath(state.cwd, rest[0] ?? '/');
			if (!abs) {
				res = usageError(`bad path: ${rest[0]}`);
				break;
			}
			const node = getNode(p.root, abs);
			if (!node || !isDir(node)) {
				res = { stdout: '', stderr: `no such directory: ${rest[0] ?? '/'}\n`, exitCode: 1 };
				break;
			}
			state.cwd = abs;
			res = { stdout: '', stderr: '', exitCode: 0 };
			break;
		}
		case 'cat': {
			if (rest.length === 0) {
				res = { stdout: `${state.stdin}${state.stdin.endsWith('\n') || state.stdin.length === 0 ? '' : '\n'}`, stderr: '', exitCode: 0 };
				break;
			}
			const abs = normalizePath(state.cwd, rest[0]);
			if (!abs) {
				res = usageError(`bad path: ${rest[0]}`);
				break;
			}
			const r = readFile(p.root, abs, { stdin: state.stdin });
			res = r.ok ? { stdout: r.value.content, stderr: '', exitCode: 0 } : { stdout: '', stderr: `${r.error}\n`, exitCode: 1 };
			break;
		}
		case 'echo': {
			res = { stdout: `${rest.join(' ')}\n`, stderr: '', exitCode: 0 };
			break;
		}
		case 'touch': {
			if (!rest[0]) {
				res = usageError('usage: touch <file>');
				break;
			}
			const r = addFile(p, state.cwd, rest[0]);
			res = r.ok ? { stdout: '', stderr: '', exitCode: 0 } : { stdout: '', stderr: `${r.error}\n`, exitCode: 1 };
			break;
		}
		case 'mkdir': {
			const dirs = rest[0] === '-p' ? rest.slice(1) : rest;
			if (dirs.length === 0) {
				res = usageError('usage: mkdir [-p] <dir>');
				break;
			}
			const abs = normalizePath(state.cwd, dirs[0]);
			if (!abs || abs.startsWith('/dev')) {
				res = usageError(`bad path: ${dirs[0]}`);
				break;
			}
			const r = ensureDirAll(p.root, abs);
			res = r.ok ? { stdout: '', stderr: '', exitCode: 0 } : { stdout: '', stderr: `${r.error}\n`, exitCode: 1 };
			break;
		}
		case 'rm': {
			const recursive = rest[0] === '-r';
			const target = recursive ? rest[1] : rest[0];
			if (!target) {
				res = usageError('usage: rm [-r] <path>');
				break;
			}
			const abs = normalizePath(state.cwd, target);
			if (!abs) {
				res = usageError(`bad path: ${target}`);
				break;
			}
			const r = removeProjectFile(p, abs, recursive);
			res = r.ok ? { stdout: '', stderr: '', exitCode: 0 } : { stdout: '', stderr: `${r.error}\n`, exitCode: 1 };
			break;
		}
		case 'mv': {
			if (rest.length < 2) {
				res = usageError('usage: mv <from> <to>');
				break;
			}
			const from = normalizePath(state.cwd, rest[0]);
			if (!from) {
				res = usageError(`bad path: ${rest[0]}`);
				break;
			}
			const r = moveProjectFile(p, from, rest[1], state.cwd);
			res = r.ok ? { stdout: '', stderr: '', exitCode: 0 } : { stdout: '', stderr: `${r.error}\n`, exitCode: 1 };
			break;
		}
		case 'open': {
			if (!rest[0]) {
				res = usageError('usage: open <file>');
				break;
			}
			const abs = normalizePath(state.cwd, rest[0]);
			if (!abs || !switchFile(p, abs)) {
				res = { stdout: '', stderr: `no such file: ${rest[0]}\n`, exitCode: 1 };
				break;
			}
			res = { stdout: `opened ${abs}\n`, stderr: '', exitCode: 0 };
			break;
		}
		case 'entry': {
			if (!rest[0]) {
				res = { stdout: `${p.entry}\n`, stderr: '', exitCode: 0 };
				break;
			}
			const abs = normalizePath(state.cwd, rest[0]);
			if (!abs) {
				res = usageError(`bad path: ${rest[0]}`);
				break;
			}
			const r = setProjectEntry(p, abs);
			res = r.ok ? { stdout: `entry: ${p.entry}\n`, stderr: '', exitCode: 0 } : { stdout: '', stderr: `${r.error}\n`, exitCode: 1 };
			break;
		}
		case 'rnx':
			res = await runRnx(p, state, rest, engine);
			break;
		default:
			res = { stdout: '', stderr: `unknown command: ${cmd} (try help)\n`, exitCode: 127 };
	}
	return applyRedirect(p, state, res, redirect);
}

function entryFor(p: IdeProject, state: ShellState, arg: string | undefined): { entry: string } | ProcResult {
	if (!arg || arg === '--') return { entry: engineEntryOf(p) };
	if (arg.startsWith('-')) return usageError(`unknown flag: ${arg}`);
	const abs = normalizePath(state.cwd, arg);
	if (!abs) return usageError(`bad path: ${arg}`);
	if (!abs.endsWith('.rnx')) return usageError('rnx target must be a .rnx file');
	const node = getNode(p.root, abs);
	if (!node || isDir(node)) return usageError(`no such file: ${arg}`);
	return { entry: abs.slice(1) };
}

async function runRnx(p: IdeProject, state: ShellState, rest: string[], engine: ShellEngine): Promise<ProcResult> {
	const [sub, ...args] = rest;
	if (!sub || sub === 'help' || sub === '--help') {
		return { stdout: 'usage: rnx check [file] | rnx run [file] [-- args...] | rnx fmt [file] | rnx test [filter]\n', stderr: '', exitCode: sub ? 0 : 2 };
	}
	for (const a of args) {
		if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(a)) {
			return { stdout: '', stderr: `error[E108]: URL imports are forbidden: ${a}\n`, exitCode: 2 };
		}
	}
	switch (sub) {
		case 'check': {
			const e = entryFor(p, state, args[0]);
			if (!('entry' in e)) return e;
			try {
				const { json } = projectFilesJson(p);
				const text = await engine.checkProject(json, e.entry);
				return text
					? { stdout: '', stderr: `${text}${text.endsWith('\n') ? '' : '\n'}`, exitCode: 2 }
					: { stdout: `ok: ${e.entry}\n`, stderr: '', exitCode: 0 };
			} catch (err) {
				return { stdout: '', stderr: `${err instanceof Error ? err.message : 'check failed'}\n`, exitCode: 2 };
			}
		}
		case 'run': {
			const dash = args.indexOf('--');
			const target = dash === 0 ? undefined : args[0];
			const argv = dash >= 0 ? args.slice(dash + 1) : [];
			if (dash < 0 && args.length > 1) {
				return usageError('pass program arguments after -- : rnx run [file] [-- args...]');
			}
			const e = entryFor(p, state, target);
			if (!('entry' in e)) return e;
			try {
				const { json } = projectFilesJson(p);
				const res = await engine.runProject(json, e.entry);
				if (res.timedOut) {
					return { stdout: '', stderr: 'run exceeded 3000 ms and was terminated\n', exitCode: 124 };
				}
				const parsed = parseRunOutput(res.output);
				const head = argv.length > 0 ? `argv: [${argv.map((a) => JSON.stringify(a)).join(', ')}]\n` : '';
				if (parsed.diagnostics && !parsed.retval) {
					return { stdout: head, stderr: `${parsed.diagnostics}\n`, exitCode: 1 };
				}
				const body = [...parsed.stdoutLines, parsed.retval].filter((l) => l.length > 0).join('\n');
				return {
					stdout: head + (body.length > 0 ? `${body}\n` : ''),
					stderr: parsed.diagnostics ? `${parsed.diagnostics}\n` : '',
					exitCode: parsed.failed ? 1 : 0
				};
			} catch (err) {
				return { stdout: '', stderr: `${err instanceof Error ? err.message : 'run failed'}\n`, exitCode: 1 };
			}
		}
		case 'fmt': {
			const r = resolveShellFile(p, state, args[0]);
			if (!('path' in r)) return r;
			try {
				const current = readFile(p.root, r.path);
				if (!current.ok) return { stdout: '', stderr: `${current.error}\n`, exitCode: 1 };
				const after = await engine.formatSource(current.value.content);
				if (after === current.value.content) return { stdout: `unchanged: ${r.path}\n`, stderr: '', exitCode: 0 };
				writeFile(p.root, r.path, after);
				return { stdout: `formatted: ${r.path}\n`, stderr: '', exitCode: 0 };
			} catch (err) {
				return { stdout: '', stderr: `${err instanceof Error ? err.message : 'fmt failed'}\n`, exitCode: 2 };
			}
		}
		case 'test': {
			const filter = args[0] && !args[0].startsWith('-') ? args[0] : undefined;
			try {
				const { json, entry } = projectFilesJson(p);
				const res = await engine.testProject(json, entry);
				if (res.timedOut) {
					return { stdout: '', stderr: 'run exceeded 3000 ms and was terminated\n', exitCode: 124 };
				}
				let lines = res.output.split('\n');
				if (filter) {
					lines = lines.filter((l) => l.includes(filter) || l.includes('passed,'));
				}
				const failed = /FAIL/.test(res.output) || /[1-9]\d* failed/.test(res.output);
				return { stdout: `${lines.join('\n')}${'\n'}`, stderr: '', exitCode: failed ? 1 : 0 };
			} catch (err) {
				return { stdout: '', stderr: `${err instanceof Error ? err.message : 'test failed'}\n`, exitCode: 1 };
			}
		}
		case 'build':
			return {
				stdout: '',
				stderr: 'rnx build is not available in the browser playground: AOT/LLVM codegen and native linking need a local toolchain. Use rnx run to execute on the WebAssembly engine.\n',
				exitCode: 2
			};
		case 'publish':
		case 'pack':
		case 'add':
		case 'fetch':
		case 'update':
			return {
				stdout: '',
				stderr: `rnx ${sub} is not available in the browser playground: registry writes and package fetches go beyond the same-domain @std/* chunk fetching the engine already does. Use rnx check/run/test.\n`,
				exitCode: 2
			};
		case 'bench':
			return {
				stdout: '',
				stderr: 'rnx bench is not wired into the IDE shell; use rnx run for execution timing.\n',
				exitCode: 2
			};
		default:
			return usageError(`unknown rnx command: ${sub}`);
	}
}
