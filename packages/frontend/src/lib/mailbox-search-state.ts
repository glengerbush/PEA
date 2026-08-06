import type {
	ArchiveSearchField,
	ArchiveSortField,
	MailboxAttachmentFilter,
	MailboxSearchIntent,
	MailboxSearchMode,
	MatchingStrategy,
	SearchFieldMatch,
	SortDirection,
} from '@pea/types';

const DEBOUNCE_MS = 250;
const SORT_FIELDS = new Set<ArchiveSortField>([
	'sentAt',
	'archivedAt',
	'sender',
	'subject',
	'sizeBytes',
]);
const DIRECTIONS = new Set<SortDirection>(['asc', 'desc']);
const MATCHING_STRATEGIES = new Set<MatchingStrategy>(['last', 'all']);
const SEARCH_FIELDS = new Set<ArchiveSearchField>([
	'subject',
	'body',
	'from',
	'senderName',
	'to',
	'cc',
	'bcc',
	'attachments.filename',
	'attachments.content',
	'importSource',
	'sourcePath',
	'tags',
]);

type TextField = 'q' | 'senderQuery' | 'recipientsQuery' | 'subjectQuery' | 'bodyQuery';
type FilterEvent =
	| { type: 'filter-changed'; field: 'senderQuery'; value: string }
	| { type: 'filter-changed'; field: 'recipientsQuery'; value: string }
	| { type: 'filter-changed'; field: 'subjectQuery'; value: string }
	| { type: 'filter-changed'; field: 'bodyQuery'; value: string }
	| { type: 'filter-changed'; field: 'fieldMatch'; value: SearchFieldMatch }
	| { type: 'filter-changed'; field: 'ingestionSourceId'; value: string }
	| { type: 'filter-changed'; field: 'hasAttachments'; value: MailboxAttachmentFilter }
	| { type: 'filter-changed'; field: 'attachmentExt'; value: string }
	| { type: 'filter-changed'; field: 'tags'; value: string };

export type MailboxNavigationType = 'enter' | 'form' | 'goto' | 'link' | 'popstate';

export type MailboxSearchEvent =
	| { type: 'typed'; field: TextField; value: string }
	| FilterEvent
	| { type: 'mode-changed'; mode: MailboxSearchMode }
	| { type: 'submitted' }
	| { type: 'sort-changed'; field: ArchiveSortField }
	| { type: 'limit-changed'; limit: number }
	| { type: 'navigated'; navigationType: MailboxNavigationType; url: string | URL }
	| {
			type: 'navigation-completed';
			navigationId: number;
			url: string | URL;
	  };

export interface MailboxSearchNavigation {
	id: number;
	url: string;
	keepFocus: true;
	replaceState: true;
}

export interface MailboxSearchView {
	draft: MailboxSearchIntent;
	applied: MailboxSearchIntent;
	showAdvanced: boolean;
	hasActiveFilters: boolean;
	resultsMatchDraft: boolean;
	clearUrl: string;
	pagination: { page: number; limit: number };
}

export interface MailboxSearchOutput {
	state: MailboxSearchView;
	navigation?: MailboxSearchNavigation;
	clearSelection?: true;
}

export interface MailboxSearchScheduler {
	set(delayMs: number, callback: () => void): unknown;
	clear(id: unknown): void;
}

export interface MailboxSearchRuntime {
	scheduler: MailboxSearchScheduler;
	onOutput(output: MailboxSearchOutput): void;
}

const browserScheduler: MailboxSearchScheduler = {
	set: (delayMs, callback) => setTimeout(callback, delayMs),
	clear: (id) => clearTimeout(id as ReturnType<typeof setTimeout>),
};

function text(params: URLSearchParams, key: string): string {
	return (params.get(key) ?? '').trim();
}

