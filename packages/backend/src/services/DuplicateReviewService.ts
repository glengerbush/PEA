import { inArray, sql } from 'drizzle-orm';
import { db } from '../database';
import { archivedEmails, fuzzyDuplicateGroups } from '../database/schema';
import { AuditService } from './AuditService';
import { SearchService } from './SearchService';
import { indexingQueue } from '../jobs/queues';
import type {
	ApproveExactDuplicateGroupDto,
	ApproveExactDuplicatesResult,
	ApproveFuzzyDuplicateGroupDto,
	ApproveFuzzyDuplicatesResult,
	ExactDuplicateEmail,
	ExactDuplicateGroup,
	ExactDuplicateGroupsResult,
	ExactDuplicateReason,
	FuzzyDuplicateEmail,
	FuzzyDuplicateGroupsResult,
	FuzzyDuplicateScanResult,
	FuzzyDuplicateSignals,
	IgnoreFuzzyDuplicateGroupsResult,
	ScanFuzzyDuplicatesResult,
} from '@open-archiver/types';

type RawGroupRow = {
	reason: ExactDuplicateReason;
	fingerprint: string;
	count: number | string | bigint;
};

type RawEmailRow = {
	id: string;
	subject: string | null;
	sender_name: string | null;
	sender_email: string;
	user_email: string;
	sent_at: Date | string;
	archived_at: Date | string;
	has_attachments: boolean;
	source_path: string | null;
	local_folder_path: string | null;
	message_id_header: string | null;
	storage_hash_sha256: string;
	duplicate_of_email_id: string | null;
	duplicate_review_status: string;
	is_duplicate_hidden: boolean;
};

type RawFuzzyGroupRow = {
	id: string;
	group_key: string;
	status: 'pending' | 'approved' | 'ignored';
	score: number;
	signals: FuzzyDuplicateSignals;
	created_at: Date | string;
	updated_at: Date | string;
};

type RawFuzzyEmailRow = RawEmailRow & {
	suggested_keeper: boolean;
};

const DEFAULT_LIMIT = 25;
const MAX_LIMIT = 100;
const DEFAULT_FUZZY_SCAN_BATCH_SIZE = 100;
const MAX_FUZZY_SCAN_BATCH_SIZE = 500;
const INDEX_UPDATE_BATCH_SIZE = 500;

function clampPositiveInteger(value: number | undefined, fallback: number, max: number): number {
	if (!value || !Number.isFinite(value) || value < 1) {
		return fallback;
	}
	return Math.min(Math.floor(value), max);
}

function toRows<T>(result: unknown): T[] {
	if (Array.isArray(result)) return result as T[];
	const maybeRows = (result as { rows?: T[] }).rows;
	return Array.isArray(maybeRows) ? maybeRows : [];
}

function toNumber(value: number | string | bigint): number {
	return typeof value === 'bigint' ? Number(value) : Number(value);
}

function groupKey(reason: ExactDuplicateReason, fingerprint: string): string {
	return `${reason}:${fingerprint}`;
}

function mapEmail(row: RawEmailRow): ExactDuplicateEmail {
	return {
		id: row.id,
		subject: row.subject,
		senderName: row.sender_name,
		senderEmail: row.sender_email,
		userEmail: row.user_email,
		sentAt: new Date(row.sent_at),
		archivedAt: new Date(row.archived_at),
		hasAttachments: row.has_attachments,
		sourcePath: row.source_path,
		localFolderPath: row.local_folder_path,
		messageIdHeader: row.message_id_header,
		storageHashSha256: row.storage_hash_sha256,
		duplicateOfEmailId: row.duplicate_of_email_id,
		duplicateReviewStatus: row.duplicate_review_status,
		isDuplicateHidden: row.is_duplicate_hidden,
	};
}

function mapFuzzyEmail(row: RawFuzzyEmailRow): FuzzyDuplicateEmail {
	return {
		...mapEmail(row),
		suggestedKeeper: row.suggested_keeper,
	};
}

