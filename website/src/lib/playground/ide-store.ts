import { deserializeProject, serializeProject, type IdeProject } from './project-model';

export const IDE_DB = 'rnx-engine';
export const IDE_VERSION = 3;
export const IDE_STORE = 'ide';
export const IDE_KEY = 'projects::v1';
export const MAX_SAVED_PROJECTS = 10;
export const STALE_AFTER_MS = 120 * 24 * 3600 * 1000;

export interface IdePersistedState {
	projects: ReturnType<typeof serializeProject>[];
	activeId: string | null;
}

export function pruneForSave(state: IdePersistedState, now: number): IdePersistedState {
	const fresh = state.projects.filter((p) => now - p.updatedAt < STALE_AFTER_MS);
	fresh.sort((a, b) => b.updatedAt - a.updatedAt);
	const kept = fresh.slice(0, MAX_SAVED_PROJECTS);
	const ids = new Set(kept.map((p) => p.id));
	return {
		projects: kept,
		activeId: state.activeId && ids.has(state.activeId) ? state.activeId : (kept[0]?.id ?? null)
	};
}

export function mergeForLoad(saved: IdePersistedState | null): { projects: IdeProject[]; activeId: string | null } {
	if (!saved) return { projects: [], activeId: null };
	const projects: IdeProject[] = [];
	for (const raw of saved.projects) {
		const p = deserializeProject(raw);
		if (p) projects.push(p);
	}
	const activeId = projects.some((p) => p.id === saved.activeId) ? saved.activeId : (projects[0]?.id ?? null);
	return { projects, activeId };
}

function openDb(version: number, stores: string[]): Promise<IDBDatabase | null> {
	return new Promise((resolve) => {
		try {
			const req = indexedDB.open(IDE_DB, version);
			req.onupgradeneeded = () => {
				for (const s of stores) {
					if (!req.result.objectStoreNames.contains(s)) req.result.createObjectStore(s);
				}
			};
			req.onsuccess = () => resolve(req.result);
			req.onerror = () => resolve(null);
		} catch {
			resolve(null);
		}
	});
}

export function openIdeDb(): Promise<IDBDatabase | null> {
	return openDb(IDE_VERSION, ['wasm', 'std', IDE_STORE]);
}

export async function loadIdeState(): Promise<IdePersistedState | null> {
	const db = await openIdeDb();
	if (!db) return null;
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(IDE_STORE, 'readonly');
			const req = tx.objectStore(IDE_STORE).get(IDE_KEY);
			req.onsuccess = () => {
				const v: unknown = req.result;
				if (typeof v === 'object' && v !== null && Array.isArray((v as { projects?: unknown }).projects)) {
					resolve(v as IdePersistedState);
				} else {
					resolve(null);
				}
			};
			req.onerror = () => resolve(null);
		} catch {
			resolve(null);
		}
	});
}

export async function saveIdeState(state: IdePersistedState): Promise<void> {
	const pruned = pruneForSave(state, Date.now());
	const db = await openIdeDb();
	if (!db) return;
	return new Promise((resolve) => {
		try {
			const tx = db.transaction(IDE_STORE, 'readwrite');
			tx.objectStore(IDE_STORE).put({ ...pruned }, IDE_KEY);
			tx.oncomplete = () => resolve();
			tx.onerror = () => resolve();
		} catch {
			resolve();
		}
	});
}
