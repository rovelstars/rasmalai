import { error, redirect } from '@sveltejs/kit';
import { dev } from '$app/environment';
import { getPackage, getVersionDoc } from '$lib/server/db';
import { localStdPackage } from '$lib/server/std-local';

// Registry-backed: every package and version resolves at request time,
// so new publishes go live without a site rebuild or a prerender pass.
//
// URLs are path-versioned: /packages/@std/fs serves latest,
// /packages/@std/fs/0.1.0 pins a version. The legacy ?v= form redirects
// to the path form.
export const prerender = false;

const VERSION_RE = /^\d+\.\d+\.\d+$/;

export async function load({ platform, params, url, setHeaders }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const legacy = url.searchParams.get('v');
	const segs = params.slug.split('/').filter(Boolean);
	let full = segs.join('/');
	let want: string | null = null;
	if (segs.length > 1 && VERSION_RE.test(segs[segs.length - 1])) {
		want = segs.pop() as string;
		full = segs.join('/');
	}
	if (legacy) {
		const tab = url.searchParams.get('tab');
		redirect(308, `/packages/${full}/${legacy}${tab ? `?tab=${tab}` : ''}`);
	}
	if (dev && full.startsWith('@std/')) {
		const local = localStdPackage(full, want);
		if (!local) error(404, 'package not found');
		setHeaders({ 'Cache-Control': 'no-store' });
		return { pkg: local.pkg, active: local.doc };
	}
	const pkg = await getPackage(env, full);
	if (!pkg) error(404, 'package not found');
	const meta = pkg.versions.find((v) => v.version === want) ?? pkg.versions[0];
	if (want && meta.version !== want) error(404, 'version not found');
	const doc = await getVersionDoc(env, pkg.name, meta.version);
	if (!doc) error(404, 'version not found');
	if (meta.version === pkg.versions[0].version) {
		setHeaders({ 'Cache-Control': 'public, max-age=300, s-maxage=300' });
	} else {
		// Pinned content is immutable, but version status (yank) can
		// change — cache a day, not a year.
		setHeaders({ 'Cache-Control': 'public, max-age=86400, s-maxage=86400' });
	}
	return { pkg, active: doc };
}
