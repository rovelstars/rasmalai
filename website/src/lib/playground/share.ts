export function encodeSnippet(code: string): string {
	const bytes = new TextEncoder().encode(code);
	let bin = '';
	for (const b of bytes) bin += String.fromCharCode(b);
	return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function decodeSnippet(hash: string): string | null {
	try {
		let s = hash.replace(/^#/, '').replace(/-/g, '+').replace(/_/g, '/');
		while (s.length % 4 !== 0) s += '=';
		const bin = atob(s);
		const bytes = new Uint8Array(bin.length);
		for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
		return new TextDecoder().decode(bytes);
	} catch {
		return null;
	}
}