export class DuplicateReviewService {
	private static auditService = new AuditService();
	private static searchService = new SearchService();

	public static async listExactDuplicateGroups(
		page?: number,
		limit?: number
	): Promise<ExactDuplicateGroupsResult> {
		const normalizedPage = clampPositiveInteger(page, 1, Number.MAX_SAFE_INTEGER);
		const normalizedLimit = clampPositiveInteger(limit, DEFAULT_LIMIT, MAX_LIMIT);
		const offset = (normalizedPage - 1) * normalizedLimit;

		const totalRows = toRows<{ total_groups: number | string | bigint }>(
			await db.execute(sql`
				WITH attachment_sets AS (
					SELECT
						ae.id AS email_id,
						string_agg(a.content_hash_sha256, ',' ORDER BY a.content_hash_sha256) AS fingerprint
					FROM archived_emails ae
					JOIN email_attachments ea ON ea.email_id = ae.id
					JOIN attachments a ON a.id = ea.attachment_id
					WHERE ae.is_duplicate_hidden = false
					GROUP BY ae.id
					HAVING count(a.id) > 0
				),
				exact_groups AS (
					SELECT 'message_id'::text AS reason, message_id_header::text AS fingerprint
					FROM archived_emails
					WHERE is_duplicate_hidden = false
						AND message_id_header IS NOT NULL
						AND message_id_header <> ''
					GROUP BY message_id_header
					HAVING count(*) > 1
					UNION ALL
					SELECT 'storage_hash'::text AS reason, storage_hash_sha256::text AS fingerprint
					FROM archived_emails
					WHERE is_duplicate_hidden = false
						AND storage_hash_sha256 IS NOT NULL
						AND storage_hash_sha256 <> ''
					GROUP BY storage_hash_sha256
					HAVING count(*) > 1
					UNION ALL
					SELECT 'attachment_hash_set'::text AS reason, fingerprint::text AS fingerprint
					FROM attachment_sets
					WHERE fingerprint IS NOT NULL AND fingerprint <> ''
					GROUP BY fingerprint
					HAVING count(*) > 1
				)
				SELECT count(*) AS total_groups FROM exact_groups
			`)
		);

		const groupRows = toRows<RawGroupRow>(
			await db.execute(sql`
				WITH attachment_sets AS (
					SELECT
						ae.id AS email_id,
						string_agg(a.content_hash_sha256, ',' ORDER BY a.content_hash_sha256) AS fingerprint
					FROM archived_emails ae
					JOIN email_attachments ea ON ea.email_id = ae.id
					JOIN attachments a ON a.id = ea.attachment_id
					WHERE ae.is_duplicate_hidden = false
					GROUP BY ae.id
					HAVING count(a.id) > 0
				),
				exact_groups AS (
					SELECT
						'message_id'::text AS reason,
						message_id_header::text AS fingerprint,
						count(*) AS count
					FROM archived_emails
					WHERE is_duplicate_hidden = false
						AND message_id_header IS NOT NULL
						AND message_id_header <> ''
					GROUP BY message_id_header
					HAVING count(*) > 1
					UNION ALL
					SELECT
						'storage_hash'::text AS reason,
						storage_hash_sha256::text AS fingerprint,
						count(*) AS count
					FROM archived_emails
					WHERE is_duplicate_hidden = false
						AND storage_hash_sha256 IS NOT NULL
						AND storage_hash_sha256 <> ''
					GROUP BY storage_hash_sha256
					HAVING count(*) > 1
					UNION ALL
					SELECT
						'attachment_hash_set'::text AS reason,
						fingerprint::text AS fingerprint,
						count(*) AS count
					FROM attachment_sets
					WHERE fingerprint IS NOT NULL AND fingerprint <> ''
					GROUP BY fingerprint
					HAVING count(*) > 1
				)
				SELECT reason, fingerprint, count
				FROM exact_groups
				ORDER BY count DESC, reason ASC, fingerprint ASC
				LIMIT ${normalizedLimit}
				OFFSET ${offset}
			`)
		);

		const groups = await Promise.all(
			groupRows.map(async (row): Promise<ExactDuplicateGroup> => {
				const emails = await this.findEmailsForGroup(row.reason, row.fingerprint);
				return {
					groupKey: groupKey(row.reason, row.fingerprint),
					reason: row.reason,
					fingerprint: row.fingerprint,
					count: toNumber(row.count),
					keeperEmailId: emails[0]?.id || '',
					emails,
				};
			})
		);

		return {
			groups: groups.filter((group) => group.emails.length > 1 && group.keeperEmailId),
			totalGroups: totalRows[0] ? toNumber(totalRows[0].total_groups) : 0,
			page: normalizedPage,
			limit: normalizedLimit,
		};
	}

