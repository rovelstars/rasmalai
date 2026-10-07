export interface VfsFile {
	kind: 'file';
	content: string;
}

export interface VfsDir {
	kind: 'dir';
	children: Record<string, VfsNode>;
}

export type VfsNode = VfsFile | VfsDir;

export interface VfsError {
	ok: false;
	error: string;
}

export interface VfsOk<T = void> {
	ok: true;
	value: T;
}

export type VfsResult<T = void> = VfsOk<T> | VfsError;

export function emptyRoot(): VfsDir {
	return { kind: 'dir', children: {} };
}

export function isDir(node: VfsNode): node is VfsDir {
	return node.kind === 'dir';
}

export function splitPath(abs: string): string[] {
	return abs.split('/').filter((p) => p.length > 0);
}

export function normalizePath(cwd: string, input: string): string | null {
	const raw = input.trim();
	if (raw.length === 0) return cwd;
	if (raw.includes('\0')) return null;
	const abs = raw.startsWith('/') ? raw : cwd === '/' ? `/${raw}` : `${cwd}/${raw}`;
	const parts: string[] = [];
	for (const piece of abs.split('/')) {
		if (piece === '' || piece === '.') continue;
		if (piece === '..') {
			if (parts.length === 0) return null;
			parts.pop();
			continue;
		}
		if (piece.length > 120) return null;
		parts.push(piece);
	}
	return `/${parts.join('/')}`;
}

export function parentDir(abs: string): string {
	const parts = splitPath(abs);
	parts.pop();
	return `/${parts.join('/')}`;
}

export function baseName(abs: string): string {
	const parts = splitPath(abs);
	return parts[parts.length - 1] ?? '';
}

export function getNode(root: VfsDir, abs: string): VfsNode | null {
	if (abs === '/') return root;
	let node: VfsNode = root;
	for (const part of splitPath(abs)) {
		if (!isDir(node)) return null;
		const next: VfsNode | undefined = node.children[part];
		if (!next) return null;
		node = next;
	}
	return node;
}

function parentOf(root: VfsDir, abs: string): { dir: VfsDir; name: string } | null {
	if (abs === '/') return null;
	const dir = getNode(root, parentDir(abs));
	if (!dir || !isDir(dir)) return null;
	return { dir, name: baseName(abs) };
}

export function ensureDir(root: VfsDir, abs: string): VfsResult {
	if (abs === '/') return { ok: true, value: undefined };
	const existing = getNode(root, abs);
	if (existing) {
		return isDir(existing) ? { ok: true, value: undefined } : { ok: false, error: `not a directory: ${abs}` };
	}
	const parent = parentOf(root, abs);
	if (!parent) return { ok: false, error: `no such directory: ${parentDir(abs)}` };
	parent.dir.children[parent.name] = { kind: 'dir', children: {} };
	return { ok: true, value: undefined };
}

export function ensureDirAll(root: VfsDir, abs: string): VfsResult {
	if (abs === '/') return { ok: true, value: undefined };
	const parts = splitPath(abs);
	let at = '';
	for (const part of parts) {
		at += `/${part}`;
		const r = ensureDir(root, at);
		if (!r.ok) return r;
	}
	return { ok: true, value: undefined };
}

export const MAX_FILE_BYTES = 262_144;
export const MAX_FILES = 64;

export function countFiles(root: VfsDir): number {
	let n = 0;
	const walk = (dir: VfsDir): void => {
		for (const child of Object.values(dir.children)) {
			if (isDir(child)) walk(child);
			else n += 1;
		}
	};
	walk(root);
	return n;
}

export function writeFile(root: VfsDir, abs: string, content: string): VfsResult<{ bytes: number }> {
	if (abs === '/dev/null') return { ok: true, value: { bytes: content.length } };
	if (abs === '/dev/stdout' || abs === '/dev/stderr') {
		return { ok: false, error: `${abs} is write-only through process output` };
	}
	if (abs.startsWith('/dev/')) return { ok: false, error: `cannot write to device: ${abs}` };
	if (content.length > MAX_FILE_BYTES) return { ok: false, error: `file exceeds 256 KiB: ${abs}` };
	const parent = parentOf(root, abs);
	if (!parent) return { ok: false, error: `no such directory: ${parentDir(abs)}` };
	const prev = parent.dir.children[parent.name];
	if (prev && isDir(prev)) return { ok: false, error: `is a directory: ${abs}` };
	if (!prev && countFiles(root) >= MAX_FILES) return { ok: false, error: 'project exceeds 64 files' };
	parent.dir.children[parent.name] = { kind: 'file', content };
	return { ok: true, value: { bytes: content.length } };
}

export interface ReadDevices {
	stdin: string;
}

export function readFile(root: VfsDir, abs: string, devices?: ReadDevices): VfsResult<{ content: string }> {
	if (abs === '/dev/null') return { ok: true, value: { content: '' } };
	if (abs === '/dev/stdin') return { ok: true, value: { content: devices?.stdin ?? '' } };
	if (abs === '/dev/stdout' || abs === '/dev/stderr') {
		return { ok: false, error: `${abs} is write-only` };
	}
	const node = getNode(root, abs);
	if (!node) return { ok: false, error: `no such file: ${abs}` };
	if (isDir(node)) return { ok: false, error: `is a directory: ${abs}` };
	return { ok: true, value: { content: node.content } };
}

