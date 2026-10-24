export interface ChunkUnit {
	hash: string;
	size: number;
	bytes: Uint8Array;
}

export interface TarEntryRef {
	name: string;
	size: number;
	dir: boolean;
	chunks: string[];
}

// One canonical file of a publish: the server derives these bytes itself,
// so identical source always yields identical bytes, identical hashes and
// identical stored objects no matter which client submitted it.
export interface CanonicalFile {
	name: string;
	bytes: Uint8Array;
}

export interface ChunkedPackage {
	units: ChunkUnit[];
	entries: TarEntryRef[];
}

const NAME_ENCODER = new TextEncoder();

// Accepted manifest entry names.
//
// This is the contract between the server and the Rust client
// (`compiler/frontend/src/fetch.rs::tar_safe_rel`). The two are written in
// different languages, so the rule is spelled out here and repeated there
// verbatim; the two implementations must stay textually identical.
//
// A manifest entry name is accepted when, after dropping at most one
// trailing `/`, all of the following hold:
//   1. it is not empty;
//   2. it holds no `\` (a separator on Windows, where joining it onto the
//      destination directory would escape that directory);
//   3. it does not start with `/`;
//   4. it carries no drive prefix (`C:`);
//   5. every `/`-separated component is non-empty and is neither `.` nor
//      `..`;
//   6. it holds no control character, NUL included;
//   7. it is at most 255 bytes long.
export const MAX_ENTRY_NAME_BYTES = 255;

export class UnsafeEntryNameError extends Error {
	readonly entryName: string;

	constructor(entryName: string, reason: string) {
		super(`unsafe entry name \`${entryName}\`: ${reason}`);
		this.entryName = entryName;
	}
}

// The reason a name is refused, or null when the name is accepted.
export function checkManifestEntryName(name: string): string | null {
	const trimmed = name.endsWith('/') ? name.slice(0, -1) : name;
	if (trimmed.length === 0) return 'the name is empty';
	if (NAME_ENCODER.encode(trimmed).length > MAX_ENTRY_NAME_BYTES) {
		return `the name is longer than ${MAX_ENTRY_NAME_BYTES} bytes`;
	}
	if (trimmed.includes('\\')) return 'the name holds a backslash';
	if (trimmed.startsWith('/')) return 'the name is absolute';
	if (/^[A-Za-z]:/.test(trimmed)) return 'the name carries a drive prefix';
	if (/[\u0000-\u001f\u007f-\u009f]/.test(trimmed)) return 'the name holds a control character';
	for (const part of trimmed.split('/')) {
		if (part.length === 0) return 'the name holds an empty path component';
		if (part === '.' || part === '..') return `the name holds a \`${part}\` path component`;
	}
	return null;
}

// The same rule applied to a whole manifest.
export function checkManifestEntries(entries: TarEntryRef[]): { name: string; reason: string } | null {
	for (const e of entries) {
		const reason = checkManifestEntryName(e.name);
		if (reason) return { name: e.name, reason };
	}
	return null;
}

export const LARGE_FILE_BYTES = 128 * 1024;
export const MAX_VIEW_BYTES = 256 * 1024;

export function decodeFileView(
	bytes: Uint8Array
): { ok: true; text: string } | { ok: false; code: 'too-large' | 'binary' } {
	if (bytes.length > MAX_VIEW_BYTES) return { ok: false, code: 'too-large' };
	try {
		return { ok: true, text: new TextDecoder('utf-8', { fatal: true }).decode(bytes) };
	} catch {
		return { ok: false, code: 'binary' };
	}
}
const CDC_MIN = 16 * 1024;
const CDC_MAX = 256 * 1024;
const CDC_MASK = 0xffff;

// The worst-case chunk count of a canonical file list. A file over the
// large-file threshold is split by content-defined chunking, and every
// part but the last is at least CDC_MIN bytes, so the split count is
// bounded before a single byte is hashed.
export function chunkCountBound(files: CanonicalFile[]): number {
	let total = 0;
	for (const f of files) {
		if (f.bytes.length > LARGE_FILE_BYTES) {
			total += Math.ceil(f.bytes.length / CDC_MIN);
		} else {
			total += 1;
		}
	}
	return total;
}

const GEAR: Uint32Array = (() => {
	const t = new Uint32Array(256);
	let s = 0x9e3779b9;
	for (let i = 0; i < 256; i++) {
		s = (s * 1103515245 + 12345) & 0xffffffff;
		t[i] = s;
	}
	return t;
})();

