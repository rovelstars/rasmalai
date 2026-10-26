import { json } from '@sveltejs/kit';
import { SPEC_VERSION, CAPABILITIES, specHeaders } from '$lib/server/registry';
import { CANONICAL_HOST } from '$lib/site';

// A self-hosted registry answers with its own host here. SITE_URL is the
// public origin of the deployment (`https://registry.example.com`); only
// its host part is served, so a path or a trailing slash cannot leak in.
function registryHost(env: Record<string, string | undefined>): string {
	const site = (env['SITE_URL'] ?? '').trim();
	const host = site.replace(/^https?:\/\//i, '').split('/')[0].trim();
	if (/^[a-z0-9.-]+(?::\d+)?$/i.test(host)) return host.toLowerCase();
	return CANONICAL_HOST;
}

export async function GET({ platform, setHeaders }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	setHeaders({ 'Cache-Control': 'public, max-age=3600, s-maxage=3600', ...specHeaders() });
	return json(
		{
			spec: SPEC_VERSION,
			capabilities: CAPABILITIES,
			registry: registryHost(env)
		},
		{ headers: specHeaders() }
	);
}
