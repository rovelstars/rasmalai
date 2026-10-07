import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import assert from 'node:assert/strict';

const dir = dirname(fileURLToPath(import.meta.url));
const wasmDir = join(dir, '..', 'src', 'lib', 'wasm');
const glue = await import(join(wasmDir, 'wasm_playground.js'));
const bytes = await readFile(join(wasmDir, 'wasm_playground_bg.wasm'));
await glue.default(bytes);

const preludeSrc = await readFile(join(dir, '..', '..', 'compiler', 'stdlib', 'src', 'prelude.rnx'), 'utf8');
const provided = JSON.parse(glue.std_provide('prelude', preludeSrc));
assert.equal(provided.ok, true, JSON.stringify(provided));

const files = JSON.stringify({
	'main.rnx': 'import { double } from "./util";\nfn Main(): Int { return double(21); }\n',
	'util.rnx': 'export fn double(x: Int): Int { return x * 2; }\n'
});

const fmt = JSON.parse(glue.format_source('fn Main():Int{return 1;}'));
assert.equal(fmt.ok, true);
assert.ok(fmt.output.endsWith('\n'), fmt.output);

const comp = JSON.parse(glue.complete('fn Main(): Int { return 0; }\n', 1, 0));
assert.ok(Array.isArray(comp.items) && comp.items.length > 0);
assert.ok(comp.items.some((i) => i.label === 'fn'));

const hov = JSON.parse(glue.hover('fn Main(): Int { return 0; }\n', 0, 4));
assert.ok(hov.signature.includes('fn Main'), JSON.stringify(hov));

const dj = JSON.parse(glue.diagnostics_json('fn Main(): Int { return 0; }\n'));
assert.deepEqual(dj, []);

const bad = JSON.parse(glue.diagnostics_json('fn Main(: Int { return 1; }'));
assert.ok(bad.length > 0 && bad[0].code);

assert.equal(glue.check_project(files, 'main.rnx'), '');
const run = glue.run_project(files, 'main.rnx');
assert.ok(run.includes('=> 42'), run);

const tests = JSON.stringify({ 'main.rnx': 'fn Main(): Int { return 0; }\ntest fn check_math() { assert(1 + 1 == 2, "math"); }\n' });
const tout = glue.test_project(tests, 'main.rnx');
assert.ok(tout.includes('ok check_math') && tout.includes('1 passed, 0 failed'), tout);

const dp = JSON.parse(glue.diagnostics_project(files, 'main.rnx'));
assert.deepEqual(dp, []);

const miss = glue.check_project(JSON.stringify({ 'main.rnx': 'import { x } from "./nope";\nfn Main(): Int { return 0; }\n' }), 'main.rnx');
assert.ok(miss.includes('E108'), miss);

console.log('wasm smoke: format/complete/hover/diag/check_project/run_project/test_project/diag_project ok');
