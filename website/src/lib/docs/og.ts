export const DEFAULT_OG_IMAGE = '/og-banner.png';

export type OgManifest = Record<string, string>;

export function ogImageKey(pathname: string): string | null {
	const segs = pathname.split('/').filter(Boolean);
	if (segs.length === 0) return 'home';
	if (segs[0] === 'guide' && segs[1]) return `guide/${segs[1]}`;
	if (segs[0] === 'manual' && segs[1]) return `manual/${segs[1]}`;
	if (segs[0] === 'docs' && segs[1] === '@std' && segs[2]) return `std/${segs[2]}`;
	if (segs[0] === 'packages' && segs[1]) {
		if (segs[1].startsWith('@') && segs[2]) return `pkg/${segs[1]}/${stripVersion(segs[2])}`;
		if (!segs[1].startsWith('@')) return `pkg/${stripVersion(segs[1])}`;
		return null;
	}
	return null;
}

function stripVersion(seg: string): string {
	const at = seg.lastIndexOf('@');
	if (at > 0) return seg.slice(0, at);
	return seg;
}

export function ogImageFor(pathname: string, manifest: OgManifest): string {
	const key = ogImageKey(pathname);
	if (key && manifest[key]) return `/${manifest[key]}`;
	return DEFAULT_OG_IMAGE;
}
