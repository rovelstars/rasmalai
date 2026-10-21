export interface ChunkUnit {
	hash: string;
	size: number;
	bytes: Uint8Array;
}

export interface TarEntry {
	name: string;
	bytes: Uint8Array;
}

export interface TarEntryRef {
	name: string;
	size: number;
	dir: boolean;
	chunks: string[];
}

// One parsed entry of an uploaded tarball.
export interface TarFile {
	name: string;
	dir: boolean;
	bytes: Uint8Array;
}

// The parsed tarball. The publish path parses the upload once and reuses
// the entries for the name check, the capability scan and the chunking.
export interface ParsedTarball {
	entries: TarFile[];
	files: number;
	raw: Uint8Array;
}

export interface ChunkedTarball {
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
//
// The one name that is not a path is the protocol sentinel: a manifest of
// exactly one entry whose name is the empty string and which is not a
// directory means "this package is one opaque tarball blob".
export const MAX_ENTRY_NAME_BYTES = 255;

export class UnsafeEntryNameError extends Error {
	readonly entryName: string;

	constructor(entryName: string, reason: string) {
		super(`unsafe tar entry name \`${entryName}\`: ${reason}`);
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

// The same rule applied to a whole manifest. The empty name is the
// sentinel, so it is accepted only as the single entry of the manifest.
export function checkManifestEntries(entries: TarEntryRef[]): { name: string; reason: string } | null {
	const sentinel = entries.length === 1 && entries[0].name === '' && !entries[0].dir;
	for (const e of entries) {
		if (e.name === '' && !e.dir) {
			if (!sentinel) {
				return { name: e.name, reason: 'the empty name is reserved for the opaque tarball sentinel' };
			}
			continue;
		}
		const reason = checkManifestEntryName(e.name);
		if (reason) return { name: e.name, reason };
	}
	return null;
}

// The tar parser. One pass over the upload produces every entry the
// publish path needs: the name check, the capability scan and the chunking
// all read this result instead of parsing the bytes again.
export function parseTarball(tar: Uint8Array): ParsedTarball {
	const entries: TarFile[] = [];
	const dec = new TextDecoder();
	let off = 0;
	let files = 0;
	let longName: string | null = null;
	while (off + 512 <= tar.length) {
		const header = tar.subarray(off, off + 512);
		let empty = true;
		for (let i = 0; i < 512; i++) {
			if (header[i] !== 0) {
				empty = false;
				break;
			}
		}
		if (empty) break;
		const rawName = dec.decode(header.subarray(0, 100)).replace(/\0.*$/, '');
		const prefix = dec.decode(header.subarray(345, 500)).replace(/\0.*$/, '');
		const sizeField = String.fromCharCode(...header.subarray(124, 136))
			.replace(/\0/g, '')
			.trim();
		const size = parseInt(sizeField, 8);
		if (!Number.isFinite(size) || size < 0) break;
		const typeflag = header[156];
		off += 512;
		if (off + size > tar.length) break;
		const data = tar.slice(off, off + size);
		off += Math.ceil(size / 512) * 512;
		if (typeflag === 76) {
			longName = dec.decode(data).replace(/\0.*$/, '');
			continue;
		}
		if (typeflag !== 0 && typeflag !== 48 && typeflag !== 53) continue;
		const name = longName ?? (prefix ? `${prefix}/${rawName}` : rawName);
		longName = null;
		if (!name) continue;
		const dir = typeflag === 53;
		if (!dir) files++;
		entries.push({ name, dir, bytes: dir ? new Uint8Array(0) : data });
	}
	return { entries, files, raw: tar };
}

// The files of a tarball, without the directory entries.
export function parseTarEntries(tar: Uint8Array): TarEntry[] {
	const out: TarEntry[] = [];
	for (const e of parseTarball(tar).entries) {
		if (e.dir) continue;
		out.push({ name: e.name, bytes: e.bytes });
	}
	return out;
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

// The chunk count of a package is bounded. Every chunk is one R2 object
// and one row, and the Free plan allows 50 external subrequests per
// invocation, so a package that needs more chunks than that cannot be
// stored on Free at all. The cap is checked before the chunking work
// starts, from the worst case the splitter can produce.
export const MAX_CHUNKS = 32;

export class TooManyChunksError extends Error {
	constructor(count: number) {
		super(`package needs ${count} chunks, over the ${MAX_CHUNKS} chunk limit`);
	}
}

// The worst-case chunk count of a tarball. A file over the large-file
// threshold is split by content-defined chunking, and every part but the
// last is at least CDC_MIN bytes, so the split count is bounded before a
// single byte is hashed.
export function chunkCountBound(parsed: ParsedTarball): number {
	let files = 0;
	let total = 0;
	for (const e of parsed.entries) {
		if (e.dir || e.bytes.length === 0) continue;
		files++;
		if (e.bytes.length > LARGE_FILE_BYTES) {
			total += Math.ceil(e.bytes.length / CDC_MIN);
		} else {
			total += 1;
		}
	}
	// A tarball with no file entry is one opaque blob, split by the same
	// rule over the whole upload.
	if (files === 0) return Math.max(1, Math.ceil(parsed.raw.length / CDC_MIN));
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

export async function chunkTarball(source: Uint8Array | ParsedTarball): Promise<ChunkedTarball> {
	const parsed = source instanceof Uint8Array ? parseTarball(source) : source;
	const bound = chunkCountBound(parsed);
	if (bound > MAX_CHUNKS) throw new TooManyChunksError(bound);
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
	if (parsed.files === 0) {
		const chunks: string[] = [];
		for (const part of cdcSplit(parsed.raw)) chunks.push(await push(part));
		return { units, entries: [{ name: '', size: parsed.raw.length, dir: false, chunks }] };
	}
	for (const f of parsed.entries) {
		if (f.dir) {
			entries.push({ name: f.name, size: 0, dir: true, chunks: [] });
			continue;
		}
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

// The entry names of a tarball, checked against the accepted-name rule
// before the tarball is accepted. The publish path calls this first, so an
// unsafe name is refused before anything is written.
export function validateTarballEntryNames(
	parsed: ParsedTarball
): { ok: true } | { ok: false; name: string; reason: string } {
	for (const e of parsed.entries) {
		const reason = checkManifestEntryName(e.name);
		if (reason) return { ok: false, name: e.name, reason };
	}
	return { ok: true };
}

function writeOctal(buf: Uint8Array, off: number, len: number, value: number): void {
	const digits = value.toString(8);
	const pad = len - 1 - digits.length;
	for (let i = 0; i < pad; i++) buf[off + i] = 48;
	for (let i = 0; i < digits.length; i++) buf[off + pad + i] = digits.charCodeAt(i);
	buf[off + len - 1] = 0;
}

function tarHeader(e: TarEntryRef): Uint8Array {
	const enc = new TextEncoder();
	const head = new Uint8Array(512);
	const nameBytes = enc.encode(e.name);
	head.set(nameBytes.subarray(0, Math.min(nameBytes.length, 100)));
	writeOctal(head, 100, 8, e.dir ? 0o755 : 0o644);
	writeOctal(head, 108, 8, 0);
	writeOctal(head, 116, 8, 0);
	writeOctal(head, 124, 12, e.size);
	writeOctal(head, 136, 12, 0);
	for (let i = 148; i < 156; i++) head[i] = 32;
	head[156] = e.dir ? 53 : 48;
	head.set(enc.encode('ustar\0'), 257);
	head.set(enc.encode('00'), 263);
	head.set(enc.encode('rnx'), 265);
	head.set(enc.encode('rnx'), 297);
	let sum = 0;
	for (let i = 0; i < 512; i++) sum += head[i];
	const sumText = sum.toString(8).padStart(6, '0');
	for (let i = 0; i < 6; i++) head[148 + i] = sumText.charCodeAt(i);
	head[154] = 0;
	head[155] = 32;
	return head;
}

// The canonical rebuild, as a stream of parts. The round-trip check walks
// the same parts the rebuild does, so the rebuild is never materialized a
// second time just to be compared.
function* rebuildParts(
	entries: TarEntryRef[],
	byHash: Map<string, Uint8Array>
): Generator<Uint8Array> {
	const fallback = entries.length === 1 && entries[0].name === '' && !entries[0].dir;
	for (const e of entries) {
		if (e.name === '' && !e.dir) {
			for (const h of e.chunks) {
				const b = byHash.get(h);
				if (!b) throw new Error(`missing chunk ${h}`);
				yield b;
			}
			continue;
		}
		yield tarHeader(e);
		if (!e.dir) {
			let done = 0;
			for (const h of e.chunks) {
				const b = byHash.get(h);
				if (!b) throw new Error(`missing chunk ${h}`);
				yield b;
				done += b.length;
			}
			if (done !== e.size) throw new Error(`size mismatch for ${e.name}`);
			const pad = (512 - (e.size % 512)) % 512;
			if (pad > 0) yield new Uint8Array(pad);
		}
	}
	if (!fallback) yield new Uint8Array(1024);
}

export function rebuildTarball(entries: TarEntryRef[], byHash: Map<string, Uint8Array>): Uint8Array {
	const parts = [...rebuildParts(entries, byHash)];
	const total = parts.reduce((n, p) => n + p.length, 0);
	const out = new Uint8Array(total);
	let off = 0;
	for (const p of parts) {
		out.set(p, off);
		off += p.length;
	}
	return out;
}

// The round-trip check: the canonical rebuild of the stored chunks,
// compared against the uploaded bytes part by part. A byte-for-byte match
// is the same check the sha256 comparison did, and a stricter one.
export function rebuildMatchesTarball(
	entries: TarEntryRef[],
	byHash: Map<string, Uint8Array>,
	original: Uint8Array
): boolean {
	let off = 0;
	for (const part of rebuildParts(entries, byHash)) {
		if (off + part.length > original.length) return false;
		const slice = original.subarray(off, off + part.length);
		for (let i = 0; i < part.length; i++) {
			if (slice[i] !== part[i]) return false;
		}
		off += part.length;
	}
	return off === original.length;
}