	public static async approveExactDuplicateGroups(
		groups: ApproveExactDuplicateGroupDto[],
		userId: string,
		actorIp: string
	): Promise<ApproveExactDuplicatesResult> {
		const indexUpdates: {
			id: string;
			duplicateOfEmailId: string | null;
			duplicateReviewStatus: string;
			isDuplicateHidden: boolean;
		}[] = [];
		let approvedGroups = 0;
		let hiddenEmails = 0;
		let keeperEmails = 0;

		for (const group of groups) {
			const keeperEmailId = group.keeperEmailId;
			const duplicateEmailIds: string[] = Array.from(
				new Set<string>(
					group.duplicateEmailIds.filter(
						(id): id is string => typeof id === 'string' && id !== keeperEmailId
					)
				)
			);
			if (!keeperEmailId || duplicateEmailIds.length === 0) {
				continue;
			}

			const [keeper] = await db
				.update(archivedEmails)
				.set({
					duplicateOfEmailId: null,
					duplicateReviewStatus: 'keeper',
					isDuplicateHidden: false,
				})
				.where(inArray(archivedEmails.id, [keeperEmailId]))
				.returning({ id: archivedEmails.id });

			const duplicates = await db
				.update(archivedEmails)
				.set({
					duplicateOfEmailId: keeperEmailId,
					duplicateReviewStatus: 'approved_duplicate',
					isDuplicateHidden: true,
				})
				.where(inArray(archivedEmails.id, duplicateEmailIds))
				.returning({ id: archivedEmails.id });

			if (keeper) {
				keeperEmails += 1;
				indexUpdates.push({
					id: keeper.id,
					duplicateOfEmailId: null,
					duplicateReviewStatus: 'keeper',
					isDuplicateHidden: false,
				});
			}

			for (const duplicate of duplicates) {
				hiddenEmails += 1;
				indexUpdates.push({
					id: duplicate.id,
					duplicateOfEmailId: keeperEmailId,
					duplicateReviewStatus: 'approved_duplicate',
					isDuplicateHidden: true,
				});
			}

			approvedGroups += 1;
		}

		for (let i = 0; i < indexUpdates.length; i += INDEX_UPDATE_BATCH_SIZE) {
			await this.searchService.updateDocuments(
				'emails',
				indexUpdates.slice(i, i + INDEX_UPDATE_BATCH_SIZE),
				'id'
			);
		}

		if (approvedGroups > 0) {
			await this.auditService.createAuditLog({
				actorIdentifier: userId,
				actionType: 'UPDATE',
				targetType: 'ArchivedEmail',
				targetId: 'bulk',
				actorIp,
				details: {
					action: 'APPROVE_EXACT_DUPLICATES',
					approvedGroups,
					hiddenEmails,
					keeperEmails,
				},
			});
		}

		return { approvedGroups, hiddenEmails, keeperEmails };
	}

