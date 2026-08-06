import { describe, expect, it } from 'vitest';
import {
	createMailboxSearchState,
	decodeMailboxSearchUrl,
	decodeMailboxSearchRequest,
	encodeMailboxSearchUrl,
	type MailboxSearchOutput,
	type MailboxSearchScheduler,
} from './mailbox-search-state';

class FakeScheduler implements MailboxSearchScheduler {
	private now = 0;
	private nextId = 1;
	private tasks = new Map<number, { at: number; callback: () => void }>();

	set(delayMs: number, callback: () => void): number {
		const id = this.nextId++;
		this.tasks.set(id, { at: this.now + delayMs, callback });
		return id;
	}

	clear(id: unknown): void {
		this.tasks.delete(id as number);
	}

	advance(milliseconds: number): void {
		this.now += milliseconds;
		for (const [id, task] of [...this.tasks.entries()]
			.filter(([, task]) => task.at <= this.now)
			.sort((left, right) => left[1].at - right[1].at)) {
			this.tasks.delete(id);
			task.callback();
		}
	}
}

function setup(url = '/mailbox') {
	const scheduler = new FakeScheduler();
	const emitted: MailboxSearchOutput[] = [];
	const search = createMailboxSearchState(decodeMailboxSearchUrl(url), {
		scheduler,
		onOutput: (output) => emitted.push(output),
	});
	return { search, scheduler, emitted };
}

describe('mailbox URL codec', () => {
	it('round-trips every mailbox search setting', () => {
		const intent = decodeMailboxSearchUrl(
			'/mailbox?senderQuery=alice&recipientsQuery=bob&subjectQuery=invoice&bodyQuery=paid&fieldMatch=any&ingestionSourceId=source-1&hasAttachments=false&attachmentExt=pdf%2Cdocx&tags=finance&sort=subject&direction=asc&limit=50&page=3&matchingStrategy=all&from=%2Fdashboard%3Fperiod%3Dmonth'
		);

		expect(intent).toEqual({
			mode: 'advanced',
			q: '',
			fields: [],
			senderQuery: 'alice',
			recipientsQuery: 'bob',
			subjectQuery: 'invoice',
			bodyQuery: 'paid',
			fieldMatch: 'any',
			ingestionSourceId: 'source-1',
			hasAttachments: 'false',
			attachmentExt: 'pdf,docx',
			tags: 'finance',
			page: 3,
			limit: 50,
			sort: 'subject',
			direction: 'asc',
			matchingStrategy: 'all',
			from: '/dashboard?period=month',
		});
		expect(decodeMailboxSearchUrl(encodeMailboxSearchUrl(intent))).toEqual(intent);
	});

	it('normalizes invalid values and omits defaults', () => {
		const intent = decodeMailboxSearchUrl(
			'/mailbox?fields=subject%2Cnope&sort=nope&direction=sideways&matchingStrategy=nope&fieldMatch=nope&hasAttachments=nope&page=-4&limit=999'
		);

		expect(intent.fields).toEqual(['subject']);
		expect(intent).toMatchObject({
			mode: 'simple',
			page: 1,
			limit: 100,
			sort: 'sentAt',
			direction: 'desc',
			matchingStrategy: 'last',
			fieldMatch: 'all',
			hasAttachments: 'any',
		});
		expect(encodeMailboxSearchUrl(intent)).toBe('/mailbox?fields=subject&limit=100');
	});

	it('round-trips an empty advanced mode through history', () => {
		const advanced = { ...decodeMailboxSearchUrl('/mailbox'), mode: 'advanced' as const };
		const url = encodeMailboxSearchUrl(advanced);

		expect(url).toBe('/mailbox?fieldMatch=all');
		expect(decodeMailboxSearchUrl(url).mode).toBe('advanced');
		expect(decodeMailboxSearchRequest(url).params.has('fieldMatch')).toBe(false);
	});

	it('decodes the loader request with the unchanged engine grammar', () => {
		const { intent, params } = decodeMailboxSearchRequest(
			'/mailbox?q=quarterly&fields=subject%2Cbody&sort=archivedAt&direction=asc&limit=50&page=2&matchingStrategy=all'
		);

		expect(intent).toEqual(decodeMailboxSearchUrl(encodeMailboxSearchUrl(intent)));
		expect(params.toString()).toBe(
			'page=2&limit=50&sort=archivedAt&direction=asc&matchingStrategy=all&q=quarterly&fields=subject%2Cbody'
		);
	});
});

