import { writable } from 'svelte/store';
import { browser } from '$app/environment';

type Theme = 'light' | 'dark' | 'system';

const KEY = 'theme';

function parse(raw: string | null): Theme | null {
	if (!raw) return null;
	try {
		const value = JSON.parse(raw);
		return value === 'light' || value === 'dark' || value === 'system' ? value : null;
	} catch {
		return null;
	}
}

/** localStorage-backed theme preference (inlined replacement for svelte-persisted-store). */
function createTheme() {
	const store = writable<Theme>((browser && parse(localStorage.getItem(KEY))) || 'system');

	if (browser) {
		store.subscribe((value) => localStorage.setItem(KEY, JSON.stringify(value)));
		window.addEventListener('storage', (event) => {
			if (event.key === KEY) {
				const next = parse(event.newValue);
				if (next) store.set(next);
			}
		});
	}

	return store;
}

export const theme = createTheme();