	public static async enqueueFuzzyDuplicateScan(
		batchSize?: number
	): Promise<ScanFuzzyDuplicatesResult> {
		const normalizedBatchSize = clampPositiveInteger(
			batchSize,
			DEFAULT_FUZZY_SCAN_BATCH_SIZE,
			MAX_FUZZY_SCAN_BATCH_SIZE
		);
		const job = await indexingQueue.add('scan-fuzzy-duplicates', {
			batchSize: normalizedBatchSize,
		});
		return {
			jobId: job.id || '',
			batchSize: normalizedBatchSize,
		};
	}

	public static async scanFuzzyDuplicateBatch(
		batchSize?: number
	): Promise<FuzzyDuplicateScanResult> {
		const normalizedBatchSize = clampPositiveInteger(
			batchSize,
			DEFAULT_FUZZY_SCAN_BATCH_SIZE,
			MAX_FUZZY_SCAN_BATCH_SIZE
		);

		const resultRows = toRows<{
			scanned_groups: number | string | bigint;
			inserted_groups: number | string | bigint;
			linked_emails: number | string | bigint;
		}>(
			await db.execute(sql`
				WITH candidate_base AS (
					SELECT
						ae.duplicate_fuzzy_group_key AS group_key,
						min(lower(ae.sender_email)) AS sender_email,
						min(ae.duplicate_subject_hash) AS duplicate_subject_hash,
						count(*) AS email_count,
						min(ae.sent_at) AS min_sent_at,
						max(ae.sent_at) AS max_sent_at,
						count(ae.duplicate_body_hash) AS body_hash_present_count,
						count(DISTINCT ae.duplicate_body_hash) FILTER (
							WHERE ae.duplicate_body_hash IS NOT NULL
						) AS body_hash_count,
						count(ae.duplicate_recipient_fingerprint) AS recipient_hash_present_count,
						count(DISTINCT ae.duplicate_recipient_fingerprint) FILTER (
							WHERE ae.duplicate_recipient_fingerprint IS NOT NULL
						) AS recipient_hash_count,
						count(ae.duplicate_attachment_fingerprint) AS attachment_hash_present_count,
						count(DISTINCT ae.duplicate_attachment_fingerprint) FILTER (
							WHERE ae.duplicate_attachment_fingerprint IS NOT NULL
						) AS attachment_hash_count
					FROM archived_emails ae
					WHERE ae.is_duplicate_hidden = false
						AND ae.duplicate_fuzzy_group_key IS NOT NULL
						AND NOT EXISTS (
							SELECT 1
							FROM fuzzy_duplicate_groups fdg
							WHERE fdg.group_key = ae.duplicate_fuzzy_group_key
								AND fdg.status IN ('approved', 'ignored')
						)
					GROUP BY ae.duplicate_fuzzy_group_key
					HAVING count(*) > 1
				),
				scored_candidates AS (
					SELECT
						group_key,
						sender_email,
						duplicate_subject_hash,
						email_count,
						min_sent_at,
						max_sent_at,
						body_hash_present_count,
						body_hash_count,
						recipient_hash_present_count,
						recipient_hash_count,
						attachment_hash_present_count,
						attachment_hash_count,
						(
							45
							+ CASE WHEN body_hash_present_count = email_count AND body_hash_count = 1 THEN 20 ELSE 0 END
							+ CASE WHEN recipient_hash_present_count = email_count AND recipient_hash_count = 1 THEN 15 ELSE 0 END
							+ CASE WHEN attachment_hash_present_count = email_count AND attachment_hash_count = 1 THEN 10 ELSE 0 END
							+ CASE WHEN max_sent_at - min_sent_at <= interval '48 hours' THEN 10 ELSE 0 END
						)::integer AS score
					FROM candidate_base
				),
				candidate_groups AS (
					SELECT *
					FROM scored_candidates
					WHERE score >= 55
					ORDER BY score DESC, email_count DESC, group_key ASC
					LIMIT ${normalizedBatchSize}
				),
				upserted_groups AS (
					INSERT INTO fuzzy_duplicate_groups (group_key, status, score, signals)
					SELECT
						group_key,
						'pending',
						score,
						jsonb_build_object(
							'senderEmail', sender_email,
							'subjectHash', duplicate_subject_hash,
							'matchingBodyHash', body_hash_present_count = email_count AND body_hash_count = 1,
							'matchingRecipients', recipient_hash_present_count = email_count AND recipient_hash_count = 1,
							'matchingAttachments', attachment_hash_present_count = email_count AND attachment_hash_count = 1,
							'sentSpreadHours', extract(epoch from (max_sent_at - min_sent_at)) / 3600
						)
					FROM candidate_groups
					ON CONFLICT (group_key) DO UPDATE SET
						score = EXCLUDED.score,
						signals = EXCLUDED.signals,
						updated_at = now()
					WHERE fuzzy_duplicate_groups.status = 'pending'
					RETURNING id, group_key
				),
				linked_emails AS (
					INSERT INTO fuzzy_duplicate_group_emails (group_id, email_id, suggested_keeper)
					SELECT
						ug.id,
						ae.id,
						ae.id = (
							SELECT keeper.id
							FROM archived_emails keeper
							WHERE keeper.is_duplicate_hidden = false
								AND keeper.duplicate_fuzzy_group_key = ug.group_key
							ORDER BY keeper.sent_at ASC, keeper.archived_at ASC, keeper.id ASC
							LIMIT 1
						)
					FROM upserted_groups ug
					JOIN archived_emails ae
						ON ae.is_duplicate_hidden = false
						AND ae.duplicate_fuzzy_group_key = ug.group_key
					ON CONFLICT DO NOTHING
					RETURNING group_id, email_id
				)
				SELECT
					(SELECT count(*) FROM candidate_groups) AS scanned_groups,
					(SELECT count(*) FROM upserted_groups) AS inserted_groups,
					(SELECT count(*) FROM linked_emails) AS linked_emails
			`)
		);

		const result = resultRows[0];
		return {
			scannedGroups: result ? toNumber(result.scanned_groups) : 0,
			insertedGroups: result ? toNumber(result.inserted_groups) : 0,
			linkedEmails: result ? toNumber(result.linked_emails) : 0,
		};
	}

