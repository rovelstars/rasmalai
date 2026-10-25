import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { initSync, format_source } from '../src/lib/wasm/wasm_playground.js';
import {
	PACKAGE_FORMAT_NOTICE,
	isFormattablePackagePath,
	selectPackageText
} from '../src/lib/packages-format.ts';

initSync(readFileSync(new URL('../src/lib/wasm/wasm_playground_bg.wasm', import.meta.url)));

describe('package file display formatting', () => {
	it('renders the minified chunk shape as multi-line source', () => {
		const minified = 'import{open}from"@std/sqlite";fn Main():Int{return 0;}';
		assert.ok(!minified.includes('\n'));
		const outcome = selectPackageText(minified, 'src/main.rnx', format_source(minified));
		assert.equal(outcome.formatted, true);
		assert.equal(outcome.notice, null);
		assert.notEqual(outcome.text, minified);
		assert.ok(outcome.text.includes('\n'));
		assert.ok(outcome.text.includes('import { open } from "@std/sqlite";'));
		const again = JSON.parse(format_source(outcome.text)) as { ok: boolean; output: string };
		assert.equal(again.output, outcome.text);
	});

	it('falls back to raw bytes when the formatter reports an error', () => {
		const raw = 'fn Main():Int{return 1;}';
		const failed = format_source('fn Main(): Int { let s = "unclosed; }');
		assert.ok(failed.includes('"ok":false'));
		const outcome = selectPackageText(raw, 'src/main.rnx', failed);
		assert.equal(outcome.text, raw);
		assert.equal(outcome.formatted, false);
		assert.equal(outcome.notice, PACKAGE_FORMAT_NOTICE);
	});

	it('falls back to raw bytes when the formatter never answers', () => {
		const raw = 'fn Main():Int{return 1;}';
		const outcome = selectPackageText(raw, 'src/main.rnx', null);
		assert.equal(outcome.text, raw);
		assert.equal(outcome.formatted, false);
		assert.equal(outcome.notice, PACKAGE_FORMAT_NOTICE);
	});

	it('falls back to raw bytes on malformed formatter output', () => {
		const raw = 'fn Main():Int{return 1;}';
		const outcome = selectPackageText(raw, 'src/main.rnx', 'not-json');
		assert.equal(outcome.text, raw);
		assert.equal(outcome.formatted, false);
		assert.equal(outcome.notice, PACKAGE_FORMAT_NOTICE);
	});

	it('leaves non-rnx files verbatim without a notice', () => {
		assert.equal(isFormattablePackagePath('LICENSE'), false);
		assert.equal(isFormattablePackagePath('src/main.rnx'), true);
		const outcome = selectPackageText('line one\n', 'LICENSE', '{"ok":true,"output":"other"}');
		assert.equal(outcome.text, 'line one\n');
		assert.equal(outcome.formatted, false);
		assert.equal(outcome.notice, null);
	});
});
