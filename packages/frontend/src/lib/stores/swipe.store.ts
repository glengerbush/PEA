import { writable } from 'svelte/store';
import { browser } from '$app/environment';

/**
 * User preference: disable the two-finger (trackpad) swipe-to-go-back gesture on
 * the email detail view. Persisted to localStorage and synced across tabs,
 * mirroring the theme store. Defaults to enabled (false = swipe is on).
 */
const KEY = 'disableTwoFingerSwipe';

function parse(raw: string | null): boolean {
	return raw === 'true';
}

function createDisableTwoFingerSwipe() {
	const store = writable<boolean>(browser ? parse(localStorage.getItem(KEY)) : false);
	if (browser) {
		store.subscribe((value) => localStorage.setItem(KEY, value ? 'true' : 'false'));
		window.addEventListener('storage', (event) => {
			if (event.key === KEY) store.set(parse(event.newValue));
		});
	}
	return store;
}

export const disableTwoFingerSwipe = createDisableTwoFingerSwipe();
