#!/usr/bin/env node
// Measures POST /api/resolve against the LIVE registry, interleaving the
// payload variants so CDN/edge drift cannot bias one arm.
//
//   usage: node measure-resolve.mjs [runs]
//
// The live server is whatever is deployed; this script measures what a client
// actually pays. Run `rnx` against a local build to compare the resolver.
const RUNS = Number(process.argv[2] || 5);
const BASE = process.env.REGISTRY_BASE || 'https://rasmalai.rovelstars.com';

// The 17 @std packages, as the live catalog publishes them.
const PKGS = [
	'@std/web', '@std/time', '@std/testing', '@std/sync', '@std/simd', '@std/random',
	'@std/process', '@std/prelude', '@std/os', '@std/net', '@std/math', '@std/json',
	'@std/io', '@std/fs', '@std/env', '@std/collections', '@std/bytes'
];

function body(withChunks) {
	return JSON.stringify({
		requirements: Object.fromEntries(PKGS.map((p) => [p, '*'])),
		have: [],
		...(withChunks ? { chunks: true } : {})
	});
}

async function one(withChunks) {
	const started = process.hrtime.bigint();
	const res = await fetch(`${BASE}/api/resolve`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: body(withChunks)
	});
	const text = await res.text();
	const ms = Number(process.hrtime.bigint() - started) / 1e6;
	let nodes = 0;
	let levels = 0;
	let inlineEntries = 0;
	try {
		const parsed = JSON.parse(text);
		levels = Array.isArray(parsed.levels) ? parsed.levels.length : 0;
		for (const level of parsed.levels ?? []) {
			for (const node of level) {
				nodes += 1;
				if (Array.isArray(node.entries)) inlineEntries += 1;
			}
		}
	} catch {
		/* error bodies are reported via status */
	}
	return { status: res.status, ms, bytes: Buffer.byteLength(text), nodes, levels, inlineEntries };
}

function median(xs) {
	const s = [...xs].sort((a, b) => a - b);
	return s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2;
}

const arms = { plain: [], inline: [] };
for (let i = 0; i < RUNS; i++) {
	// Interleave: alternate order each round so neither arm always sits in the
	// same slot relative to edge-cache warming.
	for (const [name, wc] of i % 2 === 0 ? [['plain', false], ['inline', true]] : [['inline', true], ['plain', false]]) {
		try {
			const r = await one(wc);
			if (r.status !== 200) {
				console.log(`  ${name} run ${i + 1}: HTTP ${r.status}`);
				continue;
			}
			arms[name].push(r);
			console.log(
				`  ${name.padEnd(6)} run ${i + 1}: ${r.ms.toFixed(0)} ms  ${r.bytes} B  ` +
					`levels=${r.levels} nodes=${r.nodes} inlineNodes=${r.inlineEntries}`
			);
		} catch (e) {
			console.log(`  ${name} run ${i + 1}: ERROR ${e.message}`);
		}
	}
}

console.log('\n=== summary (median) ===');
for (const [name, rs] of Object.entries(arms)) {
	if (rs.length === 0) continue;
	console.log(
		`${name.padEnd(7)} n=${rs.length}  ${median(rs.map((r) => r.ms)).toFixed(0)} ms  ` +
			`${median(rs.map((r) => r.bytes))} B  levels=${rs[0].levels} nodes=${rs[0].nodes} inlineNodes=${rs[0].inlineEntries}`
	);
}