	public static async listFuzzyDuplicateGroups(
		page?: number,
		limit?: number
	): Promise<FuzzyDuplicateGroupsResult> {
		const normalizedPage = clampPositiveInteger(page, 1, Number.MAX_SAFE_INTEGER);
		const normalizedLimit = clampPositiveInteger(limit, DEFAULT_LIMIT, MAX_LIMIT);
		const offset = (normalizedPage - 1) * normalizedLimit;

		const totalRows = toRows<{ total_groups: number | string | bigint }>(
			await db.execute(sql`
				SELECT count(*) AS total_groups
				FROM fuzzy_duplicate_groups
				WHERE status = 'pending'
			`)
		);

		const groupRows = toRows<RawFuzzyGroupRow>(
			await db.execute(sql`
				SELECT id, group_key, status, score, signals, created_at, updated_at
				FROM fuzzy_duplicate_groups
				WHERE status = 'pending'
				ORDER BY score DESC, updated_at DESC, group_key ASC
				LIMIT ${normalizedLimit}
				OFFSET ${offset}
			`)
		);

		const groups = await Promise.all(
			groupRows.map(async (group) => {
				const emails = await this.findEmailsForFuzzyGroup(group.id);
				const keeperEmailId =
					emails.find((email) => email.suggestedKeeper)?.id || emails[0]?.id || '';
				return {
					id: group.id,
					groupKey: group.group_key,
					status: group.status,
					score: group.score,
					signals: group.signals,
					createdAt: new Date(group.created_at),
					updatedAt: new Date(group.updated_at),
					keeperEmailId,
					emails,
				};
			})
		);

		return {
			groups: groups.filter((group) => group.emails.length > 1 && group.keeperEmailId),
			totalGroups: totalRows[0] ? toNumber(totalRows[0].total_groups) : 0,
			page: normalizedPage,
			limit: normalizedLimit,
		};
	}

