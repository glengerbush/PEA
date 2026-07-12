import type { NavigationType } from '@sveltejs/kit';

/**
 * Whether a just-completed navigation should re-seed the mailbox search form
 * inputs (query, fields, filters, sort…) from the URL.
 *
 * The inputs are locally-owned `$state`: the user edits them, and
 * `buildArchiveUrl()` writes them into the URL. The URL must write them back
 * only when it changed for a reason *other than our own search*.
 *
 * Search-as-you-type, pagination, sort and page-size all navigate with
 * `goto()` (`type: 'goto'`). Re-seeding after those would let a slow search's
 * response overwrite characters the user typed while it was in flight — the
 * "it deletes letters I typed" bug. Every other navigation type is
 * user-initiated (initial `enter`, `popstate` back/forward, a `link` such as
 * the Clear button, a `form` submit) and *should* re-seed the inputs.
 */
export function shouldSyncInputsFromUrl(type: NavigationType): boolean {
	return type !== 'goto';
}
