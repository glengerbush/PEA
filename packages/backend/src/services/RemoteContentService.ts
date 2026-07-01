import * as http from 'http';
import * as https from 'https';
import type { IncomingHttpHeaders } from 'http';
import { createHash } from 'crypto';
import { and, eq, inArray } from 'drizzle-orm';
import { simpleParser, type ParsedMail } from 'mailparser';
import { db } from '../database';
import { archivedEmails, remoteContentAssets } from '../database/schema';
import { config } from '../config';
import { logger } from '../config/logger';
import { StorageService } from './StorageService';
import { remoteContentQueue } from '../jobs/queues';
import { streamToBuffer } from '../helpers/streamToBuffer';
import type {
	ArchiveRemoteContentResult,
	RemoteContentPreview,
	RemoteContentStatus,
} from '@open-archiver/types';
import {
	assertSafeRemoteUrl,
	BlockedRemoteContentError,
	createPinnedLookup,
	getHeaderValue,
	isLikelyTrackingPixel,
	isLikelyTrackingUrl,
	isSafePreviewContentType,
	normalizeContentType,
	type SafeResolvedAddress,
	validateArchivableContent,
} from './RemoteContentSecurity';
import {
	decodeHtmlAttribute,
	extractCssUrls,
	extractSrcSetUrls,
	sanitizeEmailPreviewHtml,
	toRemoteUrl,
	type RemoteContentPreviewAsset,
} from './RemoteContentPreviewSanitizer';

type ArchivedEmailRecord = typeof archivedEmails.$inferSelect;
type RemoteContentAssetRecord = typeof remoteContentAssets.$inferSelect;
type RemoteHttpResponse = {
	statusCode: number;
	headers: IncomingHttpHeaders;
	body: Buffer;
};

const MAX_REMOTE_URLS_PER_EMAIL = 50;
const MAX_REMOTE_CONTENT_BYTES = 5 * 1024 * 1024;
const MAX_INLINE_CID_BYTES = 1024 * 1024;
const FETCH_TIMEOUT_MS = 10_000;
const MAX_REDIRECTS = 3;
const USER_AGENT = 'OpenArchiver-LocalRemoteContentArchiver/1.0';

const PREVIEW_CONTENT_SECURITY_POLICY = [
	"default-src 'none'",
	[
		"img-src 'self' data:",
		'http://localhost:*',
		'http://127.0.0.1:*',
		'http://[::1]:*',
		'https://localhost:*',
		'https://127.0.0.1:*',
		'https://[::1]:*',
	].join(' '),
	"style-src 'unsafe-inline'",
	"base-uri 'none'",
	"form-action 'none'",
].join('; ');

function hashValue(value: string | Buffer): string {
	return createHash('sha256').update(value).digest('hex');
}

