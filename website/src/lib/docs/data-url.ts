// Deploy-versioned docs data (scripts/version-data.mjs copies
// static/data/*.json to static/data/<sha>/ and writes
// static/data/version.json {"version": "<sha>"}). Versioned payloads
// cache immutably for a year; version.json is short-lived (60s), so a new
// deploy goes live with the pointer and needs no purge. The unversioned
// path is the fallback when the pointer is missing (local dev, old
// bundles) or a versioned file 404s mid-deploy.
const VERSION_RE = /^[0-9a-f]{8,64}$/;

let cached: string | null | undefined;
let inflight: Promise<string | null> | null = null;

export function dataVersion(fetchFn: typeof fetch = fetch): Promise<string | null> {
	if (cached !== undefined) return Promise.resolve(cached);
	inflight ??= (async () => {
		try {
			const res = await fetchFn('/data/version.json');
			if (!res.ok) return null;
			const data = (await res.json()) as { version?: unknown };
			return typeof data.version === 'string' && VERSION_RE.test(data.version)
				? data.version
				: null;
		} catch {
			return null;
		}
	})();
	return inflight.then((v) => {
		cached = v;
		inflight = null;
		return v;
	});
}

export async function fetchDataFile(
	fetchFn: typeof fetch,
	file: string
): Promise<Response | null> {
	const version = await dataVersion(fetchFn);
	if (version) {
		try {
			const res = await fetchFn(`/data/${version}/${file}`);
			if (res.ok) return res;
		} catch {
			/* fall through to the legacy path */
		}
	}
	try {
		const res = await fetchFn(`/data/${file}`);
		if (res.ok) return res;
	} catch {
		/* unavailable: callers render empty states */
	}
	return null;
}
