import { error, redirect } from '@sveltejs/kit';
import { dev } from '$app/environment';
import { getPackage, getVersionDoc } from '$lib/server/db';
import { splitNameVersion } from '$lib/server/registry';

// Registry-backed: every package and version resolves at request time,
// so new publishes go live without a site rebuild or a prerender pass.
//
// URLs are versioned with @: /packages/@std/fs serves latest,
// /packages/@std/fs@0.1.0 pins a version. Older path-versioned URLs
// (/packages/@std/fs/0.1.0) and the legacy ?v= form redirect here.
export const prerender = false;

const VERSION_RE = /^\d+\.\d+\.\d+$/;

export async function load({ platform, params, url, setHeaders }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const legacy = url.searchParams.get('v');
	const segs = params.slug.split('/').filter(Boolean);
	let full = '';
	let want: string | null = null;
	if (segs[0]?.startsWith('@')) {
		if (segs.length < 2) error(404, 'package not found');
		const nv = splitNameVersion(segs[1]);
		full = `${segs[0]}/${nv.name}`;
		want = nv.version;
		if (!want && segs.length > 2 && VERSION_RE.test(segs[2])) {
			error(404, 'package not found');
		}
	} else if (segs.length > 0) {
		const nv = splitNameVersion(segs[0]);
		full = nv.name;
		want = nv.version;
		if (!want && segs.length > 1 && VERSION_RE.test(segs[1])) {
			error(404, 'package not found');
		}
	} else {
		error(404, 'package not found');
	}
	if (legacy) {
		const tab = url.searchParams.get('tab');
		redirect(308, `/packages/${full}@${legacy}${tab ? `?tab=${tab}` : ''}`);
	}
	if (dev && full.startsWith('@std/')) {
		const { localStdPackage } = await import('$lib/server/std-local');
		const local = localStdPackage(full);
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
