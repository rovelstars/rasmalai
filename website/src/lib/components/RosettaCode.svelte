<script lang="ts">
	import RunnableCode from '$lib/components/RunnableCode.svelte';
	import { getDialect } from '$lib/stores/dialect.svelte';

	let {
		rnx,
		lang,
		other
	}: {
		rnx: string;
		lang: string;
		other: string;
	} = $props();

	function langKey(): string {
		return lang.toLowerCase().replace(/[^a-z]/g, '');
	}

	let startTab = $derived(getDialect() === langKey() ? 1 : 0);
</script>

<div class="doc-code">
	<RunnableCode code={rnx} lang="rnx" tabs={[{ label: lang, code: other, lang }]} {startTab} />
</div>
