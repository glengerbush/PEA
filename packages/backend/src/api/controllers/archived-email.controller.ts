import { Request, Response } from 'express';
import { ArchivedEmailService } from '../../services/ArchivedEmailService';
import { UserService } from '../../services/UserService';
import { SearchService } from '../../services/SearchService';
import { ArchiveTagService } from '../../services/ArchiveTagService';
import { DuplicateReviewService } from '../../services/DuplicateReviewService';
import { RemoteContentService } from '../../services/RemoteContentService';
import type {
	ApproveExactDuplicatesDto,
	ApproveFuzzyDuplicatesDto,
	ArchiveQuery,
	BulkDeleteArchivedEmailsDto,
	ArchiveSearchField,
	ArchiveSortField,
	IgnoreFuzzyDuplicateGroupsDto,
	MatchingStrategy,
	ScanFuzzyDuplicatesDto,
	SortDirection,
	UpdateArchivedEmailTagsDto,
} from '@open-archiver/types';

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
	'userEmail',
	'sourcePath',
	'sourceLabels',
	'tags',
]);

const SORT_FIELDS = new Set<ArchiveSortField>([
	'sentAt',
	'archivedAt',
	'sender',
	'subject',
	'sizeBytes',
]);

function firstQueryValue(value: unknown): string | undefined {
	if (Array.isArray(value)) {
		return firstQueryValue(value[0]);
	}
	if (typeof value === 'string') {
		return value;
	}
	return undefined;
}

function splitQueryList(value: unknown): string[] {
	const first = firstQueryValue(value);
	if (!first) return [];
	return first
		.split(',')
		.map((item) => item.trim())
		.filter(Boolean);
}

function parsePositiveInteger(value: unknown): number | undefined {
	const first = firstQueryValue(value);
	if (!first) return undefined;

	const parsed = Number(first);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : undefined;
}

function parseBoolean(value: unknown): boolean | undefined {
	const first = firstQueryValue(value);
	if (first === 'true') return true;
	if (first === 'false') return false;
	return undefined;
}

function parseFields(value: unknown): ArchiveSearchField[] | undefined {
	const fields = splitQueryList(value).filter((field): field is ArchiveSearchField =>
		SEARCH_FIELDS.has(field as ArchiveSearchField)
	);
	return fields.length > 0 ? fields : undefined;
}

function parseSort(value: unknown): ArchiveSortField | undefined {
	const first = firstQueryValue(value);
	return first && SORT_FIELDS.has(first as ArchiveSortField)
		? (first as ArchiveSortField)
		: undefined;
}

function parseDirection(value: unknown): SortDirection | undefined {
	const first = firstQueryValue(value);
	return first === 'asc' || first === 'desc' ? first : undefined;
}

function parseMatchingStrategy(value: unknown): MatchingStrategy | undefined {
	const first = firstQueryValue(value);
	return first === 'last' || first === 'all' || first === 'frequency' ? first : undefined;
}

export class ArchivedEmailController {
	private userService = new UserService();
	private searchService = new SearchService();

	public queryArchivedEmails = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;

			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const filters: NonNullable<ArchiveQuery['filters']> = {};
			const ingestionSourceId = firstQueryValue(req.query.ingestionSourceId);
			const userEmail = firstQueryValue(req.query.userEmail);
			const from = firstQueryValue(req.query.from);
			const to = firstQueryValue(req.query.to);
			const cc = firstQueryValue(req.query.cc);
			const bcc = firstQueryValue(req.query.bcc);
			const sourcePath = firstQueryValue(req.query.sourcePath);
			const sourceLabels = splitQueryList(req.query.sourceLabels);
			const tags = splitQueryList(req.query.tags);
			const hasAttachments = parseBoolean(req.query.hasAttachments);

			if (ingestionSourceId) filters.ingestionSourceId = ingestionSourceId;
			if (userEmail) filters.userEmail = userEmail;
			if (from) filters.from = from;
			if (to) filters.to = to;
			if (cc) filters.cc = cc;
			if (bcc) filters.bcc = bcc;
			if (sourcePath) filters.sourcePath = sourcePath;
			if (sourceLabels.length > 0) filters.sourceLabels = sourceLabels;
			if (tags.length > 0) filters.tags = tags;
			if (hasAttachments !== undefined) filters.hasAttachments = hasAttachments;

			for (const key of [
				'sentAfter',
				'sentBefore',
				'archivedAfter',
				'archivedBefore',
			] as const) {
				const value = firstQueryValue(req.query[key]);
				if (value) {
					filters[key] = value;
				}
			}

			const query: ArchiveQuery = {
				query: firstQueryValue(req.query.q) || '',
				filters,
				fields: parseFields(req.query.fields),
				sort: parseSort(req.query.sort),
				direction: parseDirection(req.query.direction),
				page: parsePositiveInteger(req.query.page),
				limit: parsePositiveInteger(req.query.limit),
				matchingStrategy: parseMatchingStrategy(req.query.matchingStrategy),
			};

