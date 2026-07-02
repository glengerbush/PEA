import { sqlite } from '../database';
import type {
	ArchiveQuery,
	ArchiveQueryFilters,
	ArchiveSearchField,
	ArchiveSortField,
	SearchQuery,
	SearchResult,
	EmailDocument,
	TopSender,
	SortDirection,
} from '@open-archiver/types';
import { IngestionService } from './IngestionService';
import { sendJob } from '../jobs/queue';
import { logger } from '../config/logger';

/**
 * Full-text search over SQLite FTS5 (successor to Meilisearch — one process,
 * no search daemon, the index lives inside archive.db).
 *
 * Layout: a content-full FTS5 table whose rowid mirrors archived_emails.rowid,
 * so upserts/deletes are O(log n) via the id → rowid subquery instead of
 * scanning the UNINDEXED email_id column.
 */

const FTS_TABLE = 'email_fts';

// Search-field presets (Meili attribute names, kept for API/UI compatibility)
// mapped onto FTS columns.
const FIELD_TO_COLUMN: Record<ArchiveSearchField, string> = {
	subject: 'subject',
	body: 'body',
	from: 'sender',
	senderName: 'sender',
	to: 'recipients',
	cc: 'recipients',
	bcc: 'recipients',
	'attachments.filename': 'attachments',
	'attachments.content': 'attachments',
	userEmail: 'meta',
	sourcePath: 'meta',
	sourceLabels: 'meta',
	tags: 'meta',
};
const ALL_COLUMNS = ['subject', 'body', 'sender', 'recipients', 'attachments', 'meta'];

const DEFAULT_SEARCH_FIELDS = Object.keys(FIELD_TO_COLUMN) as ArchiveSearchField[];

// Maps sort keys to archived_emails columns (identifiers, never user input).
const SORT_COLUMN_MAP: Record<ArchiveSortField, string> = {
	sentAt: 'ae.sent_at',
	archivedAt: 'ae.archived_at',
	sender: 'ae.sender_email',
	subject: 'ae.subject',
	sizeBytes: 'ae.size_bytes',
};

// bm25() takes one weight per column in table order:
// (email_id, subject, body, sender, recipients, attachments, meta)
const BM25_WEIGHTS = '0.0, 10.0, 2.0, 6.0, 3.0, 1.0, 2.0';

function clampPositiveInteger(value: number | undefined, fallback: number, max: number): number {
	if (!value || !Number.isFinite(value) || value < 1) {
		return fallback;
	}
	return Math.min(Math.floor(value), max);
}

function toTimestamp(value: string | number | Date | undefined): number | null {
	if (value === undefined) return null;
	if (value instanceof Date) return Number.isNaN(value.getTime()) ? null : value.getTime();
	if (typeof value === 'number') return Number.isFinite(value) ? value : null;

	const parsed = Date.parse(value);
	return Number.isNaN(parsed) ? null : parsed;
}

function normalizeSearchFields(fields: ArchiveSearchField[] | undefined): ArchiveSearchField[] {
	if (!fields || fields.length === 0) {
		return DEFAULT_SEARCH_FIELDS;
	}
	const allowed = new Set(DEFAULT_SEARCH_FIELDS);
	const normalized = fields.filter((field) => allowed.has(field));
	return normalized.length > 0 ? normalized : DEFAULT_SEARCH_FIELDS;
}

/**
 * Builds a safe FTS5 MATCH expression from raw user input. Every term is
 * double-quoted (FTS5 operators and email-address punctuation would otherwise
 * be query syntax); the final term is a prefix so search-as-you-type works.
 */
