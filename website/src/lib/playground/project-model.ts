import {
	countFiles,
	emptyRoot,
	ensureDirAll,
	flattenForEngine,
	getNode,
	isDir,
	listDir,
	normalizePath,
	parentDir,
	readFile,
	removePath,
	renamePath,
	resolveImportExisting,
	sortedFileList,
	writeFile,
	type VfsDir
} from './vfs';

export interface IdeProject {
	id: string;
	name: string;
	root: VfsDir;
	activePath: string;
	entry: string;
	updatedAt: number;
}

export function defaultMainSource(): string {
	return 'fn Main(): Int {\n    print("hello from Rasmalai");\n    return 0;\n}\n';
}

export function createProject(name: string, id?: string): IdeProject {
	const root = emptyRoot();
	writeFile(root, '/main.rnx', defaultMainSource());
	return {
		id: id ?? `p${Date.now().toString(36)}`,
		name: name.slice(0, 40) || 'Untitled project',
		root,
		activePath: '/main.rnx',
		entry: '/main.rnx',
		updatedAt: Date.now()
	};
}

export function createSingleFileProject(name: string, source: string, id?: string): IdeProject {
	const p = createProject(name, id);
	writeFile(p.root, '/main.rnx', source);
	return p;
}

export function getActiveContent(p: IdeProject): string {
	const r = readFile(p.root, p.activePath);
	return r.ok ? r.value.content : '';
}

export function setActiveContent(p: IdeProject, content: string): boolean {
	const r = writeFile(p.root, p.activePath, content);
	if (r.ok) p.updatedAt = Date.now();
	return r.ok;
}

export function switchFile(p: IdeProject, abs: string): boolean {
	const node = getNode(p.root, abs);
	if (!node || isDir(node)) return false;
	p.activePath = abs;
	return true;
}

export function projectFilesJson(p: IdeProject): { json: string; entry: string } {
	return { json: JSON.stringify(flattenForEngine(p.root)), entry: p.entry.slice(1) };
}

export function engineEntryOf(p: IdeProject): string {
	return p.entry.replace(/^\//, '');
}

export function allRnxFiles(p: IdeProject): string[] {
	return sortedFileList(p.root).filter((f) => f.endsWith('.rnx'));
}

export function entryCandidates(p: IdeProject): string[] {
	return allRnxFiles(p);
}

export function isEntryValid(p: IdeProject): boolean {
	const node = getNode(p.root, p.entry);
	return !!node && !isDir(node);
}

export interface ProjectOp {
	ok: boolean;
	error?: string;
}

export function addFile(p: IdeProject, cwd: string, input: string): ProjectOp & { path?: string } {
	const abs = normalizePath(cwd, input);
	if (!abs) return { ok: false, error: `bad path: ${input}` };
	if (abs.startsWith('/dev')) return { ok: false, error: 'reserved device path' };
	if (getNode(p.root, abs)) return { ok: false, error: `already exists: ${abs}` };
	const dir = ensureDirAll(p.root, parentDir(abs));
	if (!dir.ok) return { ok: false, error: dir.error };
	const name = abs.split('/').pop() ?? '';
	const seed = abs.endsWith('.rnx') ? defaultMainSource() : '';
	const w = writeFile(p.root, abs, seed);
	if (!w.ok) return { ok: false, error: w.error };
	void name;
	p.activePath = abs;
	p.updatedAt = Date.now();
	return { ok: true, path: abs };
}

export function removeProjectFile(p: IdeProject, abs: string, recursive: boolean): ProjectOp {
	if (abs === p.entry) return { ok: false, error: 'cannot delete the entry file; pick another entry first' };
	const r = removePath(p.root, abs, recursive);
	if (!r.ok) return { ok: false, error: r.error };
	if (p.activePath === abs || p.activePath.startsWith(`${abs}/`)) {
		p.activePath = isEntryValid(p) ? p.entry : (allRnxFiles(p)[0] ?? '/main.rnx');
	}
	p.updatedAt = Date.now();
	return { ok: true };
}

export function moveProjectFile(p: IdeProject, from: string, toRaw: string, cwd: string): ProjectOp & { path?: string } {
	const to = normalizePath(cwd, toRaw);
	if (!to) return { ok: false, error: `bad path: ${toRaw}` };
	if (to.startsWith('/dev')) return { ok: false, error: 'reserved device path' };
	const r = renamePath(p.root, from, to);
	if (!r.ok) return { ok: false, error: r.error };
	if (p.activePath === from) p.activePath = to;
	if (p.entry === from) p.entry = to;
	p.updatedAt = Date.now();
	return { ok: true, path: to };
}

export function setProjectEntry(p: IdeProject, abs: string): ProjectOp {
	const node = getNode(p.root, abs);
	if (!node || isDir(node)) return { ok: false, error: `no such file: ${abs}` };
	if (!abs.endsWith('.rnx')) return { ok: false, error: 'entry must be a .rnx file' };
	p.entry = abs;
	p.updatedAt = Date.now();
	return { ok: true };
}

export function resolveProjectImport(p: IdeProject, fromFile: string, spec: string): string | null {
	return resolveImportExisting(p.root, fromFile, spec);
}

export function projectImportTargets(p: IdeProject, fromFile: string): string[] {
	const r = readFile(p.root, fromFile);
	if (!r.ok) return [];
	const out: string[] = [];
	const re = /from\s+"([^"]+)"/g;
	let m: RegExpExecArray | null;
	while ((m = re.exec(r.value.content)) !== null) {
		const hit = resolveImportExisting(p.root, fromFile, m[1]);
		if (hit) out.push(hit);
	}
	return out;
}

