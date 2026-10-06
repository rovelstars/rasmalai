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

export interface ChunkedTarball {
	units: ChunkUnit[];
	entries: TarEntryRef[];
}

export function parseTarEntries(tar: Uint8Array): TarEntry[] {
	const entries: TarEntry[] = [];
	const dec = new TextDecoder();
	let off = 0;
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
		if (typeflag !== 0 && typeflag !== 48) continue;
		const name = longName ?? (prefix ? `${prefix}/${rawName}` : rawName);
		longName = null;
		if (!name) continue;
		entries.push({ name, bytes: data });
	}
	return entries;
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

function cdcSplit(data: Uint8Array): Uint8Array[] {
	const out: Uint8Array[] = [];
	let start = 0;
	let hash = 0;
	for (let i = 0; i < data.length; i++) {
		hash = ((hash << 1) + GEAR[data[i]]) & 0xffffffff;
		const len = i - start + 1;
		if (len >= CDC_MIN && (hash & CDC_MASK) === 0) {
			out.push(data.slice(start, i + 1));
			start = i + 1;
			hash = 0;
		} else if (len >= CDC_MAX) {
			out.push(data.slice(start, i + 1));
			start = i + 1;
			hash = 0;
		}
	}
	if (start < data.length) out.push(data.slice(start));
	if (out.length === 0) out.push(data.slice());
	return out;
}

export async function chunkTarball(tar: Uint8Array): Promise<ChunkedTarball> {
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
	const parsed = parseTarEntriesWithDirs(tar);
	if (parsed.files === 0) {
		const chunks: string[] = [];
		for (const part of cdcSplit(tar)) chunks.push(await push(part));
		return { units, entries: [{ name: '', size: tar.length, dir: false, chunks }] };
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
	return { units, entries };
}

interface ParsedEntry {
	name: string;
	dir: boolean;
	bytes: Uint8Array;
}

function parseTarEntriesWithDirs(tar: Uint8Array): { entries: ParsedEntry[]; files: number } {
	const entries: ParsedEntry[] = [];
	const dec = new TextDecoder();
	let off = 0;
	let files = 0;
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
		if (typeflag === 76) continue;
		if (typeflag !== 0 && typeflag !== 48 && typeflag !== 53) continue;
		const name = prefix ? `${prefix}/${rawName}` : rawName;
		if (!name) continue;
		const dir = typeflag === 53;
		if (!dir) files++;
		entries.push({ name, dir, bytes: dir ? new Uint8Array(0) : data });
	}
	return { entries, files };
}

function writeOctal(buf: Uint8Array, off: number, len: number, value: number): void {
	const digits = value.toString(8);
	const pad = len - 1 - digits.length;
	for (let i = 0; i < pad; i++) buf[off + i] = 48;
	for (let i = 0; i < digits.length; i++) buf[off + pad + i] = digits.charCodeAt(i);
	buf[off + len - 1] = 0;
}

export function rebuildTarball(entries: TarEntryRef[], byHash: Map<string, Uint8Array>): Uint8Array {
	const parts: Uint8Array[] = [];
	const enc = new TextEncoder();
	const fallback = entries.length === 1 && entries[0].name === '' && !entries[0].dir;
	for (const e of entries) {
		if (e.name === '' && !e.dir) {
			for (const h of e.chunks) {
				const b = byHash.get(h);
				if (!b) throw new Error(`missing chunk ${h}`);
				parts.push(b);
			}
			continue;
		}
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
		parts.push(head);
		if (!e.dir) {
			let done = 0;
			for (const h of e.chunks) {
				const b = byHash.get(h);
				if (!b) throw new Error(`missing chunk ${h}`);
				parts.push(b);
				done += b.length;
			}
			if (done !== e.size) throw new Error(`size mismatch for ${e.name}`);
			const pad = (512 - (e.size % 512)) % 512;
			if (pad > 0) parts.push(new Uint8Array(pad));
		}
	}
	parts.push(new Uint8Array(fallback ? 0 : 1024));
	const total = parts.reduce((n, p) => n + p.length, 0);
	const out = new Uint8Array(total);
	let off = 0;
	for (const p of parts) {
		out.set(p, off);
		off += p.length;
	}
	return out;
}
