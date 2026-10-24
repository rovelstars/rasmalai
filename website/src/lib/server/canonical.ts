import { checkManifestEntries } from './chunks.js';
import { parsePackageName } from './db.js';
import { VERSION_RE } from './registry.js';

export const PROTOCOL_VERSION = '0';
export const PROTOCOL_HEADER = 'X-RNX-Protocol';
export const REDACTED_VERSION = '0.0.0-published';

export const MAX_MANIFEST_BYTES = 512 * 1024;
const FREE_SOURCE_BYTES = 16 * 1024;
const PAID_SOURCE_BYTES = 4 * 1024 * 1024;
const FREE_FILES = 64;
const PAID_FILES = 4096;
const FREE_NEW_CHUNKS = 32;
const PAID_NEW_CHUNKS = 512;
export const MAX_CHUNKS_PER_VERSION = 4096;

export interface PlanCaps {
	sourceBytes: number;
	files: number;
	newChunks: number;
	chunksPerVersion: number;
	manifestBytes: number;
}

export function planCaps(env: Record<string, string | undefined>): PlanCaps {
	if (env['REGISTRY_PLAN'] === 'paid') {
		return {
			sourceBytes: PAID_SOURCE_BYTES,
			files: PAID_FILES,
			newChunks: PAID_NEW_CHUNKS,
			chunksPerVersion: MAX_CHUNKS_PER_VERSION,
			manifestBytes: MAX_MANIFEST_BYTES
		};
	}
	return {
		sourceBytes: FREE_SOURCE_BYTES,
		files: FREE_FILES,
		newChunks: FREE_NEW_CHUNKS,
		chunksPerVersion: MAX_CHUNKS_PER_VERSION,
		manifestBytes: MAX_MANIFEST_BYTES
	};
}

export interface PublishFileInput {
	path: string;
	content: string;
}

export interface PublishFrame {
	name: string;
	version: string;
	files: PublishFileInput[];
	description: string;
	readme: string;
	docJson: string;
	guidesJson: string;
	engineRange: string;
	keywords: string[];
	tags: string[];
	license: string;
	requestId: string;
}

export type FrameError = { status: number; message: string };

function fail(status: number, message: string): FrameError {
	return { status, message };
}

const ROOT_FILES = new Set(['Project.config', 'README', 'README.md', 'LICENSE', 'LICENSE.txt']);

// The file classes a publish may carry. Directories are not entries:
// the server chunks file bytes, and directory assembly is the client's
// job at install. Docs ride under the configured docs dir only, and
// `.rnx` sources ride under `src/` or an entry target.
export function fileAllowed(path: string, entryTargets: Set<string>, docsDir: string | null): boolean {
	if (ROOT_FILES.has(path)) return true;
	if (entryTargets.has(path)) return true;
	if (path.startsWith('src/') && path.endsWith('.rnx')) return true;
	if (docsDir && path.startsWith(`${docsDir}/`) && path.endsWith('.md')) return true;
	return false;
}

export function parsePublishFrame(body: unknown, caps: PlanCaps): { ok: true; frame: PublishFrame } | { ok: false; error: FrameError } {
	if (!body || typeof body !== 'object' || Array.isArray(body)) {
		return { ok: false, error: fail(400, 'publish body must be a JSON object') };
	}
	const v = body as Record<string, unknown>;
	const str = (key: string): string | null =>
		v[key] === undefined ? null : typeof v[key] === 'string' ? (v[key] as string) : null;
	const name = str('name');
	const version = str('version');
	if (!name || !parsePackageName(name)) {
		return { ok: false, error: fail(400, 'invalid package name (expected name or @scope/name)') };
	}
	if (!version || !VERSION_RE.test(version)) {
		return { ok: false, error: fail(400, 'invalid package version (expected X.Y.Z)') };
	}
	if (!Array.isArray(v['files']) || v['files'].length === 0) {
		return { ok: false, error: fail(400, 'publish body carries no files') };
	}
	if (v['files'].length > caps.files) {
		return { ok: false, error: fail(413, `package lists ${v['files'].length} files, over the ${caps.files} file limit`) };
	}
	const rawFiles = v['files'] as unknown[];
	const files: PublishFileInput[] = [];
	const seen = new Set<string>();
	let sourceBytes = 0;
	for (const item of rawFiles) {
		if (!item || typeof item !== 'object' || Array.isArray(item)) {
			return { ok: false, error: fail(400, 'each file needs a `path` and a `content` string') };
		}
		const row = item as Record<string, unknown>;
		if (typeof row['path'] !== 'string' || typeof row['content'] !== 'string') {
			return { ok: false, error: fail(400, 'each file needs a `path` and a `content` string') };
		}
		const path = row['path'] as string;
		const content = row['content'] as string;
		if (seen.has(path)) return { ok: false, error: fail(400, `duplicate file \`${path}\``) };
		seen.add(path);
		sourceBytes += Buffer.byteLength(content, 'utf8');
		if (sourceBytes > caps.sourceBytes) {
			return { ok: false, error: fail(413, `package source exceeds the ${caps.sourceBytes} byte limit`) };
		}
		files.push({ path, content });
	}
	if (!seen.has('Project.config')) {
		return { ok: false, error: fail(400, 'package ships no Project.config') };
	}
	if (!seen.has('LICENSE') && !seen.has('LICENSE.txt')) {
		return { ok: false, error: fail(400, 'package ships no LICENSE') };
	}
	const bad = checkManifestEntries(files.map((f) => ({ name: f.path, size: 0, dir: false, chunks: [] })));
	if (bad) return { ok: false, error: fail(400, `unsafe entry name \`${bad.name}\`: ${bad.reason}`) };
	const keywords = v['keywords'];
	const tags = v['tags'];
	const frame: PublishFrame = {
		name,
		version,
		files,
		description: str('description') ?? `${name} package`,
		readme: str('readme') ?? `# ${name}\n`,
		docJson: str('docJson') ?? '{"modules":[]}',
		guidesJson: Array.isArray(v['guides']) ? JSON.stringify(v['guides']) : (str('guidesJson') ?? '[]'),
		engineRange: str('engineRange') ?? '',
		keywords: Array.isArray(keywords) ? keywords.filter((k): k is string => typeof k === 'string') : [],
		tags: Array.isArray(tags) ? tags.filter((t): t is string => typeof t === 'string') : [],
		license: str('license') ?? 'MIT',
		requestId: str('requestId') ?? ''
	};
	if (Buffer.byteLength(frame.docJson, 'utf8') > 1024 * 1024) {
		return { ok: false, error: fail(413, 'payload exceeds size limits') };
	}
	return { ok: true, frame };
}
