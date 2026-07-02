import { asc, eq, inArray, sql } from 'drizzle-orm';
import { db, sqlite } from '../database';
import { archivedEmails, fuzzyDuplicateGroups } from '../database/schema';
import { ArchivedEmailService } from './ArchivedEmailService';
import { UserService } from './UserService';
import { logger } from '../config/logger';
import { sendJob } from '../jobs/queue';
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
	User,
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
	message_id_header: string | null;
	storage_hash_sha256: string;
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
		// SQLite booleans surface as 0/1 in raw queries
		hasAttachments: Boolean(row.has_attachments),
		sourcePath: row.source_path,
		messageIdHeader: row.message_id_header,
		storageHashSha256: row.storage_hash_sha256,
	};
}

function mapFuzzyEmail(row: RawFuzzyEmailRow): FuzzyDuplicateEmail {
	return {
		...mapEmail(row),
		suggestedKeeper: Boolean(row.suggested_keeper),
	};
}

// From + recipients + exact send time. Catches duplicates whose body/subject or
// styling differ (e.g. broken HTML) but whose headers are identical — if the
// sender, the full recipient set, and the send timestamp all match, it's almost
// certainly the same message. (A grouping key only needs determinism, not
// hashing — SQLite has no md5(), so the raw concatenation is the key.)
function headersFingerprintSql() {
	return sql`lower(coalesce(sender_email, '')) || '|' || coalesce(duplicate_recipient_fingerprint, '') || '|' || CAST(sent_at AS TEXT)`;
}

export class DuplicateReviewService {

	public static async listExactDuplicateGroups(
		page?: number,
		limit?: number,
		reason?: string
	): Promise<ExactDuplicateGroupsResult> {
		const normalizedPage = clampPositiveInteger(page, 1, Number.MAX_SAFE_INTEGER);
		const normalizedLimit = clampPositiveInteger(limit, DEFAULT_LIMIT, MAX_LIMIT);
		const offset = (normalizedPage - 1) * normalizedLimit;

		// Pull every email's duplicate signals in one pass, then group by CONNECTED
		// COMPONENT (union-find). A cluster that matches several signals (e.g. same
		// Message-ID AND same raw hash) becomes ONE group tagged with all matching
		// reasons — not one overlapping group per signal. Scoped to a personal-size
		// archive; a very large archive would want an incremental approach.
		const signalRows = toRows<{
			id: string;
			message_id: string | null;
			storage_hash: string | null;
			attachment_fp: string | null;
			headers_fp: string | null;
		}>(
			db.all(sql`
				WITH attachment_sets AS (
					SELECT ae.id AS email_id,
						group_concat(a.content_hash_sha256, ',' ORDER BY a.content_hash_sha256) AS att_fp
					FROM archived_emails ae
					JOIN email_attachments ea ON ea.email_id = ae.id
					JOIN attachments a ON a.id = ea.attachment_id
					GROUP BY ae.id
					HAVING count(a.id) > 0
				)
				SELECT ae.id AS id,
					nullif(ae.message_id_header, '') AS message_id,
					nullif(ae.storage_hash_sha256, '') AS storage_hash,
					s.att_fp AS attachment_fp,
					CASE
						WHEN ae.sender_email IS NOT NULL AND ae.sender_email <> ''
							AND ae.duplicate_recipient_fingerprint IS NOT NULL
						THEN ${headersFingerprintSql()}
					END AS headers_fp
				FROM archived_emails ae
				LEFT JOIN attachment_sets s ON s.email_id = ae.id
			`)
		);

		// Reason priority (strongest first) — also the primary-reason order.
		const REASON_KEYS: {
			key: 'storage_hash' | 'message_id' | 'attachment_fp' | 'headers_fp';
			reason: ExactDuplicateReason;
		}[] = [
			{ key: 'storage_hash', reason: 'storage_hash' },
			{ key: 'message_id', reason: 'message_id' },
			{ key: 'attachment_fp', reason: 'attachment_hash_set' },
			{ key: 'headers_fp', reason: 'sender_recipients_sent' },
		];

		// value → member email ids, per signal (used for union + reason detection).
		const byKeyValue: Record<string, Map<string, string[]>> = {
			storage_hash: new Map(),
			message_id: new Map(),
			attachment_fp: new Map(),
			headers_fp: new Map(),
		};
		const parent = new Map<string, string>();
		const find = (x: string): string => {
			let root = x;
			while (parent.get(root) !== root) root = parent.get(root) as string;
			let cur = x;
			while (parent.get(cur) !== root) {
				const next = parent.get(cur) as string;
				parent.set(cur, root);
				cur = next;
			}
			return root;
		};
		const union = (a: string, b: string) => {
			const ra = find(a);
			const rb = find(b);
			if (ra !== rb) parent.set(ra, rb);
		};

		for (const row of signalRows) {
			parent.set(row.id, row.id);
			for (const { key } of REASON_KEYS) {
				const value = row[key];
				if (!value) continue;
				const map = byKeyValue[key];
				const arr = map.get(value);
				if (arr) arr.push(row.id);
				else map.set(value, [row.id]);
			}
		}
		for (const { key } of REASON_KEYS) {
			for (const ids of byKeyValue[key].values()) {
				for (let i = 1; i < ids.length; i++) union(ids[0], ids[i]);
			}
		}

		// Assemble connected components (size ≥ 2 = a duplicate cluster).
		const components = new Map<string, string[]>();
		for (const row of signalRows) {
			const root = find(row.id);
			const arr = components.get(root);
			if (arr) arr.push(row.id);
			else components.set(root, [row.id]);
		}

		const minId = (ids: string[]) => ids.reduce((m, x) => (x < m ? x : m), ids[0]);
		const clusters: { ids: string[]; reasons: ExactDuplicateReason[] }[] = [];
		for (const ids of components.values()) {
			if (ids.length < 2) continue;
			const idSet = new Set(ids);
			const reasons: ExactDuplicateReason[] = [];
			for (const { key, reason } of REASON_KEYS) {
				const applies = [...byKeyValue[key].values()].some(
					(members) => members.filter((id) => idSet.has(id)).length >= 2
				);
				if (applies) reasons.push(reason);
			}
			clusters.push({ ids, reasons });
		}

		const filtered = reason
			? clusters.filter((c) => c.reasons.includes(reason as ExactDuplicateReason))
			: clusters;
		filtered.sort(
			(a, b) => b.ids.length - a.ids.length || (minId(a.ids) < minId(b.ids) ? -1 : 1)
		);

		const totalGroups = filtered.length;
		const pageClusters = filtered.slice(offset, offset + normalizedLimit);

		const groups = await Promise.all(
			pageClusters.map(async (cluster): Promise<ExactDuplicateGroup> => {
				const emails = await this.findEmailsByIds(cluster.ids);
				const key = minId(cluster.ids);
				const primary =
					REASON_KEYS.map((r) => r.reason).find((r) => cluster.reasons.includes(r)) ||
					cluster.reasons[0];
				return {
					groupKey: `cluster:${key}`,
					reason: primary,
					reasons: cluster.reasons,
					fingerprint: key,
					count: emails.length,
					keeperEmailId: emails[0]?.id || '',
					emails,
				};
			})
		);

		return {
			groups: groups.filter((group) => group.emails.length > 1 && group.keeperEmailId),
			totalGroups,
			page: normalizedPage,
			limit: normalizedLimit,
		};
	}

