<script lang="ts">
	import PackageDocPage from '$lib/components/PackageDocPage.svelte';

	let { data } = $props();
</script>

<PackageDocPage {data} />