export async function sha256Hex(data: Uint8Array): Promise<string> {
	const digest = await crypto.subtle.digest('SHA-256', data as BufferSource);
	return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

// Content-defined chunking. The rolling hash is updated byte by byte, but
// the boundary is checked on word boundaries: four bytes of hash per step
// and one decision per step, instead of a decision per byte.
function cdcSplit(data: Uint8Array): Uint8Array[] {
	const out: Uint8Array[] = [];
	let start = 0;
	let hash = 0;
	let i = 0;
	const n = data.length;
	for (; i + 4 <= n; i += 4) {
		hash = ((hash << 1) + GEAR[data[i]]) & 0xffffffff;
		hash = ((hash << 1) + GEAR[data[i + 1]]) & 0xffffffff;
		hash = ((hash << 1) + GEAR[data[i + 2]]) & 0xffffffff;
		hash = ((hash << 1) + GEAR[data[i + 3]]) & 0xffffffff;
		const len = i + 4 - start;
		if ((len >= CDC_MIN && (hash & CDC_MASK) === 0) || len >= CDC_MAX) {
			out.push(data.slice(start, i + 4));
			start = i + 4;
			hash = 0;
		}
	}
	for (; i < n; i++) {
		hash = ((hash << 1) + GEAR[data[i]]) & 0xffffffff;
	}
	if (start < n) out.push(data.slice(start));
	if (out.length === 0) out.push(data.slice());
	return out;
}

export async function chunkCanonicalFiles(files: CanonicalFile[]): Promise<ChunkedPackage> {
	const units: ChunkUnit[] = [];
	const byHash = new Map<string, ChunkUnit>();
	const entries: TarEntryRef[] = [];
	const push = async (bytes: Uint8Array): Promise<string> => {
		const hash = await sha256Hex(bytes);
		if (!byHash.has(hash)) {
			const unit = { hash, size: bytes.length, bytes };
			byHash.set(hash, unit);
			units.push(unit);
		}
		return hash;
	};
	for (const f of files) {
		const chunks: string[] = [];
		if (f.bytes.length > LARGE_FILE_BYTES) {
			for (const part of cdcSplit(f.bytes)) chunks.push(await push(part));
		} else if (f.bytes.length > 0) {
			chunks.push(await push(f.bytes));
		}
		entries.push({ name: f.name, size: f.bytes.length, dir: false, chunks });
	}
	const unsafe = checkManifestEntries(entries);
	if (unsafe) throw new UnsafeEntryNameError(unsafe.name, unsafe.reason);
	return { units, entries };
}

// The version identity: sha256 over the canonical entry bytes, one entry
// after the other in stored order. Both the Rust client and this server
// implement these bytes, and the golden vectors in the test suites pin
// them byte-identical.
export function manifestDigestBytes(entries: TarEntryRef[]): Uint8Array {
	const enc = new TextEncoder();
	let total = 0;
	const parts: Uint8Array[] = [];
	for (const e of entries) {
		const name = enc.encode(e.name);
		const head = new Uint8Array(4 + name.length + 8 + 1 + 4 + e.chunks.length * 32);
		const view = new DataView(head.buffer);
		view.setUint32(0, name.length, true);
		head.set(name, 4);
		let off = 4 + name.length;
		let size = BigInt(e.size);
		for (let i = 0; i < 8; i++) {
			head[off + i] = Number(size & 0xffn);
			size >>= 8n;
		}
		off += 8;
		head[off] = e.dir ? 1 : 0;
		off += 1;
		view.setUint32(off, e.chunks.length, true);
		off += 4;
		for (const h of e.chunks) {
			for (let i = 0; i < 32; i++) {
				head[off++] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
			}
		}
		parts.push(head);
		total += head.length;
	}
	const out = new Uint8Array(total);
	let at = 0;
	for (const p of parts) {
		out.set(p, at);
		at += p.length;
	}
	return out;
}

export async function manifestDigest(entries: TarEntryRef[]): Promise<string> {
	return sha256Hex(manifestDigestBytes(entries));
}

// One sha256 over a byte stream, in a single call. Hashing each chunk in
// its own `crypto.subtle` call pays per-call overhead per chunk; the same
// bytes in one call cost a fraction of it.
export async function sha256HexConcat(parts: Uint8Array[]): Promise<string> {
	let total = 0;
	for (const p of parts) total += p.length;
	const merged = new Uint8Array(total);
	let off = 0;
	for (const p of parts) {
		merged.set(p, off);
		off += p.length;
	}
	return sha256Hex(merged);
}
