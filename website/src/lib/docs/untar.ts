export interface UntarredFile {
	path: string;
	size: number;
	bytes: Uint8Array;
}

function parseOctal(bytes: Uint8Array): number {
	let text = '';
	for (const b of bytes) {
		if (b === 0) break;
		text += String.fromCharCode(b);
	}
	text = text.trim();
	if (!/^[0-7]+$/.test(text)) return NaN;
	return parseInt(text, 8);
}

function decodeName(bytes: Uint8Array): string {
	let end = bytes.indexOf(0);
	if (end < 0) end = bytes.length;
	return new TextDecoder().decode(bytes.subarray(0, end));
}

export function untar(tar: Uint8Array): UntarredFile[] {
	const out: UntarredFile[] = [];
	let off = 0;
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
		const rawName = decodeName(header.subarray(0, 100));
		const prefix = decodeName(header.subarray(345, 500));
		const size = parseOctal(header.subarray(124, 136));
		if (!Number.isFinite(size) || size < 0) break;
		const typeflag = header[156];
		off += 512;
		if (off + size > tar.length) break;
		const data = tar.slice(off, off + size);
		off += Math.ceil(size / 512) * 512;
		if (typeflag === 53) continue;
		if (typeflag !== 0 && typeflag !== 48) continue;
		const name = prefix ? `${prefix}/${rawName}` : rawName;
		if (!name || name.includes('..')) continue;
		out.push({ path: name, size, bytes: data });
	}
	return out;
}

export async function gunzip(data: Uint8Array): Promise<Uint8Array> {
	const stream = new Response(data as BodyInit).body;
	if (!stream) throw new Error('empty response body');
	const out = await new Response(stream.pipeThrough(new DecompressionStream('gzip'))).arrayBuffer();
	return new Uint8Array(out);
}

export function isGzip(data: Uint8Array): boolean {
	return data.length >= 2 && data[0] === 0x1f && data[1] === 0x8b;
}
