<svelte:head>
	<title>License: MIT or Apache-2.0 | Rasmalai</title>
	<meta
		name="description"
		content="Rasmalai is dual-licensed under MIT or Apache-2.0, at your option: use it commercially, modify it, and ship it closed source."
	/>
</svelte:head>

<main class="shell pb-24 pt-14">
	<header>
		<p class="font-mono text-xs text-aura-faint">SPDX-Identifier: MIT OR Apache-2.0</p>
		<h1 class="display-s mt-4 max-w-[18ch]">MIT or Apache-2.0</h1>
		<p class="lede mt-5">
			The compiler, the standard library, and the toolchain are dual-licensed under MIT or
			Apache-2.0, at your option. Take either one. Ship it in a product, fork it, sell support
			around it. MIT asks you to keep the copyright line and the permission notice; Apache-2.0
			keeps those and adds an express patent grant. The full Apache text is in the repository
			LICENSE.
		</p>
	</header>

	<div class="lic-grid">
		<dl class="lic-meta">
			<div>
				<dt>Permissions</dt>
				<dd>Commercial use, modification, distribution, private use</dd>
			</div>
			<div>
				<dt>Conditions</dt>
				<dd>
					License and copyright notice in copies; Apache-2.0 also asks that modified files be
					marked as changed
				</dd>
			</div>
			<div>
				<dt>Limitations</dt>
				<dd>No warranty, no liability</dd>
			</div>
			<div>
				<dt>Patent grant</dt>
				<dd>Apache-2.0 carries an express patent grant from contributors; MIT has none</dd>
			</div>
		</dl>

		<div class="lic-body">
			<p>MIT License</p>
			<p>Copyright (c) Rasmalai contributors</p>
			<p>
				Permission is hereby granted, free of charge, to any person obtaining a copy of this
				software and associated documentation files (the "Software"), to deal in the Software
				without restriction, including without limitation the rights to use, copy, modify,
				merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
				permit persons to whom the Software is furnished to do so, subject to the following
				conditions:
			</p>
			<p>
				The above copyright notice and this permission notice shall be included in all copies
				or substantial portions of the Software.
			</p>
			<p>
				THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
				INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
				PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
				HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF
				CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE
				OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
			</p>

			<section class="lic-apache">
				<p class="lic-apache-title">Apache-2.0</p>
				<p>
					Apache License, Version 2.0 is the second option, chosen instead of MIT. It grants
					the same permissions and adds an express patent grant from every contributor. It
					also asks that copies keep the license and notices and that modified files be
					marked as changed. Read the full text in the repository:
					<a href="https://github.com/rovelstars/rasmalai/blob/main/LICENSE">LICENSE</a>.
				</p>
			</section>
		</div>
	</div>
</main>

<style>
	.lic-grid {
		display: grid;
		gap: var(--space-xl) clamp(32px, 6vw, 88px);
		margin-top: var(--space-2xl);
		border-top: 1px solid var(--color-rule);
		padding-top: var(--space-lg);
	}

	@media (min-width: 860px) {
		.lic-grid {
			grid-template-columns: 220px minmax(0, 1fr);
		}
	}

	.lic-meta div + div {
		margin-top: var(--space-md);
	}

	.lic-meta dt {
		font-family: var(--font-mono);
		font-size: var(--text-2xs);
		color: var(--color-faint);
	}

	.lic-meta dd {
		margin-top: 2px;
		font-size: var(--text-sm);
		line-height: 1.5;
		color: var(--color-muted);
	}

	.lic-body {
		max-width: var(--measure);
		font-size: var(--text-md);
		line-height: 1.72;
		color: var(--color-muted);
	}

	.lic-body p + p {
		margin-top: var(--space-md);
	}

	.lic-body p:nth-child(-n + 2) {
		color: var(--color-ink);
	}

	.lic-body > p:last-of-type {
		margin-top: var(--space-lg);
		border-top: 1px solid var(--color-rule);
		padding-top: var(--space-md);
		font-size: var(--text-sm);
		line-height: 1.7;
		color: var(--color-muted);
	}

	.lic-apache {
		margin-top: var(--space-lg);
		border-top: 1px solid var(--color-rule);
		padding-top: var(--space-md);
	}

	.lic-apache p + p {
		margin-top: var(--space-sm);
	}

	.lic-apache-title {
		color: var(--color-ink);
	}

	.lic-body a {
		color: var(--color-ink);
		text-decoration: underline;
		text-decoration-color: var(--color-accent);
		text-underline-offset: 2px;
	}
</style>