	private static async findEmailsByIds(ids: string[]): Promise<ExactDuplicateEmail[]> {
		if (ids.length === 0) return [];
		const rows = await db
			.select({
				id: archivedEmails.id,
				subject: archivedEmails.subject,
				sender_name: archivedEmails.senderName,
				sender_email: archivedEmails.senderEmail,
				user_email: archivedEmails.userEmail,
				sent_at: archivedEmails.sentAt,
				archived_at: archivedEmails.archivedAt,
				has_attachments: archivedEmails.hasAttachments,
				source_path: archivedEmails.sourcePath,
				message_id_header: archivedEmails.messageIdHeader,
				storage_hash_sha256: archivedEmails.storageHashSha256,
			})
			.from(archivedEmails)
			.where(inArray(archivedEmails.id, ids))
			.orderBy(asc(archivedEmails.sentAt), asc(archivedEmails.archivedAt), asc(archivedEmails.id));
		return rows.map((row) => mapEmail(row as unknown as RawEmailRow));
	}

	/**
	 * Permanently deletes the duplicate copies of a group, reusing the standard
	 * delete path (DB + search + storage + empty-folder cleanup). The keeper is
	 * preserved by the caller.
	 */
	private static async deleteDuplicateEmails(
		duplicateEmailIds: string[],
		actor: User,
		actorIp: string
	): Promise<number> {
		let deleted = 0;
		for (const emailId of duplicateEmailIds) {
			try {
				await ArchivedEmailService.deleteArchivedEmail(emailId, actor, actorIp);
				deleted += 1;
			} catch (error) {
				logger.warn(
					{ emailId, error: error instanceof Error ? error.message : String(error) },
					'Failed to delete duplicate email during approval'
				);
			}
		}
		return deleted;
	}

