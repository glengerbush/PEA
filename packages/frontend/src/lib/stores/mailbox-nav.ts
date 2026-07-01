import { writable } from 'svelte/store';

/**
 * Remembers the last mailbox list URL (including its search/filter/page query)
 * so the email detail page can offer a "Back" button that returns to the exact
 * previous view. Set by the mailbox list page on navigation; read by the email
 * detail page. Falls back to `/mailbox` when empty (e.g. the email was opened
 * directly without visiting the list first).
 */
export const lastMailboxListUrl = writable<string | null>(null);