function positiveInteger(value: string | null, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

function fields(value: string | null): ArchiveSearchField[] {
	if (!value || value === 'all') return [];
	return value
		.split(',')
		.map((field) => field.trim())
		.filter((field): field is ArchiveSearchField =>
			SEARCH_FIELDS.has(field as ArchiveSearchField)
		);
}

function hasAdvancedMeaning(intent: MailboxSearchIntent): boolean {
	return Boolean(
		intent.senderQuery ||
		intent.recipientsQuery ||
		intent.subjectQuery ||
		intent.bodyQuery ||
		intent.fieldMatch === 'any' ||
		intent.ingestionSourceId !== 'all' ||
		intent.hasAttachments !== 'any' ||
		intent.attachmentExt ||
		intent.tags
	);
}

function asUrl(value: string | URL): URL {
	return value instanceof URL ? value : new URL(value, 'http://mailbox.local');
}

export function decodeMailboxSearchUrl(value: string | URL): MailboxSearchIntent {
	const params = asUrl(value).searchParams;
	const sortValue = params.get('sort');
	const directionValue = params.get('direction');
	const strategyValue = params.get('matchingStrategy');
	const attachmentValue = params.get('hasAttachments');
	const intent: MailboxSearchIntent = {
		mode: 'simple',
		q: text(params, 'q'),
		fields: fields(params.get('fields')),
		senderQuery: text(params, 'senderQuery'),
		recipientsQuery: text(params, 'recipientsQuery'),
		subjectQuery: text(params, 'subjectQuery'),
		bodyQuery: text(params, 'bodyQuery'),
		fieldMatch: params.get('fieldMatch') === 'any' ? 'any' : 'all',
		ingestionSourceId: text(params, 'ingestionSourceId') || 'all',
		hasAttachments:
			attachmentValue === 'true' || attachmentValue === 'false' ? attachmentValue : 'any',
		attachmentExt: text(params, 'attachmentExt'),
		tags: text(params, 'tags'),
		page: positiveInteger(params.get('page'), 1),
		limit: Math.min(positiveInteger(params.get('limit'), 25), 100),
		sort:
			sortValue && SORT_FIELDS.has(sortValue as ArchiveSortField)
				? (sortValue as ArchiveSortField)
				: 'sentAt',
		direction:
			directionValue && DIRECTIONS.has(directionValue as SortDirection)
				? (directionValue as SortDirection)
				: 'desc',
		matchingStrategy:
			strategyValue && MATCHING_STRATEGIES.has(strategyValue as MatchingStrategy)
				? (strategyValue as MatchingStrategy)
				: 'last',
		from: text(params, 'from'),
	};
	intent.mode =
		hasAdvancedMeaning(intent) || params.get('fieldMatch') === 'all' ? 'advanced' : 'simple';
	if (intent.mode === 'advanced') {
		intent.q = '';
		intent.fields = [];
	} else {
		intent.senderQuery = '';
		intent.recipientsQuery = '';
		intent.subjectQuery = '';
		intent.bodyQuery = '';
		intent.fieldMatch = 'all';
		intent.ingestionSourceId = 'all';
		intent.hasAttachments = 'any';
		intent.attachmentExt = '';
		intent.tags = '';
	}
	return intent;
}

function setNonDefault(
	params: URLSearchParams,
	key: string,
	value: string,
	defaultValue = ''
): void {
	const normalized = value.trim();
	if (normalized && normalized !== defaultValue) params.set(key, normalized);
}

function appendSearchCriteria(
	params: URLSearchParams,
	intent: MailboxSearchIntent,
	preserveEmptyAdvancedMode = false
): void {
	if (intent.mode === 'advanced') {
		setNonDefault(params, 'senderQuery', intent.senderQuery);
		setNonDefault(params, 'recipientsQuery', intent.recipientsQuery);
		setNonDefault(params, 'subjectQuery', intent.subjectQuery);
		setNonDefault(params, 'bodyQuery', intent.bodyQuery);
		setNonDefault(params, 'fieldMatch', intent.fieldMatch, 'all');
		setNonDefault(params, 'ingestionSourceId', intent.ingestionSourceId, 'all');
		setNonDefault(params, 'hasAttachments', intent.hasAttachments, 'any');
		setNonDefault(params, 'attachmentExt', intent.attachmentExt);
		setNonDefault(params, 'tags', intent.tags);
		if (preserveEmptyAdvancedMode && !hasAdvancedMeaning(intent)) {
			params.set('fieldMatch', 'all');
		}
	} else {
		setNonDefault(params, 'q', intent.q);
		if (intent.fields.length > 0) params.set('fields', intent.fields.join(','));
	}
}

export function encodeMailboxSearchUrl(intent: MailboxSearchIntent): string {
	const params = new URLSearchParams();
	appendSearchCriteria(params, intent, true);
	setNonDefault(params, 'sort', intent.sort, 'sentAt');
	setNonDefault(params, 'direction', intent.direction, 'desc');
	setNonDefault(params, 'limit', String(intent.limit), '25');
	setNonDefault(params, 'matchingStrategy', intent.matchingStrategy, 'last');
	setNonDefault(params, 'from', intent.from);
	if (intent.page > 1) params.set('page', String(intent.page));
	const query = params.toString();
	return `/mailbox${query ? `?${query}` : ''}`;
}

export function toArchivedEmailSearchParams(intent: MailboxSearchIntent): URLSearchParams {
	const params = new URLSearchParams({
		page: String(intent.page),
		limit: String(intent.limit),
		sort: intent.sort,
		direction: intent.direction,
		matchingStrategy: intent.matchingStrategy,
	});
	appendSearchCriteria(params, intent);
	return params;
}

export function decodeMailboxSearchRequest(value: string | URL): {
	intent: MailboxSearchIntent;
	params: URLSearchParams;
} {
	const intent = decodeMailboxSearchUrl(value);
	return { intent, params: toArchivedEmailSearchParams(intent) };
}

function comparableMeaning(intent: MailboxSearchIntent): string {
	const normalized = { ...intent, page: 1, from: '' };
	return encodeMailboxSearchUrl(normalized);
}

function activeFilters(intent: MailboxSearchIntent): boolean {
	const searchIsActive =
		intent.mode === 'advanced'
			? hasAdvancedMeaning(intent)
			: Boolean(intent.q || intent.fields.length > 0);
	return searchIsActive || intent.matchingStrategy !== 'last';
}

function view(draft: MailboxSearchIntent, applied: MailboxSearchIntent): MailboxSearchView {
	return {
		draft: structuredClone(draft),
		applied: structuredClone(applied),
		showAdvanced: draft.mode === 'advanced',
		hasActiveFilters: activeFilters(applied),
		resultsMatchDraft: comparableMeaning(draft) === comparableMeaning(applied),
		clearUrl: encodeMailboxSearchUrl({
			...draft,
			mode: 'simple',
			q: '',
			fields: [],
			senderQuery: '',
			recipientsQuery: '',
			subjectQuery: '',
			bodyQuery: '',
			fieldMatch: 'all',
			ingestionSourceId: 'all',
			hasAttachments: 'any',
			attachmentExt: '',
			tags: '',
			page: 1,
			sort: 'sentAt',
			direction: 'desc',
			limit: 25,
			matchingStrategy: 'last',
		}),
		pagination: { page: applied.page, limit: applied.limit },
	};
}

export interface MailboxSearchState {
	readonly state: MailboxSearchView;
	dispatch(event: MailboxSearchEvent): MailboxSearchOutput;
	urlForPage(page: number): string;
	dispose(): void;
}

export function createMailboxSearchState(
	initial: MailboxSearchIntent,
	runtime: Partial<MailboxSearchRuntime> & Pick<MailboxSearchRuntime, 'onOutput'>
): MailboxSearchState {
	let draft = structuredClone(initial);
	let applied = structuredClone(initial);
	let pendingTimer: unknown;
	let nextNavigationId = 1;
	let latestNavigationId = 0;
	let disposed = false;
	const scheduler = runtime.scheduler ?? browserScheduler;

	function output(
		navigation?: MailboxSearchNavigation,
		clearSelection?: true
	): MailboxSearchOutput {
		return { state: view(draft, applied), navigation, clearSelection };
	}

	function cancelPending(): void {
		if (pendingTimer !== undefined) scheduler.clear(pendingTimer);
		pendingTimer = undefined;
	}

	function navigate(): MailboxSearchOutput {
		cancelPending();
		draft.page = 1;
		const id = nextNavigationId++;
		latestNavigationId = id;
		return output(
			{ id, url: encodeMailboxSearchUrl(draft), keepFocus: true, replaceState: true },
			true
		);
	}

	function scheduleNavigation(): MailboxSearchOutput {
		cancelPending();
		pendingTimer = scheduler.set(DEBOUNCE_MS, () => {
			pendingTimer = undefined;
			if (!disposed) runtime.onOutput(navigate());
		});
		return output(undefined, true);
	}

	function dispatch(event: MailboxSearchEvent): MailboxSearchOutput {
		if (disposed) return output();
		switch (event.type) {
			case 'typed':
				draft[event.field] = event.value;
				return scheduleNavigation();
			case 'filter-changed':
				Object.assign(draft, { [event.field]: event.value, page: 1 });
				return navigate();
			case 'mode-changed':
				draft.mode = event.mode;
				return navigate();
			case 'submitted':
				return navigate();
			case 'sort-changed':
				if (draft.sort === event.field) {
					draft.direction = draft.direction === 'asc' ? 'desc' : 'asc';
				} else {
					draft.sort = event.field;
					draft.direction = 'desc';
				}
				return navigate();
			case 'limit-changed':
				draft.limit = Math.min(Math.max(Math.floor(event.limit) || 25, 1), 100);
				return navigate();
			case 'navigated':
				if (event.navigationType === 'goto') return output();
				cancelPending();
				latestNavigationId = nextNavigationId++;
				const previousMeaning = comparableMeaning(applied);
				draft = decodeMailboxSearchUrl(event.url);
				applied = structuredClone(draft);
				return output(
					undefined,
					previousMeaning === comparableMeaning(applied) ? undefined : true
				);
			case 'navigation-completed':
				if (event.navigationId !== latestNavigationId) return output();
				applied = decodeMailboxSearchUrl(event.url);
				return output();
		}
	}

	return {
		get state() {
			return view(draft, applied);
		},
		dispatch,
		urlForPage(page) {
			return encodeMailboxSearchUrl({
				...draft,
				page: Math.max(1, Math.floor(page) || 1),
			});
		},
		dispose() {
			disposed = true;
			cancelPending();
		},
	};
}
