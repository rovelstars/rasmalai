import { json } from '@sveltejs/kit';
import { SPEC_VERSION, CAPABILITIES, specHeaders } from '$lib/server/registry';
import { CANONICAL_HOST } from '$lib/site';

export async function GET({ setHeaders }) {
	setHeaders({ 'Cache-Control': 'public, max-age=3600, s-maxage=3600', ...specHeaders() });
	return json(
		{
			spec: SPEC_VERSION,
			capabilities: CAPABILITIES,
			registry: CANONICAL_HOST
		},
		{ headers: specHeaders() }
	);
}