export interface SerializedProject {
	id: string;
	name: string;
	root: VfsDir;
	activePath: string;
	entry: string;
	updatedAt: number;
}

export function serializeProject(p: IdeProject): SerializedProject {
	return {
		id: p.id,
		name: p.name,
		root: p.root,
		activePath: p.activePath,
		entry: p.entry,
		updatedAt: p.updatedAt
	};
}

export function deserializeProject(raw: unknown): IdeProject | null {
	if (typeof raw !== 'object' || raw === null) return null;
	const o = raw as Record<string, unknown>;
	if (typeof o['id'] !== 'string' || typeof o['entry'] !== 'string') return null;
	if (typeof o['root'] !== 'object' || o['root'] === null) return null;
	const root = o['root'] as VfsDir;
	if (root.kind !== 'dir' || typeof root.children !== 'object') return null;
	if (countFiles(root) > 64) return null;
	const p: IdeProject = {
		id: o['id'] as string,
		name: typeof o['name'] === 'string' ? (o['name'] as string).slice(0, 40) : 'Untitled project',
		root,
		activePath: typeof o['activePath'] === 'string' ? (o['activePath'] as string) : '/main.rnx',
		entry: o['entry'] as string,
		updatedAt: typeof o['updatedAt'] === 'number' ? (o['updatedAt'] as number) : Date.now()
	};
	if (!isEntryValid(p)) {
		const first = allRnxFiles(p)[0];
		if (!first) return null;
		p.entry = first;
	}
	const active = getNode(p.root, p.activePath);
	if (!active || isDir(active)) p.activePath = p.entry;
	return p;
}

export interface LegacyTab {
	id?: unknown;
	name?: unknown;
	code?: unknown;
}

export function migrateLegacyTabs(tabs: LegacyTab[]): IdeProject | null {
	const files = tabs.filter((t) => t && typeof t.code === 'string');
	if (files.length === 0) return null;
	const root = emptyRoot();
	let n = 0;
	for (const t of files.slice(0, 64)) {
		n += 1;
		const safe = typeof t.name === 'string' && t.name ? t.name.replace(/[^A-Za-z0-9_-]+/g, '_').slice(0, 24) : `snippet_${n}`;
		writeFile(root, `/${safe}.rnx`, t.code as string);
	}
	const first = sortedFileList(root).find((f) => f.endsWith('.rnx')) ?? '/main.rnx';
	return {
		id: `p${Date.now().toString(36)}m`,
		name: 'Migrated snippets',
		root,
		activePath: first,
		entry: first,
		updatedAt: Date.now()
	};
}

export { listDir, ensureDirAll };