	public static async approveFuzzyDuplicateGroups(
		groups: ApproveFuzzyDuplicateGroupDto[],
		userId: string,
		actorIp: string
	): Promise<ApproveFuzzyDuplicatesResult> {
		const indexUpdates: {
			id: string;
			duplicateOfEmailId: string | null;
			duplicateReviewStatus: string;
			isDuplicateHidden: boolean;
		}[] = [];
		let approvedGroups = 0;
		let hiddenEmails = 0;
		let keeperEmails = 0;

		for (const group of groups) {
			const keeperEmailId = group.keeperEmailId;
			const duplicateEmailIds: string[] = Array.from(
				new Set<string>(
					group.duplicateEmailIds.filter(
						(id): id is string => typeof id === 'string' && id !== keeperEmailId
					)
				)
			);
			if (!group.groupId || !keeperEmailId || duplicateEmailIds.length === 0) {
				continue;
			}

			const [keeper] = await db
				.update(archivedEmails)
				.set({
					duplicateOfEmailId: null,
					duplicateReviewStatus: 'keeper',
					isDuplicateHidden: false,
				})
				.where(inArray(archivedEmails.id, [keeperEmailId]))
				.returning({ id: archivedEmails.id });

			const duplicates = await db
				.update(archivedEmails)
				.set({
					duplicateOfEmailId: keeperEmailId,
					duplicateReviewStatus: 'approved_duplicate',
					isDuplicateHidden: true,
				})
				.where(inArray(archivedEmails.id, duplicateEmailIds))
				.returning({ id: archivedEmails.id });

			await db
				.update(fuzzyDuplicateGroups)
				.set({ status: 'approved', updatedAt: new Date() })
				.where(inArray(fuzzyDuplicateGroups.id, [group.groupId]));

			if (keeper) {
				keeperEmails += 1;
				indexUpdates.push({
					id: keeper.id,
					duplicateOfEmailId: null,
					duplicateReviewStatus: 'keeper',
					isDuplicateHidden: false,
				});
			}

			for (const duplicate of duplicates) {
				hiddenEmails += 1;
				indexUpdates.push({
					id: duplicate.id,
					duplicateOfEmailId: keeperEmailId,
					duplicateReviewStatus: 'approved_duplicate',
					isDuplicateHidden: true,
				});
			}

			approvedGroups += 1;
		}

		for (let i = 0; i < indexUpdates.length; i += INDEX_UPDATE_BATCH_SIZE) {
			await this.searchService.updateDocuments(
				'emails',
				indexUpdates.slice(i, i + INDEX_UPDATE_BATCH_SIZE),
				'id'
			);
		}

		if (approvedGroups > 0) {
			await this.auditService.createAuditLog({
				actorIdentifier: userId,
				actionType: 'UPDATE',
				targetType: 'ArchivedEmail',
				targetId: 'bulk',
				actorIp,
				details: {
					action: 'APPROVE_FUZZY_DUPLICATES',
					approvedGroups,
					hiddenEmails,
					keeperEmails,
				},
			});
		}

		return { approvedGroups, hiddenEmails, keeperEmails };
	}

	public static async ignoreFuzzyDuplicateGroups(
		groupIds: string[]
	): Promise<IgnoreFuzzyDuplicateGroupsResult> {
		const uniqueGroupIds = Array.from(new Set(groupIds.filter(Boolean)));
		if (uniqueGroupIds.length === 0) {
			return { ignoredGroups: 0 };
		}

		const ignored = await db
			.update(fuzzyDuplicateGroups)
			.set({ status: 'ignored', updatedAt: new Date() })
			.where(inArray(fuzzyDuplicateGroups.id, uniqueGroupIds))
			.returning({ id: fuzzyDuplicateGroups.id });

		return { ignoredGroups: ignored.length };
	}