	public static async approveExactDuplicateGroups(
		groups: ApproveExactDuplicateGroupDto[],
		userId: string,
		actorIp: string
	): Promise<ApproveExactDuplicatesResult> {
		const actor = await new UserService().findById(userId);
		if (!actor) {
			throw new Error('Acting user not found');
		}
		let approvedGroups = 0;
		let deletedEmails = 0;
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
				.select({ id: archivedEmails.id })
				.from(archivedEmails)
				.where(inArray(archivedEmails.id, [keeperEmailId]));

			// Permanently delete the duplicate copies; keep the keeper.
			deletedEmails += await this.deleteDuplicateEmails(duplicateEmailIds, actor, actorIp);

			if (keeper) {
				keeperEmails += 1;
			}

			approvedGroups += 1;
		}

		return { approvedGroups, deletedEmails, keeperEmails };
	}

	public static async enqueueFuzzyDuplicateScan(
		batchSize?: number
	): Promise<ScanFuzzyDuplicatesResult> {
		const normalizedBatchSize = clampPositiveInteger(
			batchSize,
			DEFAULT_FUZZY_SCAN_BATCH_SIZE,
			MAX_FUZZY_SCAN_BATCH_SIZE
		);
		const jobId = await sendJob('indexing', 'scan-fuzzy-duplicates', {
			batchSize: normalizedBatchSize,
		});
		return {
			jobId: jobId || '',
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

		// Phase 1 (read-only): score candidate groups. sent_at is epoch-ms, so the
		// old `interval '48 hours'` becomes plain millisecond arithmetic. Note
		// count(DISTINCT x) already ignores NULLs, so no FILTER is needed.
		const candidates = toRows<{
			group_key: string;
			sender_email: string | null;
			duplicate_subject_hash: string | null;
			email_count: number;
			min_sent_at: number;
			max_sent_at: number;
			body_hash_present_count: number;
			body_hash_count: number;
			recipient_hash_present_count: number;
			recipient_hash_count: number;
			attachment_hash_present_count: number;
			attachment_hash_count: number;
			score: number;
		}>(
			db.all(sql`
				WITH candidate_base AS (
					SELECT
						ae.duplicate_fuzzy_group_key AS group_key,
						min(lower(ae.sender_email)) AS sender_email,
						min(ae.duplicate_subject_hash) AS duplicate_subject_hash,
						count(*) AS email_count,
						min(ae.sent_at) AS min_sent_at,
						max(ae.sent_at) AS max_sent_at,
						count(ae.duplicate_body_hash) AS body_hash_present_count,
						count(DISTINCT ae.duplicate_body_hash) AS body_hash_count,
						count(ae.duplicate_recipient_fingerprint) AS recipient_hash_present_count,
						count(DISTINCT ae.duplicate_recipient_fingerprint) AS recipient_hash_count,
						count(ae.duplicate_attachment_fingerprint) AS attachment_hash_present_count,
						count(DISTINCT ae.duplicate_attachment_fingerprint) AS attachment_hash_count
					FROM archived_emails ae
					WHERE ae.duplicate_fuzzy_group_key IS NOT NULL
						AND NOT EXISTS (
							SELECT 1
							FROM fuzzy_duplicate_groups fdg
							WHERE fdg.group_key = ae.duplicate_fuzzy_group_key
								AND fdg.status IN ('approved', 'ignored')
						)
					GROUP BY ae.duplicate_fuzzy_group_key
					HAVING count(*) > 1
				)
				SELECT *,
					(
						45
						+ CASE WHEN body_hash_present_count = email_count AND body_hash_count = 1 THEN 20 ELSE 0 END
						+ CASE WHEN recipient_hash_present_count = email_count AND recipient_hash_count = 1 THEN 15 ELSE 0 END
						+ CASE WHEN attachment_hash_present_count = email_count AND attachment_hash_count = 1 THEN 10 ELSE 0 END
						+ CASE WHEN max_sent_at - min_sent_at <= ${48 * 3600 * 1000} THEN 10 ELSE 0 END
					) AS score
				FROM candidate_base
				WHERE (
						45
						+ CASE WHEN body_hash_present_count = email_count AND body_hash_count = 1 THEN 20 ELSE 0 END
						+ CASE WHEN recipient_hash_present_count = email_count AND recipient_hash_count = 1 THEN 15 ELSE 0 END
						+ CASE WHEN attachment_hash_present_count = email_count AND attachment_hash_count = 1 THEN 10 ELSE 0 END
						+ CASE WHEN max_sent_at - min_sent_at <= ${48 * 3600 * 1000} THEN 10 ELSE 0 END
					) >= 55
				ORDER BY score DESC, email_count DESC, group_key ASC
				LIMIT ${normalizedBatchSize}
			`)
		);

		// Phase 2 (write): upsert groups + link emails. SQLite has no
		// data-modifying CTEs, so this is a synchronous transaction instead —
		// with better-sqlite3 the whole batch is a single atomic, race-free unit.
		let upsertedCount = 0;
		let linkedCount = 0;
		sqlite.transaction(() => {
			for (const candidate of candidates) {
				const existing = db
					.select({ id: fuzzyDuplicateGroups.id, status: fuzzyDuplicateGroups.status })
					.from(fuzzyDuplicateGroups)
					.where(eq(fuzzyDuplicateGroups.groupKey, candidate.group_key))
					.all();
				// Mirrors the old ON CONFLICT ... WHERE status = 'pending' guard:
				// approved/ignored groups are excluded upstream, but never overwrite
				// a non-pending group that appeared mid-scan.
				if (existing.length > 0 && existing[0].status !== 'pending') {
					continue;
				}
				const signals = {
					senderEmail: candidate.sender_email,
					subjectHash: candidate.duplicate_subject_hash,
					matchingBodyHash:
						candidate.body_hash_present_count === candidate.email_count &&
						candidate.body_hash_count === 1,
					matchingRecipients:
						candidate.recipient_hash_present_count === candidate.email_count &&
						candidate.recipient_hash_count === 1,
					matchingAttachments:
						candidate.attachment_hash_present_count === candidate.email_count &&
						candidate.attachment_hash_count === 1,
					sentSpreadHours:
						(candidate.max_sent_at - candidate.min_sent_at) / 3_600_000,
				};
				let groupId: string;
				if (existing.length > 0) {
					groupId = existing[0].id;
					db.update(fuzzyDuplicateGroups)
						.set({ score: candidate.score, signals, updatedAt: new Date() })
						.where(eq(fuzzyDuplicateGroups.id, groupId))
						.run();
				} else {
					const [inserted] = db
						.insert(fuzzyDuplicateGroups)
						.values({
							groupKey: candidate.group_key,
							status: 'pending',
							score: candidate.score,
							signals,
						})
						.returning({ id: fuzzyDuplicateGroups.id })
						.all();
					groupId = inserted.id;
				}
				upsertedCount++;

				const linked = db.run(sql`
					INSERT OR IGNORE INTO fuzzy_duplicate_group_emails (group_id, email_id, suggested_keeper)
					SELECT
						${groupId},
						ae.id,
						ae.id = (
							SELECT keeper.id
							FROM archived_emails keeper
							WHERE keeper.duplicate_fuzzy_group_key = ${candidate.group_key}
							ORDER BY keeper.sent_at ASC, keeper.archived_at ASC, keeper.id ASC
							LIMIT 1
						)
					FROM archived_emails ae
					WHERE ae.duplicate_fuzzy_group_key = ${candidate.group_key}
				`);
				linkedCount += Number(linked.changes ?? 0);
			}
		})();

		return {
			scannedGroups: candidates.length,
			insertedGroups: upsertedCount,
			linkedEmails: linkedCount,
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
			db.all(sql`
				SELECT count(*) AS total_groups
				FROM fuzzy_duplicate_groups
				WHERE status = 'pending'
			`)
		);

		const groupRows = toRows<RawFuzzyGroupRow>(
			db.all(sql`
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
		const actor = await new UserService().findById(userId);
		if (!actor) {
			throw new Error('Acting user not found');
		}
		let approvedGroups = 0;
		let deletedEmails = 0;
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
				.select({ id: archivedEmails.id })
				.from(archivedEmails)
				.where(inArray(archivedEmails.id, [keeperEmailId]));

			// Permanently delete the duplicate copies; keep the keeper.
			deletedEmails += await this.deleteDuplicateEmails(duplicateEmailIds, actor, actorIp);

			await db
				.update(fuzzyDuplicateGroups)
				.set({ status: 'approved', updatedAt: new Date() })
				.where(inArray(fuzzyDuplicateGroups.id, [group.groupId]));

			if (keeper) {
				keeperEmails += 1;
			}

			approvedGroups += 1;
		}

		return { approvedGroups, deletedEmails, keeperEmails };
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

	private static async findEmailsForFuzzyGroup(groupId: string): Promise<FuzzyDuplicateEmail[]> {
		const rows = toRows<RawFuzzyEmailRow>(
			db.all(sql`
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
					ae.message_id_header,
					ae.storage_hash_sha256,
					fge.suggested_keeper
				FROM fuzzy_duplicate_group_emails fge
				JOIN archived_emails ae ON ae.id = fge.email_id
				WHERE fge.group_id = ${groupId}
				ORDER BY fge.suggested_keeper DESC, ae.sent_at ASC, ae.archived_at ASC, ae.id ASC
			`)
		);

		return rows.map(mapFuzzyEmail);
	}
}