describe('mailbox search state', () => {
	it('keeps newer typing when an earlier search completes', () => {
		const { search, scheduler, emitted } = setup();
		search.dispatch({ type: 'typed', field: 'q', value: 'swan' });
		scheduler.advance(250);
		const navigation = emitted.at(-1)?.navigation;
		expect(navigation?.url).toBe('/mailbox?q=swan');

		search.dispatch({ type: 'typed', field: 'q', value: 'swans' });
		search.dispatch({
			type: 'navigation-completed',
			navigationId: navigation!.id,
			url: navigation!.url,
		});

		expect(search.state.draft.q).toBe('swans');
		expect(search.state.applied.q).toBe('swan');
		expect(search.state.resultsMatchDraft).toBe(false);
	});

	it('lets only the latest out-of-order navigation update applied state', () => {
		const { search } = setup();
		const first = search.dispatch({ type: 'submitted' });
		search.dispatch({ type: 'typed', field: 'q', value: 'newer' });
		const second = search.dispatch({ type: 'submitted' });

		search.dispatch({
			type: 'navigation-completed',
			navigationId: second.navigation!.id,
			url: second.navigation!.url,
		});
		search.dispatch({
			type: 'navigation-completed',
			navigationId: first.navigation!.id,
			url: first.navigation!.url,
		});

		expect(search.state.applied.q).toBe('newer');
	});

	it('debounces typing, resets the delay, and Enter applies immediately', () => {
		const { search, scheduler, emitted } = setup();
		search.dispatch({ type: 'typed', field: 'q', value: 'a' });
		scheduler.advance(200);
		search.dispatch({ type: 'typed', field: 'q', value: 'ab' });
		scheduler.advance(249);
		expect(emitted).toHaveLength(0);
		scheduler.advance(1);
		expect(emitted.at(-1)?.navigation?.url).toBe('/mailbox?q=ab');

		search.dispatch({ type: 'typed', field: 'q', value: 'abc' });
		const immediate = search.dispatch({ type: 'submitted' });
		expect(immediate.navigation?.url).toBe('/mailbox?q=abc');
		scheduler.advance(250);
		expect(emitted).toHaveLength(1);
	});

	it('cancels pending work when disposed', () => {
		const { search, scheduler, emitted } = setup();
		search.dispatch({ type: 'typed', field: 'q', value: 'leave' });
		search.dispose();
		scheduler.advance(250);
		expect(emitted).toHaveLength(0);
	});

	it.each(['enter', 'link', 'popstate', 'form'] as const)(
		'reseeds draft on %s navigation',
		(navigationType) => {
			const { search } = setup('/mailbox?q=old');
			search.dispatch({ type: 'typed', field: 'q', value: 'unfinished' });
			search.dispatch({
				type: 'navigated',
				navigationType,
				url: '/mailbox?senderQuery=alice&page=2',
			});
			expect(search.state.draft).toMatchObject({
				mode: 'advanced',
				q: '',
				senderQuery: 'alice',
				page: 2,
			});
			expect(search.state.resultsMatchDraft).toBe(true);
		}
	);

	it('ignores framework navigation notifications for its own goto', () => {
		const { search } = setup('/mailbox?q=old');
		search.dispatch({ type: 'typed', field: 'q', value: 'new' });
		search.dispatch({
			type: 'navigated',
			navigationType: 'goto',
			url: '/mailbox?q=old',
		});
		expect(search.state.draft.q).toBe('new');
	});

	it('clears selection and invalidates pending search on user navigation', () => {
		const { search } = setup('/mailbox?q=old&page=3');
		const pending = search.dispatch({ type: 'submitted' }).navigation!;
		const navigated = search.dispatch({
			type: 'navigated',
			navigationType: 'popstate',
			url: '/mailbox?q=history&page=2',
		});
		search.dispatch({
			type: 'navigation-completed',
			navigationId: pending.id,
			url: pending.url,
		});

		expect(navigated.clearSelection).toBe(true);
		expect(search.state.applied).toMatchObject({ q: 'history', page: 2 });
	});

	it('switches modes without leaking hidden criteria', () => {
		const { search } = setup('/mailbox?q=simple');
		search.dispatch({ type: 'filter-changed', field: 'senderQuery', value: 'alice' });
		const advanced = search.dispatch({ type: 'mode-changed', mode: 'advanced' });
		expect(advanced.navigation?.url).toBe('/mailbox?senderQuery=alice');

		const simple = search.dispatch({ type: 'mode-changed', mode: 'simple' });
		expect(simple.navigation?.url).toBe('/mailbox?q=simple');
	});

	it('clears selection for meaning changes but not pagination', () => {
		const { search } = setup(
			'/mailbox?q=old&sort=subject&direction=desc&page=3&from=%2Fdashboard'
		);
		const typed = search.dispatch({ type: 'typed', field: 'q', value: 'new' });
		const filtered = search.dispatch({
			type: 'filter-changed',
			field: 'senderQuery',
			value: 'alice',
		});
		const directionChanged = search.dispatch({ type: 'sort-changed', field: 'subject' });
		const limited = search.dispatch({ type: 'limit-changed', limit: 50 });

		expect(typed.clearSelection).toBe(true);
		expect(filtered.clearSelection).toBe(true);
		expect(directionChanged.clearSelection).toBe(true);
		expect(directionChanged.navigation?.url).toContain('direction=asc');
		expect(limited.clearSelection).toBe(true);
		expect(limited.navigation?.url).not.toContain('page=');
		expect(search.urlForPage(4)).toContain('page=4');
		expect(search.urlForPage(4)).toContain('from=%2Fdashboard');
	});

	it('derives advanced visibility, active filters, and clear target', () => {
		const { search } = setup('/mailbox?fieldMatch=any&from=%2Fdashboard');
		expect(search.state).toMatchObject({
			showAdvanced: true,
			hasActiveFilters: true,
			resultsMatchDraft: true,
			clearUrl: '/mailbox?from=%2Fdashboard',
			pagination: { page: 1, limit: 25 },
		});
	});
});