function buildMatchExpression(
	query: string,
	fields: ArchiveSearchField[],
	mode: 'and' | 'or'
): string | null {
	const terms = query
		.split(/\s+/)
		.map((term) => term.replace(/"/g, '').trim())
		.filter((term) => term.length > 0)
		.slice(0, 12);
	if (terms.length === 0) {
		return null;
	}
	const quoted = terms.map((term, index) =>
		index === terms.length - 1 ? `"${term}"*` : `"${term}"`
	);
	const body = quoted.join(mode === 'or' ? ' OR ' : ' ');

	const columns = [...new Set(fields.map((field) => FIELD_TO_COLUMN[field]))];
	if (columns.length >= ALL_COLUMNS.length) {
		return body;
	}
	return `{${columns.join(' ')}} : (${body})`;
}

interface EmailRow {
	id: string;
	user_email: string;
	sender_email: string;
	sender_name: string | null;
	recipients: string | null;
	subject: string | null;
	sent_at: number;
	archived_at: number;
	ingestion_source_id: string;
	thread_id: string | null;
	message_id_header: string | null;
	has_attachments: number;
	source_path: string | null;
	source_labels: string | null;
	tags: string | null;
	size_bytes: number;
	snippet?: string;
}

function rowToDocument(row: EmailRow): EmailDocument {
	const recipients = row.recipients ? JSON.parse(row.recipients) : {};
	const addresses = (list: Array<{ address?: string }> | undefined): string[] =>
		(list ?? []).map((entry) => entry.address ?? '').filter(Boolean);
	return {
		id: row.id,
		userEmail: row.user_email,
		from: row.sender_email,
		senderName: row.sender_name ?? '',
		to: addresses(recipients.to),
		cc: addresses(recipients.cc),
		bcc: addresses(recipients.bcc),
		subject: row.subject ?? '',
		body: row.snippet ?? '',
		attachments: [],
		timestamp: row.sent_at,
		archivedAt: row.archived_at,
		ingestionSourceId: row.ingestion_source_id,
		threadId: row.thread_id,
		messageIdHeader: row.message_id_header,
		hasAttachments: Boolean(row.has_attachments),
		sourcePath: row.source_path,
		sourceLabels: row.source_labels ? JSON.parse(row.source_labels) : [],
		tags: row.tags ? JSON.parse(row.tags) : [],
		sizeBytes: row.size_bytes,
	};
}

let ftsReady = false;

export class SearchService {
	/** Creates the FTS table if missing. Replaces Meilisearch index configuration. */
	public async configureEmailIndex(): Promise<void> {
		if (ftsReady) {
			return;
		}
		sqlite.exec(`
			CREATE VIRTUAL TABLE IF NOT EXISTS ${FTS_TABLE} USING fts5(
				email_id UNINDEXED,
				subject, body, sender, recipients, attachments, meta,
				tokenize = 'unicode61 remove_diacritics 2',
				prefix = '2 3'
			);
		`);
		// Drift safety: drop index rows whose email no longer exists (e.g. a
		// crash between source deletion and index cleanup).
		sqlite.exec(
			`DELETE FROM ${FTS_TABLE} WHERE email_id NOT IN (SELECT id FROM archived_emails)`
		);
		ftsReady = true;
	}

	private ensureReady(): void {
		if (!ftsReady) {
			// Sync path of configureEmailIndex (it only awaits nothing).
			void this.configureEmailIndex();
		}
	}

	/** Indexes (or re-indexes) full email documents. indexName kept for API compat. */
	public async addDocuments(_indexName: string, documents: EmailDocument[], _primaryKey?: string) {
		this.ensureReady();
		const del = sqlite.prepare(
			`DELETE FROM ${FTS_TABLE} WHERE rowid = (SELECT rowid FROM archived_emails WHERE id = ?)`
		);
		const ins = sqlite.prepare(`
			INSERT INTO ${FTS_TABLE} (rowid, email_id, subject, body, sender, recipients, attachments, meta)
			SELECT rowid, ?, ?, ?, ?, ?, ?, ? FROM archived_emails WHERE id = ?
		`);
		sqlite.transaction(() => {
			for (const doc of documents) {
				del.run(doc.id);
				ins.run(
					doc.id,
					doc.subject ?? '',
					doc.body ?? '',
					`${doc.from ?? ''} ${doc.senderName ?? ''}`.trim(),
					[...doc.to, ...doc.cc, ...doc.bcc].join(' '),
					doc.attachments
						.map((attachment) => `${attachment.filename}\n${attachment.content}`)
						.join('\n'),
					[doc.userEmail, doc.sourcePath ?? '', ...doc.sourceLabels, ...doc.tags].join(' '),
					doc.id
				);
			}
		})();
	}

	/**
	 * Partial updates. The only caller updates {id, tags} after tag edits — the
	 * meta column is recomputed from the database row.
	 */
	public async updateDocuments(
		_indexName: string,
		documents: Array<{ id: string } & Partial<EmailDocument>>,
		_primaryKey?: string
	) {
		this.ensureReady();
		const fetch = sqlite.prepare(
			`SELECT user_email, source_path, source_labels, tags FROM archived_emails WHERE id = ?`
		);
		const update = sqlite.prepare(`
			UPDATE ${FTS_TABLE} SET meta = ?
			WHERE rowid = (SELECT rowid FROM archived_emails WHERE id = ?)
		`);
		sqlite.transaction(() => {
			for (const doc of documents) {
				const row = fetch.get(doc.id) as
					| {
							user_email: string;
							source_path: string | null;
							source_labels: string | null;
							tags: string | null;
					  }
					| undefined;
				if (!row) {
					continue;
				}
				const labels: string[] = row.source_labels ? JSON.parse(row.source_labels) : [];
				const tags: string[] = row.tags ? JSON.parse(row.tags) : [];
				update.run(
					[row.user_email, row.source_path ?? '', ...labels, ...tags].join(' '),
					doc.id
				);
			}
		})();
	}

	/** Removes emails from the index by id. Call BEFORE deleting the rows. */
	public async deleteDocuments(_indexName: string, ids: string[]) {
		this.ensureReady();
		const del = sqlite.prepare(
			`DELETE FROM ${FTS_TABLE} WHERE rowid = (SELECT rowid FROM archived_emails WHERE id = ?)`
		);
		sqlite.transaction(() => {
			for (const id of ids) {
				del.run(id);
			}
		})();
	}

	/** Removes a whole source's emails from the index. Call BEFORE deleting the rows. */
	public async deleteDocumentsBySource(ingestionSourceId: string) {
		this.ensureReady();
		sqlite
			.prepare(
				`DELETE FROM ${FTS_TABLE} WHERE rowid IN (SELECT rowid FROM archived_emails WHERE ingestion_source_id = ?)`
			)
			.run(ingestionSourceId);
	}

	/** Escape hatch: wipes the index and re-enqueues indexing for every email. */
	public async rebuildIndex(): Promise<{ enqueuedBatches: number; totalEmails: number }> {
		this.ensureReady();
		sqlite.exec(`DELETE FROM ${FTS_TABLE}`);
		const ids = sqlite
			.prepare(`SELECT id FROM archived_emails ORDER BY archived_at ASC`)
			.all() as Array<{ id: string }>;
		const BATCH = 500;
		let batches = 0;
		for (let i = 0; i < ids.length; i += BATCH) {
			await sendJob('indexing', 'index-email-batch', {
				emails: ids.slice(i, i + BATCH).map((row) => ({ archivedEmailId: row.id })),
			});
			batches++;
		}
		logger.info({ totalEmails: ids.length, batches }, 'Search index rebuild enqueued');
		return { enqueuedBatches: batches, totalEmails: ids.length };
	}

	public async searchEmails(
		dto: SearchQuery,
		userId: string,
		actorIp: string
	): Promise<SearchResult> {
		return this.queryArchivedEmails(
			{
				query: dto.query,
				filters: dto.filters as ArchiveQueryFilters,
				fields: dto.fields,
				sort: dto.sort,
				direction: dto.direction,
				page: dto.page,
				limit: dto.limit,
				matchingStrategy: dto.matchingStrategy,
			},
			userId,
			actorIp
		);
	}

	public async queryArchivedEmails(
		dto: ArchiveQuery,
		_userId: string,
		_actorIp: string
	): Promise<SearchResult> {
		this.ensureReady();
		const startedAt = Date.now();
		const query = (dto.query || '').trim();
		const page = clampPositiveInteger(dto.page, 1, Number.MAX_SAFE_INTEGER);
		const limit = clampPositiveInteger(dto.limit, 10, 100);
		const fields = normalizeSearchFields(dto.fields);
		const sortColumn = SORT_COLUMN_MAP[dto.sort || 'sentAt'];
		const direction = dto.direction === 'asc' ? 'ASC' : 'DESC';
		// 'all' = every word must match; 'last'/'frequency' (Meili word-dropping
		// strategies) are approximated with an any-word fallback pass.
		const strict = dto.matchingStrategy === 'all';

		const filterSql = await this.buildFilterSql(dto.filters);

		const run = (match: string | null) => {
			if (match) {
				const base = `
					FROM ${FTS_TABLE} f
					JOIN archived_emails ae ON ae.rowid = f.rowid
					WHERE ${FTS_TABLE} MATCH ?${filterSql.clause}`;
				const hits = sqlite
					.prepare(
						`SELECT ae.*, snippet(${FTS_TABLE}, 2, '', '', '…', 24) AS snippet ${base}
						ORDER BY ${sortColumn} ${direction}, bm25(${FTS_TABLE}, ${BM25_WEIGHTS}) ASC
						LIMIT ? OFFSET ?`
					)
					.all(match, ...filterSql.params, limit, (page - 1) * limit) as EmailRow[];
				const [{ total }] = sqlite
					.prepare(`SELECT count(*) AS total ${base}`)
					.all(match, ...filterSql.params) as Array<{ total: number }>;
				return { hits, total };
			}
			const base = `FROM archived_emails ae WHERE 1=1${filterSql.clause}`;
			const hits = sqlite
				.prepare(
					`SELECT ae.* ${base} ORDER BY ${sortColumn} ${direction} LIMIT ? OFFSET ?`
				)
				.all(...filterSql.params, limit, (page - 1) * limit) as EmailRow[];
			const [{ total }] = sqlite
				.prepare(`SELECT count(*) AS total ${base}`)
				.all(...filterSql.params) as Array<{ total: number }>;
			return { hits, total };
		};

		let result = run(query ? buildMatchExpression(query, fields, 'and') : null);
		if (query && result.total === 0 && !strict) {
			const orMatch = buildMatchExpression(query, fields, 'or');
			// Multi-word queries only — a single word has nothing to relax.
			if (orMatch && / OR /.test(orMatch)) {
				result = run(orMatch);
			}
		}

		return {
			hits: result.hits.map(rowToDocument),
			total: result.total,
			page,
			limit,
			totalPages: Math.ceil(result.total / limit),
			processingTimeMs: Date.now() - startedAt,
		};
	}

	/** Translates the (Meili-era) filter object into SQL over archived_emails. */
	private async buildFilterSql(
		filters: ArchiveQueryFilters | undefined
	): Promise<{ clause: string; params: unknown[] }> {
		const parts: string[] = [];
		const params: unknown[] = [];
		if (!filters) {
			return { clause: '', params };
		}

		if (filters.ingestionSourceId) {
			const groupIds = await IngestionService.findGroupSourceIds(filters.ingestionSourceId);
			parts.push(
				`ae.ingestion_source_id IN (${groupIds.map(() => '?').join(', ')})`
			);
			params.push(...groupIds);
		}
		const equals: Array<[key: 'userEmail' | 'from' | 'sourcePath', column: string]> = [
			['userEmail', 'ae.user_email'],
			['from', 'ae.sender_email'],
			['sourcePath', 'ae.source_path'],
		];
		for (const [key, column] of equals) {
			const value = filters[key];
			if (typeof value === 'string' && value.length > 0) {
				parts.push(`${column} = ?`);
				params.push(value);
			}
		}
		// Recipient filters match the address anywhere in the recipients JSON.
		for (const key of ['to', 'cc', 'bcc'] as const) {
			const value = filters[key];
			if (typeof value === 'string' && value.length > 0) {
				parts.push(
					`EXISTS (SELECT 1 FROM json_each(ae.recipients, '$.' || ?) r WHERE json_extract(r.value, '$.address') = ?)`
				);
				params.push(key, value);
			}
		}
		if (typeof filters.hasAttachments === 'boolean') {
			parts.push(`ae.has_attachments = ?`);
			params.push(filters.hasAttachments ? 1 : 0);
		}
		for (const [key, column] of [
			['sourceLabels', 'ae.source_labels'],
			['tags', 'ae.tags'],
		] as const) {
			const values = filters[key];
			if (Array.isArray(values)) {
				for (const value of values) {
					if (typeof value === 'string' && value.length > 0) {
						parts.push(
							`EXISTS (SELECT 1 FROM json_each(COALESCE(${column}, '[]')) j WHERE j.value = ?)`
						);
						params.push(value);
					}
				}
			}
		}
		const ranges: Array<[value: number | null, clause: string]> = [
			[toTimestamp(filters.sentAfter), 'ae.sent_at >= ?'],
			[toTimestamp(filters.sentBefore), 'ae.sent_at <= ?'],
			[toTimestamp(filters.archivedAfter), 'ae.archived_at >= ?'],
			[toTimestamp(filters.archivedBefore), 'ae.archived_at <= ?'],
		];
		for (const [value, clause] of ranges) {
			if (value !== null) {
				parts.push(clause);
				params.push(value);
			}
		}
		return { clause: parts.length ? ` AND ${parts.join(' AND ')}` : '', params };
	}

	/** Top senders across the whole archive (dashboard). Uncapped, unlike Meili's 100-value facet limit. */
	public async getTopSenders(limit = 10): Promise<TopSender[]> {
		const rows = sqlite
			.prepare(
				`SELECT sender_email AS sender, count(*) AS count
				FROM archived_emails GROUP BY sender_email
				ORDER BY count DESC, sender ASC LIMIT ?`
			)
			.all(limit) as TopSender[];
		return rows;
	}

	/** Distinct tag values for the mailbox filter dropdown. Uncapped. */
	public async getFilterFacets(): Promise<{ tags: string[] }> {
		const rows = sqlite
			.prepare(
				`SELECT DISTINCT j.value AS tag
				FROM archived_emails ae, json_each(COALESCE(ae.tags, '[]')) j
				WHERE trim(j.value) <> '' ORDER BY tag COLLATE NOCASE ASC`
			)
			.all() as Array<{ tag: string }>;
		return { tags: rows.map((row) => row.tag) };
	}
}