	private static async findEmailsForGroup(
		reason: ExactDuplicateReason,
		fingerprint: string
	): Promise<ExactDuplicateEmail[]> {
		if (reason === 'attachment_hash_set') {
			return this.findEmailsByAttachmentSet(fingerprint);
		}

		const column =
			reason === 'message_id'
				? sql`message_id_header = ${fingerprint}`
				: sql`storage_hash_sha256 = ${fingerprint}`;

		const rows = toRows<RawEmailRow>(
			await db.execute(sql`
				SELECT
					id,
					subject,
					sender_name,
					sender_email,
					user_email,
					sent_at,
					archived_at,
					has_attachments,
					source_path,
					local_folder_path,
					message_id_header,
					storage_hash_sha256,
					duplicate_of_email_id,
					duplicate_review_status,
					is_duplicate_hidden
				FROM archived_emails
				WHERE is_duplicate_hidden = false AND ${column}
				ORDER BY sent_at ASC, archived_at ASC, id ASC
			`)
		);

		return rows.map(mapEmail);
	}

	private static async findEmailsByAttachmentSet(
		fingerprint: string
	): Promise<ExactDuplicateEmail[]> {
		const rows = toRows<RawEmailRow>(
			await db.execute(sql`
				WITH email_sets AS (
					SELECT
						ae.id AS email_id,
						string_agg(a.content_hash_sha256, ',' ORDER BY a.content_hash_sha256) AS fingerprint
					FROM archived_emails ae
					JOIN email_attachments ea ON ea.email_id = ae.id
					JOIN attachments a ON a.id = ea.attachment_id
					WHERE ae.is_duplicate_hidden = false
					GROUP BY ae.id
					HAVING count(a.id) > 0
				)
				SELECT
					ae.id,
					ae.subject,
					ae.sender_name,
					ae.sender_email,
					ae.user_email,
					ae.sent_at,
					ae.archived_at,
					ae.has_attachments,
					ae.source_path,
					ae.local_folder_path,
					ae.message_id_header,
					ae.storage_hash_sha256,
					ae.duplicate_of_email_id,
					ae.duplicate_review_status,
					ae.is_duplicate_hidden
				FROM email_sets es
				JOIN archived_emails ae ON ae.id = es.email_id
				WHERE es.fingerprint = ${fingerprint}
					AND ae.is_duplicate_hidden = false
				ORDER BY ae.sent_at ASC, ae.archived_at ASC, ae.id ASC
			`)
		);

		return rows.map(mapEmail);
	}

	private static async findEmailsForFuzzyGroup(groupId: string): Promise<FuzzyDuplicateEmail[]> {
		const rows = toRows<RawFuzzyEmailRow>(
			await db.execute(sql`
				SELECT
					ae.id,
					ae.subject,
					ae.sender_name,
					ae.sender_email,
					ae.user_email,
					ae.sent_at,
					ae.archived_at,
					ae.has_attachments,
					ae.source_path,
					ae.local_folder_path,
					ae.message_id_header,
					ae.storage_hash_sha256,
					ae.duplicate_of_email_id,
					ae.duplicate_review_status,
					ae.is_duplicate_hidden,
					fge.suggested_keeper
				FROM fuzzy_duplicate_group_emails fge
				JOIN archived_emails ae ON ae.id = fge.email_id
				WHERE fge.group_id = ${groupId}
					AND ae.is_duplicate_hidden = false
				ORDER BY fge.suggested_keeper DESC, ae.sent_at ASC, ae.archived_at ASC, ae.id ASC
			`)
		);

		return rows.map(mapFuzzyEmail);
	}
}