function escapeHtml(value: string): string {
	return value
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')
		.replace(/'/g, '&#39;');
}

function getAttributeMap(rawAttributes: string): Map<string, string> {
	const attributes = new Map<string, string>();
	const attrPattern = /([^\s=/"'<>`]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
	let match: RegExpExecArray | null;

	while ((match = attrPattern.exec(rawAttributes)) !== null) {
		const name = match[1].toLowerCase();
		const value = decodeHtmlAttribute(match[2] ?? match[3] ?? match[4] ?? '');
		attributes.set(name, value);
	}

	return attributes;
}

function extensionForContentType(contentType: string | null): string {
	switch (contentType) {
		case 'image/png':
			return '.png';
		case 'image/jpeg':
			return '.jpg';
		case 'image/gif':
			return '.gif';
		case 'image/webp':
			return '.webp';
		case 'image/avif':
			return '.avif';
		default:
			return '.bin';
	}
}

export class RemoteContentService {
	private static storageService = new StorageService();

	public static async enqueueRemoteContentArchive(
		emailIds: string[]
	): Promise<ArchiveRemoteContentResult> {
		const uniqueEmailIds = Array.from(new Set(emailIds.filter(Boolean)));
		if (uniqueEmailIds.length > 0) {
			await db
				.update(archivedEmails)
				.set({ remoteContentStatus: 'pending' })
				.where(inArray(archivedEmails.id, uniqueEmailIds));
		}
		const job = await remoteContentQueue.add('archive-remote-content-batch', {
			emailIds: uniqueEmailIds,
		});
		return {
			jobId: job.id || '',
			emailIds: uniqueEmailIds,
		};
	}

	public static async archiveEmailRemoteContentBatch(emailIds: string[]): Promise<{
		processedEmails: number;
		archivedAssets: number;
		failedAssets: number;
		blockedAssets: number;
	}> {
		let processedEmails = 0;
		let archivedAssets = 0;
		let failedAssets = 0;
		let blockedAssets = 0;

		const uniqueEmailIds = Array.from(new Set(emailIds.filter(Boolean)));
		for (const emailId of uniqueEmailIds) {
			processedEmails += 1;
			try {
				const result = await this.archiveEmailRemoteContent(emailId);
				archivedAssets += result.archivedAssets;
				failedAssets += result.failedAssets;
				blockedAssets += result.blockedAssets;
			} catch (error) {
				failedAssets += 1;
				await db
					.update(archivedEmails)
					.set({
						remoteContentStatus: 'failed',
						remoteContentArchivedAt: new Date(),
					})
					.where(eq(archivedEmails.id, emailId));
				logger.warn(
					{
						emailId,
						error: error instanceof Error ? error.message : String(error),
					},
					'Remote content archive failed for email'
				);
			}
		}

		return { processedEmails, archivedAssets, failedAssets, blockedAssets };
	}

	public static async archiveEmailRemoteContent(emailId: string): Promise<{
		remoteUrlCount: number;
		archivedAssets: number;
		failedAssets: number;
		blockedAssets: number;
	}> {
		const email = await db.query.archivedEmails.findFirst({
			where: eq(archivedEmails.id, emailId),
		});
		if (!email) {
			return { remoteUrlCount: 0, archivedAssets: 0, failedAssets: 0, blockedAssets: 0 };
		}

		await db
			.update(archivedEmails)
			.set({ remoteContentStatus: 'pending' })
			.where(eq(archivedEmails.id, emailId));

		const parsedEmail = await this.parseStoredEmail(email);
		const html = typeof parsedEmail.html === 'string' ? parsedEmail.html : '';
		const remoteUrls = this.extractRemoteUrls(html).slice(0, MAX_REMOTE_URLS_PER_EMAIL);

		if (remoteUrls.length === 0) {
			await db
				.update(archivedEmails)
				.set({
					remoteContentStatus: 'skipped',
					remoteContentAssetCount: 0,
					remoteContentArchivedAt: new Date(),
				})
				.where(eq(archivedEmails.id, emailId));
			return { remoteUrlCount: 0, archivedAssets: 0, failedAssets: 0, blockedAssets: 0 };
		}

		for (const url of remoteUrls) {
			await this.archiveRemoteAsset(email, url);
		}

		// Second pass: archive images referenced from within archived stylesheets,
		// so the inlined CSS can render its background images with no view-time
		// network access.
		await this.archiveStylesheetSubresources(email);

		const assets = await db.query.remoteContentAssets.findMany({
			where: eq(remoteContentAssets.emailId, emailId),
		});
		const archivedAssets = assets.filter((asset) => asset.status === 'archived').length;
		const failedAssets = assets.filter((asset) => asset.status === 'failed').length;
		const blockedAssets = assets.filter((asset) => asset.status === 'blocked').length;
		const remoteContentStatus = this.summarizeEmailStatus(
			archivedAssets,
			failedAssets,
			blockedAssets
		);

		await db
			.update(archivedEmails)
			.set({
				remoteContentStatus,
				remoteContentAssetCount: archivedAssets,
				remoteContentArchivedAt: new Date(),
			})
			.where(eq(archivedEmails.id, emailId));

		return {
			remoteUrlCount: remoteUrls.length,
			archivedAssets,
			failedAssets,
			blockedAssets,
		};
	}

	public static async buildPreview(
		emailId: string,
		_userId: string
	): Promise<RemoteContentPreview> {
		const email = await this.getEmailForPreview(emailId);
		const [parsedEmail, assets] = await Promise.all([
			this.parseStoredEmail(email),
			db.query.remoteContentAssets.findMany({
				where: eq(remoteContentAssets.emailId, emailId),
			}),
		]);

		const html =
			typeof parsedEmail.html === 'string' && parsedEmail.html.trim()
				? parsedEmail.html
				: this.renderTextPreview(parsedEmail.text || '');
		const safeHtml = await this.sanitizePreviewHtml(emailId, html, parsedEmail, assets);
		const remoteUrls = this.extractRemoteUrls(
			typeof parsedEmail.html === 'string' ? parsedEmail.html : ''
		);

		return {
			emailId,
			html: `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${PREVIEW_CONTENT_SECURITY_POLICY}"><base target="_blank"></head><body>${safeHtml}</body></html>`,
			status: email.remoteContentStatus as RemoteContentStatus,
			remoteUrlCount: remoteUrls.length,
			archivedAssetCount: assets.filter((asset) => asset.status === 'archived').length,
			blockedAssetCount: assets.filter((asset) => asset.status === 'blocked').length,
			failedAssetCount: assets.filter((asset) => asset.status === 'failed').length,
		};
	}

	/** Lists this email's remote-content assets (archived, failed, and blocked —
	 *  not pending) for the detail-page list: source URL, type, size, status, and
	 *  the failure reason when it didn't archive. */
	public static async listRemoteAssets(emailId: string): Promise<
		{
			id: string;
			originalUrl: string;
			contentType: string | null;
			sizeBytes: number | null;
			status: RemoteContentAssetRecord['status'];
			failureReason: string | null;
			previewable: boolean;
		}[]
	> {
		await this.getEmailForPreview(emailId);
		const assets = await db.query.remoteContentAssets.findMany({
			where: eq(remoteContentAssets.emailId, emailId),
		});
		const rank: Record<string, number> = { archived: 0, failed: 1, blocked: 2, pending: 3 };
		return assets
			.filter((asset) => asset.status !== 'pending')
			// Archived stylesheets are inlined into the preview, not standalone
			// downloadable assets (and the asset endpoint won't serve CSS raw).
			.filter(
				(asset) =>
					!(
						asset.status === 'archived' &&
						normalizeContentType(asset.contentType) === 'text/css'
					)
			)
			.sort((a, b) => (rank[a.status] ?? 9) - (rank[b.status] ?? 9))
			.map((asset) => {
				const contentType = normalizeContentType(asset.contentType);
				return {
					id: asset.id,
					originalUrl: asset.originalUrl,
					contentType,
					sizeBytes: asset.sizeBytes,
					status: asset.status,
					failureReason: asset.failureReason,
					previewable:
						asset.status === 'archived' &&
						!!asset.storagePath &&
						isSafePreviewContentType(contentType),
				};
			});
	}

	public static async getRemoteAssetStream(
		emailId: string,
		assetId: string,
		_userId: string
	): Promise<{
		stream: NodeJS.ReadableStream;
		contentType: string;
		sizeBytes: number | null;
	}> {
		await this.getEmailForPreview(emailId);
		const asset = await db.query.remoteContentAssets.findFirst({
			where: and(
				eq(remoteContentAssets.id, assetId),
				eq(remoteContentAssets.emailId, emailId)
			),
		});

		if (!asset || asset.status !== 'archived' || !asset.storagePath) {
			throw new Error('Remote content asset not found');
		}

		if (!isSafePreviewContentType(normalizeContentType(asset.contentType))) {
			throw new Error('Remote content asset is not previewable');
		}

		return {
			stream: await this.storageService.get(asset.storagePath),
			contentType: normalizeContentType(asset.contentType) || 'application/octet-stream',
			sizeBytes: asset.sizeBytes,
		};
	}

	private static async getEmailForPreview(emailId: string): Promise<ArchivedEmailRecord> {
		const email = await db.query.archivedEmails.findFirst({
			where: eq(archivedEmails.id, emailId),
		});
		if (!email) {
			throw new Error('Archived email not found');
		}

		return email;
	}

	private static async parseStoredEmail(email: ArchivedEmailRecord): Promise<ParsedMail> {
		const stream = await this.storageService.get(email.storagePath);
		const raw = await streamToBuffer(stream);
		return simpleParser(raw);
	}

	private static extractRemoteUrls(html: string): string[] {
		const urls = new Set<string>();
		// Never archive (and therefore never fetch) open-tracking beacons, which would
		// notify the sender that the email was opened.
		const addUrl = (url: string | null): void => {
			if (url && !isLikelyTrackingUrl(url)) {
				urls.add(url);
			}
		};
		const tagPattern = /<([a-zA-Z][\w:-]*)([^>]*)>/g;
		let match: RegExpExecArray | null;

		while ((match = tagPattern.exec(html)) !== null) {
			const tag = match[1].toLowerCase();
			const attrs = getAttributeMap(match[2]);
			// A tiny/hidden image is a tracking pixel — skip all of its URLs entirely.
			const isTrackingPixel = isLikelyTrackingPixel(attrs);

			if (!isTrackingPixel) {
				for (const attrName of ['src', 'background', 'poster']) {
					const value = attrs.get(attrName);
					addUrl(value ? toRemoteUrl(value) : null);
				}

				const srcSet = attrs.get('srcset');
				if (srcSet) {
					for (const url of extractSrcSetUrls(srcSet)) {
						addUrl(url);
					}
				}
			}

			const style = attrs.get('style');
			if (style) {
				for (const url of extractCssUrls(style)) {
					addUrl(url);
				}
			}

			if (tag === 'link') {
				const href = attrs.get('href');
				addUrl(href ? toRemoteUrl(href) : null);
			}
		}

		for (const url of extractCssUrls(html)) {
			addUrl(url);
		}

		return Array.from(urls);
	}

	/**
	 * Archives images referenced by `url(...)` inside this email's already-archived
	 * stylesheets. Relative URLs are resolved against the stylesheet's own URL.
	 * `@import` is intentionally NOT followed (no stylesheet recursion / SSRF
	 * amplification). Bounded by the same per-email asset cap.
	 */
	private static async archiveStylesheetSubresources(email: ArchivedEmailRecord): Promise<void> {
		const assets = await db.query.remoteContentAssets.findMany({
			where: eq(remoteContentAssets.emailId, email.id),
		});
		const stylesheets = assets.filter(
			(asset) =>
				asset.status === 'archived' &&
				asset.storagePath &&
				normalizeContentType(asset.contentType) === 'text/css'
		);
		if (stylesheets.length === 0) return;

		const known = new Set(assets.map((asset) => asset.urlHash));
		const subUrls = new Set<string>();
		const urlPattern = /url\(\s*(?:"([^"]+)"|'([^']+)'|([^)]+))\s*\)/gi;

		for (const sheet of stylesheets) {
			let cssText: string;
			try {
				cssText = (
					await streamToBuffer(await this.storageService.get(sheet.storagePath as string))
				).toString('utf8');
			} catch {
				continue;
			}
			const base = sheet.finalUrl || sheet.originalUrl;
			let match: RegExpExecArray | null;
			while ((match = urlPattern.exec(cssText)) !== null) {
				const raw = (match[1] ?? match[2] ?? match[3] ?? '').trim();
				if (!raw || raw.startsWith('data:') || raw.startsWith('#')) continue;
				let resolved: string | null = null;
				try {
					const url = new URL(raw, base);
					resolved =
						url.protocol === 'http:' || url.protocol === 'https:' ? url.href : null;
				} catch {
					resolved = null;
				}
				if (!resolved || isLikelyTrackingUrl(resolved)) continue;
				if (known.has(hashValue(resolved))) continue;
				subUrls.add(resolved);
			}
		}

		for (const url of Array.from(subUrls).slice(0, MAX_REMOTE_URLS_PER_EMAIL)) {
			await this.archiveRemoteAsset(email, url);
		}
	}

	private static async archiveRemoteAsset(
		email: ArchivedEmailRecord,
		originalUrl: string
	): Promise<void> {
		const urlHash = hashValue(originalUrl);
		const existing = await db.query.remoteContentAssets.findFirst({
			where: and(
				eq(remoteContentAssets.emailId, email.id),
				eq(remoteContentAssets.urlHash, urlHash)
			),
		});

		if (existing?.status === 'archived' && existing.storagePath) {
			return;
		}

		let asset = existing;
		if (!asset) {
			const inserted = await db
				.insert(remoteContentAssets)
				.values({
					emailId: email.id,
					originalUrl,
					urlHash,
					status: 'pending',
				})
				.onConflictDoNothing()
				.returning();
			asset = inserted[0];
			// Lost a race with a concurrent worker (or the manual per-email
			// archive route) that inserted the same (emailId, urlHash) between our
			// findFirst and insert. onConflictDoNothing returns no row in that
			// case — re-read the existing row instead of throwing a unique
			// violation that would mislabel the whole email as failed.
			if (!asset) {
				asset = await db.query.remoteContentAssets.findFirst({
					where: and(
						eq(remoteContentAssets.emailId, email.id),
						eq(remoteContentAssets.urlHash, urlHash)
					),
				});
			}
		}
		if (!asset) {
			throw new Error('Failed to create remote content asset record');
		}

		try {
			const fetched = await this.fetchRemoteContent(originalUrl);
			const contentHash = hashValue(fetched.body);
			const storagePath = `${config.storage.openArchiverFolderName}/remote-content/${email.id}/${asset.id}${extensionForContentType(fetched.contentType)}`;

			await this.storageService.put(storagePath, fetched.body);
			await db
				.update(remoteContentAssets)
				.set({
					finalUrl: fetched.finalUrl,
					status: 'archived',
					contentType: fetched.contentType,
					sizeBytes: fetched.body.length,
					contentHashSha256: contentHash,
					storagePath,
					failureReason: null,
					updatedAt: new Date(),
				})
				.where(eq(remoteContentAssets.id, asset.id));
		} catch (error) {
			const status = error instanceof BlockedRemoteContentError ? 'blocked' : 'failed';
			await db
				.update(remoteContentAssets)
				.set({
					status,
					failureReason: error instanceof Error ? error.message : String(error),
					updatedAt: new Date(),
				})
				.where(eq(remoteContentAssets.id, asset.id));

			logger.warn(
				{
					emailId: email.id,
					originalUrl,
					status,
					error: error instanceof Error ? error.message : String(error),
				},
				'Remote content asset was not archived'
			);
		}
	}

	private static async fetchRemoteContent(
		rawUrl: string,
		redirectCount = 0
	): Promise<{ body: Buffer; contentType: string | null; finalUrl: string }> {
		if (redirectCount > MAX_REDIRECTS) {
			throw new BlockedRemoteContentError('Too many redirects');
		}

		const url = new URL(rawUrl);
		const resolvedAddress = await assertSafeRemoteUrl(url);
		const response = await this.requestRemoteContent(url, resolvedAddress);

		if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
			const location = getHeaderValue(response.headers.location);
			if (!location) {
				throw new BlockedRemoteContentError('Redirect without location header');
			}
			const redirectedUrl = new URL(location, url);
			return this.fetchRemoteContent(redirectedUrl.href, redirectCount + 1);
		}

		if (response.statusCode < 200 || response.statusCode >= 300) {
			throw new Error(`Remote server returned ${response.statusCode}`);
		}

		const contentType = validateArchivableContent(
			response.body,
			getHeaderValue(response.headers['content-type'])
		);
		return {
			body: response.body,
			contentType,
			finalUrl: url.href,
		};
	}

	private static async requestRemoteContent(
		url: URL,
		resolvedAddress: SafeResolvedAddress
	): Promise<RemoteHttpResponse> {
		const requestOptions: http.RequestOptions & {
			servername?: string;
			autoSelectFamily?: boolean;
		} = {
			method: 'GET',
			headers: {
				'User-Agent': USER_AGENT,
				Accept: 'image/avif,image/webp,image/png,image/jpeg,image/gif,*/*;q=0.1',
			},
			lookup: createPinnedLookup(resolvedAddress),
			// We've already resolved one SSRF-safe address; connect straight to it
			// rather than letting Node's Happy-Eyeballs path re-run the lookup
			// (which produced cryptic "Invalid IP address: undefined" under load).
			family: resolvedAddress.family,
			autoSelectFamily: false,
		};

		if (url.protocol === 'https:') {
			requestOptions.servername = url.hostname;
		}

		return new Promise((resolve, reject) => {
			let settled = false;
			const fail = (error: Error): void => {
				if (settled) return;
				settled = true;
				reject(error);
			};

			const handleResponse = (response: http.IncomingMessage): void => {
				this.readBoundedResponseBody(response)
					.then((body) => {
						if (settled) return;
						settled = true;
						resolve({
							statusCode: response.statusCode || 0,
							headers: response.headers,
							body,
						});
					})
					.catch(fail);
			};

			const request =
				url.protocol === 'https:'
					? https.request(url, requestOptions, handleResponse)
					: http.request(url, requestOptions, handleResponse);

			request.setTimeout(FETCH_TIMEOUT_MS, () => {
				request.destroy(new BlockedRemoteContentError('Remote content fetch timed out'));
			});
			request.on('error', fail);
			request.end();
		});
	}

	private static async readBoundedResponseBody(response: http.IncomingMessage): Promise<Buffer> {
		return new Promise((resolve, reject) => {
			const contentLength = Number(getHeaderValue(response.headers['content-length']) || '0');
			if (contentLength > MAX_REMOTE_CONTENT_BYTES) {
				response.resume();
				reject(new BlockedRemoteContentError('Remote content is too large'));
				return;
			}

			const chunks: Buffer[] = [];
			let totalBytes = 0;
			let settled = false;
			const fail = (error: Error): void => {
				if (settled) return;
				settled = true;
				reject(error);
			};

			response.on('data', (chunk: Buffer | string) => {
				const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
				totalBytes += buffer.length;
				if (totalBytes > MAX_REMOTE_CONTENT_BYTES) {
					const error = new BlockedRemoteContentError('Remote content is too large');
					fail(error);
					response.destroy(error);
					return;
				}
				chunks.push(buffer);
			});

			response.on('end', () => {
				if (settled) return;
				settled = true;
				resolve(Buffer.concat(chunks));
			});
			response.on('error', fail);
		});
	}

	private static summarizeEmailStatus(
		archivedAssets: number,
		failedAssets: number,
		blockedAssets: number
	): RemoteContentStatus {
		// Derive status from the actual asset outcomes — which include image
		// subresources discovered inside archived stylesheets — not from the count
		// of remote URLs in the HTML. Those two diverge once
		// archiveStylesheetSubresources() adds CSS-referenced assets, which would
		// otherwise mislabel a fully-archived email as 'partial' or mask a failed
		// subresource as 'archived'. Callers handle the no-remote-content ('skipped')
		// case before reaching here.
		const resolved = archivedAssets + failedAssets + blockedAssets;
		if (resolved === 0) return 'skipped';
		if (failedAssets === 0 && blockedAssets === 0) return 'archived';
		if (archivedAssets > 0) return 'partial';
		return 'failed';
	}

	private static renderTextPreview(text: string): string {
		return `<div>${escapeHtml(text || '').replace(/\r?\n/g, '<br>')}</div>`;
	}

	private static async sanitizePreviewHtml(
		emailId: string,
		html: string,
		parsedEmail: ParsedMail,
		assets: RemoteContentAssetRecord[]
	): Promise<string> {
		const cidMap = this.buildCidMap(parsedEmail);
		const cssByUrl = await this.loadArchivedStylesheets(assets);
		return sanitizeEmailPreviewHtml({
			emailId,
			html,
			cidMap,
			assets: assets as RemoteContentPreviewAsset[],
			cssByUrl,
		});
	}

	/** Reads the bytes of archived `text/css` assets so they can be inlined into
	 *  the preview. Keyed by both original and final URL for matching. */
	private static async loadArchivedStylesheets(
		assets: RemoteContentAssetRecord[]
	): Promise<Map<string, string>> {
		const cssByUrl = new Map<string, string>();
		const cssAssets = assets.filter(
			(asset) =>
				asset.status === 'archived' &&
				asset.storagePath &&
				normalizeContentType(asset.contentType) === 'text/css'
		);
		for (const asset of cssAssets) {
			try {
				const buffer = await streamToBuffer(
					await this.storageService.get(asset.storagePath as string)
				);
				const text = buffer.toString('utf8');
				cssByUrl.set(asset.originalUrl, text);
				if (asset.finalUrl) cssByUrl.set(asset.finalUrl, text);
			} catch (error) {
				logger.warn(
					{ assetId: asset.id, error: error instanceof Error ? error.message : String(error) },
					'Failed to read archived stylesheet for preview'
				);
			}
		}
		return cssByUrl;
	}

	private static buildCidMap(parsedEmail: ParsedMail): Map<string, string> {
		const cidMap = new Map<string, string>();
		for (const attachment of parsedEmail.attachments || []) {
			if (
				!attachment.cid ||
				!attachment.content ||
				attachment.content.length > MAX_INLINE_CID_BYTES
			) {
				continue;
			}

			const contentType = normalizeContentType(attachment.contentType);
			if (!isSafePreviewContentType(contentType)) {
				continue;
			}

			const cid = attachment.cid.replace(/^<|>$/g, '');
			cidMap.set(cid, `data:${contentType};base64,${attachment.content.toString('base64')}`);
		}
		return cidMap;
	}
}
