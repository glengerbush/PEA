import type { EmailDocument } from './email.types';

export type MatchingStrategy = 'last' | 'all' | 'frequency';
export type ArchiveSearchField =
	| 'subject'
	| 'body'
	| 'from'
	| 'senderName'
	| 'to'
	| 'cc'
	| 'bcc'
	| 'attachments.filename'
	| 'attachments.content'
	| 'userEmail'
	| 'sourcePath'
	| 'sourceLabels'
	| 'tags';
export type ArchiveSortField = 'sentAt' | 'archivedAt' | 'sender' | 'subject' | 'sizeBytes';
export type SortDirection = 'asc' | 'desc';

export interface SearchQuery {
	query: string;
	filters?: Record<string, any>;
	fields?: ArchiveSearchField[];
	sort?: ArchiveSortField;
	direction?: SortDirection;
	page?: number;
	limit?: number;
	matchingStrategy?: MatchingStrategy;
}

export interface ArchiveQueryFilters {
	ingestionSourceId?: string;
	userEmail?: string;
	from?: string;
	to?: string;
	cc?: string;
	bcc?: string;
	hasAttachments?: boolean;
	sourcePath?: string;
	sourceLabels?: string[];
	tags?: string[];
	sentAfter?: string | number | Date;
	sentBefore?: string | number | Date;
	archivedAfter?: string | number | Date;
	archivedBefore?: string | number | Date;
}

export interface ArchiveQuery {
	query?: string;
	filters?: ArchiveQueryFilters;
	fields?: ArchiveSearchField[];
	sort?: ArchiveSortField;
	direction?: SortDirection;
	page?: number;
	limit?: number;
	matchingStrategy?: MatchingStrategy;
}

export interface SearchHit extends EmailDocument {
	_matchesPosition?: {
		[key: string]: { start: number; length: number; indices?: number[] }[];
	};
	_formatted?: Partial<EmailDocument>;
}

export interface SearchResult {
	hits: SearchHit[];
	total: number;
	page: number;
	limit: number;
	totalPages: number;
	processingTimeMs: number;
}
