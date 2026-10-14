// Single source of truth for the canonical production origin. Every
// hardcoded reference to the production host must import from here —
// never inline the string — so a domain move is a one-line change.
// See `docs/host.ts` for the reverse direction (rewriting canonical
// URLs to the current origin in preview deployments).
export const CANONICAL_ORIGIN = 'https://rasmalai.rovelstars.com';
export const CANONICAL_HOST = 'rasmalai.rovelstars.com';

export function canonicalUrl(pathname: string): string {
	return `${CANONICAL_ORIGIN}${pathname.startsWith('/') ? pathname : `/${pathname}`}`;
}
