<script lang="ts">
	import { File, Folder, FolderOpen, Plus, Trash2, Pencil, Play } from 'lucide-svelte';
	import { sortedFileList } from '$lib/playground/vfs';
	import { addFile, moveProjectFile, removeProjectFile, setProjectEntry, switchFile, allRnxFiles, type IdeProject } from '$lib/playground/project-model';

	let {
		project,
		onChanged
	}: {
		project: IdeProject;
		onChanged: () => void;
	} = $props();

	let newName = $state('');
	let renaming: string | null = $state(null);
	let renameValue = $state('');
	let collapsed = $state(new Set<string>());

	let entries = $derived(sortedFileList(project.root));

	function depthOf(path: string): number {
		return path.split('/').filter(Boolean).length - 1;
	}

	function toggleDir(path: string): void {
		const dir = path.endsWith('/') ? path.slice(0, -1) : path;
		if (collapsed.has(dir)) collapsed.delete(dir);
		else collapsed.add(dir);
		onChanged();
	}

	function hiddenByCollapse(path: string): boolean {
		const parts = path.split('/').filter(Boolean);
		let at = '';
		for (let i = 0; i < parts.length - 1; i++) {
			at += `/${parts[i]}`;
			if (collapsed.has(at)) return true;
		}
		return false;
	}

	function isDir(path: string): boolean {
		return path.endsWith('/');
	}

	function select(path: string): void {
		if (isDir(path)) {
			toggleDir(path);
			return;
		}
		if (switchFile(project, path)) onChanged();
	}

	function create(): void {
		const name = newName.trim();
		if (!name) return;
		const r = addFile(project, '/', name.startsWith('/') ? name : `/${name}`);
		newName = '';
		if (!r.ok) {
			newName = name;
		}
		onChanged();
	}

	function startRename(path: string): void {
		renaming = path;
		renameValue = path.split('/').pop() ?? '';
	}

	function commitRename(path: string): void {
		const clean = isDir(path) ? path.slice(0, -1) : path;
		const dir = clean.split('/').slice(0, -1).join('/') || '/';
		const target = renameValue.trim();
		if (target && target !== clean.split('/').pop()) {
			moveProjectFile(project, clean, target.includes('/') ? target : `${dir === '/' ? '' : dir}/${target}`, '/');
		}
		renaming = null;
		onChanged();
	}

	function remove(path: string): void {
		const clean = isDir(path) ? path.slice(0, -1) : path;
		removeProjectFile(project, clean, true);
		onChanged();
	}

	function makeEntry(path: string): void {
		setProjectEntry(project, path);
		onChanged();
	}
</script>

<section class="pg-sidepane" aria-label="Project files">
	<div class="pg-pane-head">
		<span>Explorer</span>
		<span class="dim">{allRnxFiles(project).length} rnx</span>
	</div>
	<div class="pg-files">
		<div class="pg-file-new">
			<input
				bind:value={newName}
				onkeydown={(e) => {
					if (e.key === 'Enter') create();
				}}
				placeholder="new file or dir/ (e.g. lib/util.rnx)"
				aria-label="New file name"
			/>
			<button onclick={create} class="pg-ico" aria-label="Create file">
				<Plus size={13} />
			</button>
		</div>
		<div class="pg-file-list" role="tree" aria-label="Project files">
			{#each entries as path (path)}
				{#if !hiddenByCollapse(path)}
					{@const depth = depthOf(path)}
					{@const dir = isDir(path)}
					{@const clean = dir ? path.slice(0, -1) : path}
					{@const active = !dir && project.activePath === clean}
					{@const isEntry = !dir && project.entry === clean}
					<div
						role="treeitem"
						aria-selected={active}
						class="pg-file"
						class:is-active={active}
						style="padding-left: {6 + depth * 14}px"
					>
						<button onclick={() => select(path)} class="flex min-w-0 flex-1 items-center gap-1.5 truncate" title={clean}>
							{#if dir}
								{#if collapsed.has(clean)}<Folder size={13} class="shrink-0" />{:else}<FolderOpen size={13} class="shrink-0" />{/if}
							{:else}
								<File size={13} class="shrink-0" />
							{/if}
							{#if renaming === path}
								<input
									bind:value={renameValue}
									onclick={(e) => e.stopPropagation()}
									onkeydown={(e) => {
										if (e.key === 'Enter') commitRename(path);
										if (e.key === 'Escape') renaming = null;
									}}
									onblur={() => commitRename(path)}
									class="w-full bg-transparent outline-none"
									aria-label="Rename file"
								/>
							{:else}
								<span class="truncate">{clean.split('/').pop()}{dir ? '/' : ''}</span>
							{/if}
						</button>
						{#if isEntry}
							<span class="pg-entry">entry</span>
						{/if}
						<span class="pg-file-tools">
							{#if !dir && !isEntry && clean.endsWith('.rnx')}
								<button onclick={() => makeEntry(clean)} title="Set as entry" aria-label="Set {clean} as entry">
									<Play size={11} />
								</button>
							{/if}
							<button onclick={() => startRename(path)} title="Rename" aria-label="Rename {clean}">
								<Pencil size={11} />
							</button>
							<button onclick={() => remove(path)} class="hover:!text-aura-red" title="Delete" aria-label="Delete {clean}">
								<Trash2 size={11} />
							</button>
						</span>
					</div>
				{/if}
			{/each}
		</div>
	</div>
</section>
