export interface ChunkUnit {
	hash: string;
	size: number;
	bytes: Uint8Array;
}

export interface TarEntry {
	name: string;
	bytes: Uint8Array;
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

export function splitTarFiles(tar: Uint8Array): Uint8Array[] {
	const files: Uint8Array[] = [];
	let off = 0;
	while (off + 512 <= tar.length) {
		const header = tar.slice(off, off + 512);
		let empty = true;
		for (let i = 0; i < 512; i++) {
			if (header[i] !== 0) {
				empty = false;
				break;
			}
		}
		if (empty) break;
		const sizeField = String.fromCharCode(...header.slice(124, 136)).replace(/\0/g, '').trim();
		const size = parseInt(sizeField, 8);
		if (!Number.isFinite(size) || size < 0) break;
		const typeflag = header[156];
		off += 512;
		if (typeflag === 53) continue;
		if (off + size > tar.length) break;
		if (size > 0) files.push(tar.slice(off, off + size));
		off += Math.ceil(size / 512) * 512;
	}
	return files;
}

export async function chunkTarball(tar: Uint8Array): Promise<ChunkUnit[]> {
	const units: ChunkUnit[] = [];
	const push = async (bytes: Uint8Array) => {
		units.push({ hash: await sha256Hex(bytes), size: bytes.length, bytes });
	};
	const files = splitTarFiles(tar);
	if (files.length === 0) {
		for (const part of cdcSplit(tar)) await push(part);
		return units;
	}
	for (const file of files) {
		if (file.length > LARGE_FILE_BYTES) {
			for (const part of cdcSplit(file)) await push(part);
		} else {
			await push(file);
		}
	}
	return units;
}
