import { writable } from 'svelte/store';

/**
 * Cross-list view state so returning from an email lands you where you were.
 *
 * `positions` remembers each list view's window scroll offset, keyed by its
 * full URL (pathname + search), so /mailbox, /trash, /dashboard/duplicates and
 * every filter/sort/page variant restore independently. In-memory only: a page
 * reload starts fresh, which is the expected "cold open" behaviour.
 *
 * `lastOpenedEmailId` is the email most recently opened from any list, so the
 * list can highlight that row on return — you never lose which one you'd read.
 */
const positions = new Map<string, number>();

export function saveListScroll(key: string, y: number): void {
	positions.set(key, y);
}

export function getListScroll(key: string): number | undefined {
	return positions.get(key);
}

export const lastOpenedEmailId = writable<string | null>(null);