			const result = await this.searchService.queryArchivedEmails(
				query,
				userId,
				req.ip || 'unknown'
			);
			return res.status(200).json(result);
		} catch (error) {
			console.error('Query archived emails error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(500).json({ message });
		}
	};

	public listFilterFacets = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const facets = await this.searchService.getFilterFacets();
			return res.status(200).json(facets);
		} catch (error) {
			console.error('List archive filter facets error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(500).json({ message });
		}
	};

	public updateArchivedEmailTags = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const dto = req.body as Partial<UpdateArchivedEmailTagsDto>;
			if (!Array.isArray(dto.emailIds)) {
				return res.status(400).json({ message: 'emailIds are required' });
			}

			const result = await ArchiveTagService.updateEmailTags(
				{
					emailIds: dto.emailIds,
					addTags: Array.isArray(dto.addTags) ? dto.addTags : [],
					removeTags: Array.isArray(dto.removeTags) ? dto.removeTags : [],
				},
				userId,
				req.ip || 'unknown'
			);
			return res.status(200).json(result);
		} catch (error) {
			console.error('Update archived email tags error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(400).json({ message });
		}
	};

	public listExactDuplicateGroups = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const allowedReasons = new Set([
				'message_id',
				'storage_hash',
				'attachment_hash_set',
				'sender_recipients_sent',
			]);
			const reasonParam = firstQueryValue(req.query.reason);
			const reason = reasonParam && allowedReasons.has(reasonParam) ? reasonParam : undefined;

			const result = await DuplicateReviewService.listExactDuplicateGroups(
				parsePositiveInteger(req.query.page),
				parsePositiveInteger(req.query.limit),
				reason
			);
			return res.status(200).json(result);
		} catch (error) {
			console.error('List exact duplicate groups error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(500).json({ message });
		}
	};

	public approveExactDuplicateGroups = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const dto = req.body as Partial<ApproveExactDuplicatesDto>;
			if (!Array.isArray(dto.groups)) {
				return res.status(400).json({ message: 'groups are required' });
			}

			const result = await DuplicateReviewService.approveExactDuplicateGroups(
				dto.groups,
				userId,
				req.ip || 'unknown'
			);
			return res.status(200).json(result);
		} catch (error) {
			console.error('Approve exact duplicate groups error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(400).json({ message });
		}
	};

	public listFuzzyDuplicateGroups = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const result = await DuplicateReviewService.listFuzzyDuplicateGroups(
				parsePositiveInteger(req.query.page),
				parsePositiveInteger(req.query.limit)
			);
			return res.status(200).json(result);
		} catch (error) {
			console.error('List fuzzy duplicate groups error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(500).json({ message });
		}
	};

	public scanFuzzyDuplicateGroups = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const dto = req.body as Partial<ScanFuzzyDuplicatesDto>;
			const result = await DuplicateReviewService.enqueueFuzzyDuplicateScan(dto.batchSize);
			return res.status(202).json(result);
		} catch (error) {
			console.error('Scan fuzzy duplicate groups error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(400).json({ message });
		}
	};

	public approveFuzzyDuplicateGroups = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const dto = req.body as Partial<ApproveFuzzyDuplicatesDto>;
			if (!Array.isArray(dto.groups)) {
				return res.status(400).json({ message: 'groups are required' });
			}

			const result = await DuplicateReviewService.approveFuzzyDuplicateGroups(
				dto.groups,
				userId,
				req.ip || 'unknown'
			);
			return res.status(200).json(result);
		} catch (error) {
			console.error('Approve fuzzy duplicate groups error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(400).json({ message });
		}
	};

	public ignoreFuzzyDuplicateGroups = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const dto = req.body as Partial<IgnoreFuzzyDuplicateGroupsDto>;
			if (!Array.isArray(dto.groupIds)) {
				return res.status(400).json({ message: 'groupIds are required' });
			}

			const result = await DuplicateReviewService.ignoreFuzzyDuplicateGroups(dto.groupIds);
			return res.status(200).json(result);
		} catch (error) {
			console.error('Ignore fuzzy duplicate groups error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(400).json({ message });
		}
	};

	public getRemoteContentPreview = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const preview = await RemoteContentService.buildPreview(req.params.id, userId);
			return res.status(200).json(preview);
		} catch (error) {
			console.error('Get remote content preview error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			const status = message === 'Archived email not found' ? 404 : 500;
			return res.status(status).json({ message });
		}
	};

	public enqueueRemoteContentArchive = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const result = await RemoteContentService.enqueueRemoteContentArchive([req.params.id]);
			return res.status(202).json(result);
		} catch (error) {
			console.error('Queue remote content archive error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			return res.status(400).json({ message });
		}
	};

	public listRemoteContentAssets = async (req: Request, res: Response): Promise<Response> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const assets = await RemoteContentService.listRemoteAssets(req.params.id);
			return res.status(200).json(assets);
		} catch (error) {
			console.error('List remote content assets error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			const status = message === 'Archived email not found' ? 404 : 500;
			return res.status(status).json({ message });
		}
	};

	public getRemoteContentAsset = async (req: Request, res: Response): Promise<void> => {
		try {
			const userId = req.user?.sub;
			if (!userId) {
				res.status(401).json({ message: req.t('errors.unauthorized') });
				return;
			}

			const asset = await RemoteContentService.getRemoteAssetStream(
				req.params.id,
				req.params.assetId,
				userId
			);
			res.setHeader('Content-Type', asset.contentType);
			res.setHeader('Content-Security-Policy', "default-src 'none'; img-src 'self' data:");
			res.setHeader('X-Content-Type-Options', 'nosniff');
			res.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
			res.setHeader('Referrer-Policy', 'no-referrer');
			res.setHeader('Cache-Control', 'private, max-age=86400');
			if (asset.sizeBytes !== null) {
				res.setHeader('Content-Length', String(asset.sizeBytes));
			}
			asset.stream.pipe(res);
		} catch (error) {
			console.error('Get remote content asset error:', error);
			const message =
				error instanceof Error ? error.message : req.t('errors.internalServerError');
			res.status(404).json({ message });
		}
	};

	public getArchivedEmails = async (req: Request, res: Response): Promise<Response> => {
		try {
			const { ingestionSourceId } = req.params;
			const page = parseInt(req.query.page as string, 10) || 1;
			const limit = parseInt(req.query.limit as string, 10) || 10;
			const userId = req.user?.sub;

			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const result = await ArchivedEmailService.getArchivedEmails(
				ingestionSourceId,
				page,
				limit,
				userId
			);
			return res.status(200).json(result);
		} catch (error) {
			console.error('Get archived emails error:', error);
			return res.status(500).json({ message: req.t('errors.internalServerError') });
		}
	};

	public getArchivedEmailById = async (req: Request, res: Response): Promise<Response> => {
		try {
			const { id } = req.params;
			const userId = req.user?.sub;

			if (!userId) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}
			const actor = await this.userService.findById(userId);
			if (!actor) {
				return res.status(401).json({ message: req.t('errors.unauthorized') });
			}

			const email = await ArchivedEmailService.getArchivedEmailById(
				id,
				userId,
				actor,
				req.ip || 'unknown'
			);
			if (!email) {
				return res.status(404).json({ message: req.t('archivedEmail.notFound') });
			}
			return res.status(200).json(email);
		} catch (error) {
			console.error(`Get archived email by id ${req.params.id} error:`, error);
			return res.status(500).json({ message: req.t('errors.internalServerError') });
		}
	};

	public bulkDeleteArchivedEmails = async (req: Request, res: Response): Promise<Response> => {
		const userId = req.user?.sub;
		if (!userId) {
			return res.status(401).json({ message: req.t('errors.unauthorized') });
		}
		const actor = await this.userService.findById(userId);
		if (!actor) {
			return res.status(401).json({ message: req.t('errors.unauthorized') });
		}

		const dto = req.body as Partial<BulkDeleteArchivedEmailsDto>;
		if (!Array.isArray(dto.emailIds) || dto.emailIds.length === 0) {
			return res.status(400).json({ message: 'emailIds are required' });
		}

		const deletedIds: string[] = [];
		const failed: { id: string; reason: string }[] = [];
		// Delete one at a time so each email keeps its per-email guards (legal
		// hold / retention policy) and a single blocked email doesn't fail the batch.
		for (const id of dto.emailIds) {
			try {
				await ArchivedEmailService.deleteArchivedEmail(id, actor, req.ip || 'unknown');
				deletedIds.push(id);
			} catch (error) {
				failed.push({
					id,
					reason: error instanceof Error ? error.message : 'Unknown error',
				});
			}
		}

		return res.status(200).json({
			requestedCount: dto.emailIds.length,
			deletedCount: deletedIds.length,
			deletedIds,
			failed,
		});
	};

	public deleteArchivedEmail = async (req: Request, res: Response): Promise<Response> => {
		const { id } = req.params;
		const userId = req.user?.sub;
		if (!userId) {
			return res.status(401).json({ message: req.t('errors.unauthorized') });
		}
		const actor = await this.userService.findById(userId);
		if (!actor) {
			return res.status(401).json({ message: req.t('errors.unauthorized') });
		}

		try {
			await ArchivedEmailService.deleteArchivedEmail(id, actor, req.ip || 'unknown');
			return res.status(204).send();
		} catch (error) {
			console.error(`Delete archived email ${req.params.id} error:`, error);
			if (error instanceof Error) {
				if (error.message === 'Archived email not found') {
					return res.status(404).json({ message: req.t('archivedEmail.notFound') });
				}
				return res.status(500).json({ message: error.message });
			}
			return res.status(500).json({ message: req.t('errors.internalServerError') });
		}
	};
}
