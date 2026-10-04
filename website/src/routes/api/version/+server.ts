import { json } from '@sveltejs/kit';
import { SPEC_VERSION, CAPABILITIES, specHeaders } from '$lib/server/registry';

export async function GET({ setHeaders }) {
	setHeaders({ 'Cache-Control': 'public, max-age=3600, s-maxage=3600', ...specHeaders() });
	return json(
		{
			spec: SPEC_VERSION,
			capabilities: CAPABILITIES,
			registry: 'rasmalai.rovelstars.com'
		},
		{ headers: specHeaders() }
	);
}
