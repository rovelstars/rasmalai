const origin = (process.argv[2] ?? 'http://localhost:5173').replace(/\/$/, '');
let failures = 0;

function check(name, cond, detail = '') {
	if (cond) console.log(`ok: ${name}`);
	else {
		failures++;
		console.error(`FAIL: ${name}${detail ? ` (${detail})` : ''}`);
	}
}

const version = await fetch(`${origin}/api/version`);
check('discovery answers 200', version.status === 200, `HTTP ${version.status}`);
check(
	'discovery carries the spec header',
	version.headers.get('rnx-registry-spec') === '0',
	version.headers.get('rnx-registry-spec')
);
const discovery = await version.json().catch(() => null);
check('discovery body has spec 0', discovery?.spec === 0, JSON.stringify(discovery));
for (const cap of ['tombstones', 'guides', 'jsdoc', 'yank', 'transfer']) {
	check(`capability advertised: ${cap}`, (discovery?.capabilities ?? []).includes(cap));
}
check(
	'discovery names this deployment host',
	discovery?.registry === new URL(origin).host,
	String(discovery?.registry)
);
check(
	'discovery is cached for an hour',
	(version.headers.get('cache-control') ?? '').includes('max-age=3600'),
	version.headers.get('cache-control')
);

const missing = await fetch(`${origin}/api/v0/packages/@no-such-scope/no-such-pkg@9.9.9`);
check('unknown version answers 404', missing.status === 404, `HTTP ${missing.status}`);
const missingBody = await missing.json().catch(() => null);
check('unknown version names the package', (missingBody?.message ?? '').includes('does not exist'), JSON.stringify(missingBody));

const resolve = await fetch(`${origin}/api/v0/resolve`, {
	method: 'POST',
	headers: { 'content-type': 'application/json' },
	body: JSON.stringify({ requirements: { '@no-such-scope/no-such-pkg': '*' } })
});
check('resolve answers 404 for an unknown package', resolve.status === 404, `HTTP ${resolve.status}`);

const chunk = await fetch(`${origin}/api/v0/packages/@no-such-scope/no-such-pkg@9.9.9/chunks`);
check('chunk manifest of an unknown version answers 404', chunk.status === 404, `HTTP ${chunk.status}`);

if (failures > 0) {
	console.error(`${failures} check(s) failed`);
	process.exit(1);
}
console.log('deployment looks conforming (anonymous reads only; no mutations performed)');