export function removePath(root: VfsDir, abs: string, recursive: boolean): VfsResult {
	if (abs === '/' || abs.startsWith('/dev')) return { ok: false, error: `cannot remove: ${abs}` };
	const parent = parentOf(root, abs);
	if (!parent) return { ok: false, error: `no such file: ${abs}` };
	const node = parent.dir.children[parent.name];
	if (!node) return { ok: false, error: `no such file: ${abs}` };
	if (isDir(node) && Object.keys(node.children).length > 0 && !recursive) {
		return { ok: false, error: `directory not empty: ${abs}` };
	}
	delete parent.dir.children[parent.name];
	return { ok: true, value: undefined };
}

export interface DirEntry {
	name: string;
	kind: 'file' | 'dir';
}

export function listDir(root: VfsDir, abs: string): VfsResult<{ entries: DirEntry[] }> {
	if (abs === '/dev') {
		return {
			ok: true,
			value: {
				entries: [
					{ name: 'null', kind: 'file' },
					{ name: 'stdin', kind: 'file' },
					{ name: 'stdout', kind: 'file' },
					{ name: 'stderr', kind: 'file' }
				]
			}
		};
	}
	const node = getNode(root, abs);
	if (!node) return { ok: false, error: `no such directory: ${abs}` };
	if (!isDir(node)) return { ok: false, error: `not a directory: ${abs}` };
	const entries = Object.keys(node.children)
		.sort()
		.map((name) => ({
			name,
			kind: (isDir(node.children[name]) ? 'dir' : 'file') as 'file' | 'dir'
		}));
	return { ok: true, value: { entries } };
}

export function renamePath(root: VfsDir, from: string, to: string): VfsResult {
	if (from === '/' || to === '/' || from.startsWith('/dev') || to.startsWith('/dev')) {
		return { ok: false, error: 'cannot move devices or root' };
	}
	const srcParent = parentOf(root, from);
	if (!srcParent) return { ok: false, error: `no such file: ${from}` };
	const node = srcParent.dir.children[srcParent.name];
	if (!node) return { ok: false, error: `no such file: ${from}` };
	const dstParent = parentOf(root, to);
	if (!dstParent) return { ok: false, error: `no such directory: ${parentDir(to)}` };
	if (dstParent.dir.children[dstParent.name]) return { ok: false, error: `already exists: ${to}` };
	if (isDir(node) && to.startsWith(from === '/' ? '/' : `${from}/`)) {
		return { ok: false, error: 'cannot move a directory into itself' };
	}
	delete srcParent.dir.children[srcParent.name];
	dstParent.dir.children[dstParent.name] = node;
	return { ok: true, value: undefined };
}

export function movePathsForRename(files: string[], from: string, to: string): string[] {
	const prefix = from === '/' ? '/' : `${from}/`;
	return files.map((f) => (f === from || f.startsWith(prefix) ? to + f.slice(from.length) : f));
}

export function resolveImportCandidates(fromFile: string, spec: string): string[] | null {
	const t = spec.trim();
	if (!t.startsWith('.')) return null;
	if (t.includes('\0') || t.includes(':')) return null;
	const dir = parentDir(fromFile);
	const tryJoin = (rel: string): string | null => {
		const parts: string[] = splitPath(dir);
		for (const piece of rel.split('/')) {
			if (piece === '' || piece === '.') continue;
			if (piece === '..') {
				if (parts.length === 0) return null;
				parts.pop();
				continue;
			}
			parts.push(piece);
		}
		return `/${parts.join('/')}`;
	};
	const base = tryJoin(t);
	if (!base) return null;
	if (/\.[A-Za-z0-9]+$/.test(base)) return [base];
	return [`${base}.rnx`, `${base}/mod.rnx`, `${base}/index.rnx`];
}

export function resolveImportExisting(root: VfsDir, fromFile: string, spec: string): string | null {
	const candidates = resolveImportCandidates(fromFile, spec);
	if (!candidates) return null;
	for (const c of candidates) {
		const node = getNode(root, c);
		if (node && !isDir(node)) return c;
	}
	return null;
}

export function flattenForEngine(root: VfsDir): Record<string, string> {
	const out: Record<string, string> = {};
	const walk = (dir: VfsDir, prefix: string): void => {
		for (const name of Object.keys(dir.children).sort()) {
			const child = dir.children[name];
			const path = prefix === '/' ? `/${name}` : `${prefix}/${name}`;
			if (isDir(child)) walk(child, path);
			else if (path.endsWith('.rnx')) out[path.slice(1)] = child.content;
		}
	};
	walk(root, '/');
	return out;
}

export function sortedFileList(root: VfsDir): string[] {
	const out: string[] = [];
	const walk = (dir: VfsDir, prefix: string): void => {
		for (const name of Object.keys(dir.children).sort()) {
			const child = dir.children[name];
			const path = prefix === '/' ? `/${name}` : `${prefix}/${name}`;
			if (isDir(child)) {
				out.push(`${path}/`);
				walk(child, path);
			} else {
				out.push(path);
			}
		}
	};
	walk(root, '/');
	return out;
}
