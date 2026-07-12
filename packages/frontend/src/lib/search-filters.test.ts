import { describe, it, expect } from 'vitest';
import { shouldSyncInputsFromUrl } from './search-filters';

// Regression for the "search eats the letters I type" bug. The mailbox search
// box used to be $derived(loader data), so when a slow search's response landed
// it overwrote the input — deleting characters typed while it was in flight. The
// box is now local $state; the URL re-seeds it only on user-initiated navigation
// and NEVER after our own search goto. This predicate is exactly that gate.
describe('shouldSyncInputsFromUrl', () => {
	it('leaves the inputs alone after our own search-as-you-type goto', () => {
		// The exact scenario that used to eat letters: a debounced search fired
		// (a goto) carrying "swan"; the user typed on to "swans" while it was in
		// flight. When that navigation completes (type 'goto') the box must NOT be
		// re-seeded from the URL, or "swans" snaps back to "swan".
		expect(shouldSyncInputsFromUrl('goto')).toBe(false);
	});

	it('re-seeds the inputs on user-initiated navigation', () => {
		// Back/forward, the Clear link, a form submit, and the initial load are all
		// the user changing the URL — the inputs should follow it.
		expect(shouldSyncInputsFromUrl('enter')).toBe(true);
		expect(shouldSyncInputsFromUrl('popstate')).toBe(true);
		expect(shouldSyncInputsFromUrl('link')).toBe(true);
		expect(shouldSyncInputsFromUrl('form')).toBe(true);
	});
});
