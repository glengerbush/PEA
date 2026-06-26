import sanitizeHtml = require('sanitize-html');
import { isSafePreviewContentType, normalizeContentType } from './RemoteContentSecurity';

type Attributes = sanitizeHtml.Attributes;

export type RemoteContentPreviewAsset = {
	id: string;
	originalUrl: string;
	finalUrl: string | null;
	status: string;
	storagePath: string | null;
	contentType: string | null;
};

const ALLOWED_TAGS = [
	'a',
	'abbr',
	'b',
	'big',
	'blockquote',
	'br',
	'caption',
	'center',
	'cite',
	'code',
	'col',
	'colgroup',
	'dd',
	'del',
	'div',
	'dl',
	'dt',
	'em',
	'font',
	'h1',
	'h2',
	'h3',
	'h4',
	'h5',
	'h6',
	'hr',
	'i',
	'img',
	'li',
	'ol',
	'p',
	'pre',
	's',
	'small',
	'span',
	'strong',
	'sub',
	'sup',
	'table',
	'tbody',
	'td',
	'tfoot',
	'th',
	'thead',
	'tr',
	'u',
	'ul',
];

const SAFE_GLOBAL_ATTRIBUTES = [
	'align',
	'alt',
	'bgcolor',
	'border',
	'cellpadding',
	'cellspacing',
	'class',
	'colspan',
	'dir',
	'height',
	'lang',
	'rowspan',
	'style',
	'title',
	'valign',
	'width',
];

const BLOCKED_ATTRIBUTES = new Set(['srcdoc', 'formaction', 'ping', 'manifest', 'xmlns']);

export function decodeHtmlAttribute(value: string): string {
	return value
		.replace(/&amp;/gi, '&')
		.replace(/&quot;/gi, '"')
		.replace(/&#39;|&apos;/gi, "'")
		.replace(/&lt;/gi, '<')
		.replace(/&gt;/gi, '>')
		.replace(/&#x([0-9a-f]+);/gi, (_, hex: string) => String.fromCodePoint(parseInt(hex, 16)))
		.replace(/&#(\d+);/g, (_, decimal: string) => String.fromCodePoint(parseInt(decimal, 10)));
}

export function toRemoteUrl(value: string): string | null {
	const trimmed = value.replace(/[\u0000-\u001f\u007f]+/g, '').trim();
	if (!trimmed) return null;

	try {
		const url = new URL(trimmed);
		return url.protocol === 'http:' || url.protocol === 'https:' ? url.href : null;
	} catch {
		return null;
	}
}

export function extractSrcSetUrls(value: string): string[] {
	return value
		.split(',')
		.map((item) => item.trim().split(/\s+/)[0])
		.map(toRemoteUrl)
		.filter((url): url is string => Boolean(url));
}

export function extractCssUrls(value: string): string[] {
	const urls: string[] = [];
	const urlPattern = /url\(\s*(?:"([^"]+)"|'([^']+)'|([^)]+))\s*\)/gi;
	let match: RegExpExecArray | null;

	while ((match = urlPattern.exec(value)) !== null) {
		const url = toRemoteUrl(decodeHtmlAttribute(match[1] ?? match[2] ?? match[3] ?? ''));
		if (url) urls.push(url);
	}

	return urls;
}

function sanitizeStyle(value: string): string {
	if (/expression\s*\(|behavior\s*:|-moz-binding|@import/i.test(value)) {
		return '';
	}

	return value
		.split(';')
		.map((declaration) => declaration.trim())
		.filter((declaration) => declaration && !/url\s*\(/i.test(declaration))
		.join('; ');
}

function isSafeLinkUrl(value: string): boolean {
	const trimmed = value.trim();
	if (!trimmed || trimmed.startsWith('#')) return true;

	try {
		const url = new URL(trimmed);
		return ['http:', 'https:', 'mailto:', 'tel:'].includes(url.protocol);
	} catch {
		return false;
	}
}

function sanitizeCommonAttributes(attribs: Attributes): Attributes {
	const sanitized: Attributes = {};
	for (const [rawName, value] of Object.entries(attribs)) {
		const name = rawName.toLowerCase();
		if (name.startsWith('on') || BLOCKED_ATTRIBUTES.has(name)) continue;

		if (name === 'style') {
			const safeStyle = sanitizeStyle(value);
			if (safeStyle) sanitized.style = safeStyle;
			continue;
		}

		sanitized[name] = value;
	}
	return sanitized;
}

function safeSrcSetDescriptor(value: string): boolean {
	return /^\d+w$/.test(value) || /^\d+(\.\d+)?x$/.test(value);
}

function rewriteSrcSet(
	emailId: string,
	value: string,
	cidMap: Map<string, string>,
	assetByUrl: Map<string, RemoteContentPreviewAsset>
): string | null {
	const entries = value
		.split(',')
		.map((entry) => {
			const [rawUrl, ...descriptors] = entry.trim().split(/\s+/);
			const rewritten = rewriteImageSource(emailId, rawUrl, cidMap, assetByUrl);
			if (!rewritten) return null;

			const safeDescriptors = descriptors.filter(safeSrcSetDescriptor);
			return [rewritten, ...safeDescriptors].join(' ');
		})
		.filter((entry): entry is string => Boolean(entry));

	return entries.length > 0 ? entries.join(', ') : null;
}

function rewriteImageSource(
	emailId: string,
	value: string,
	cidMap: Map<string, string>,
	assetByUrl: Map<string, RemoteContentPreviewAsset>
): string | null {
	const trimmed = value.trim();
	const cidMatch = /^cid:(.+)$/i.exec(trimmed);
	if (cidMatch) {
		return cidMap.get(cidMatch[1].replace(/^<|>$/g, '')) || null;
	}

	if (/^data:image\/(png|jpeg|gif|webp|avif);base64,/i.test(trimmed)) {
		return trimmed;
	}

	const remoteUrl = toRemoteUrl(trimmed);
	if (!remoteUrl) {
		return null;
	}

	const asset = assetByUrl.get(remoteUrl);
	if (
		!asset ||
		asset.status !== 'archived' ||
		!asset.storagePath ||
		!isSafePreviewContentType(normalizeContentType(asset.contentType))
	) {
		return null;
	}

	return `/api/v1/archived-emails/${emailId}/remote-assets/${asset.id}`;
}

export function sanitizeEmailPreviewHtml({
	emailId,
	html,
	cidMap,
	assets,
}: {
	emailId: string;
	html: string;
	cidMap: Map<string, string>;
	assets: RemoteContentPreviewAsset[];
}): string {
	const assetByUrl = new Map<string, RemoteContentPreviewAsset>();
	for (const asset of assets) {
		assetByUrl.set(asset.originalUrl, asset);
		if (asset.finalUrl) assetByUrl.set(asset.finalUrl, asset);
	}

	return sanitizeHtml(html, {
		allowedTags: ALLOWED_TAGS,
		allowedAttributes: {
			'*': SAFE_GLOBAL_ATTRIBUTES,
			a: ['href', 'target', 'rel'],
			img: ['src', 'srcset'],
		},
		allowedSchemes: ['http', 'https', 'mailto', 'tel'],
		allowedSchemesByTag: {
			img: ['http', 'https', 'data'],
		},
		allowProtocolRelative: false,
		disallowedTagsMode: 'discard',
		parseStyleAttributes: false,
		transformTags: {
			'*': (tagName, attribs) => ({
				tagName,
				attribs: sanitizeCommonAttributes(attribs),
			}),
			a: (tagName, attribs) => {
				const sanitized = sanitizeCommonAttributes(attribs);
				if (attribs.href && isSafeLinkUrl(attribs.href)) {
					sanitized.href = attribs.href;
					sanitized.target = '_blank';
					sanitized.rel = 'noopener noreferrer';
				} else {
					delete sanitized.href;
				}
				return { tagName, attribs: sanitized };
			},
			img: (tagName, attribs) => {
				const sanitized = sanitizeCommonAttributes(attribs);
				const rewrittenSrc = attribs.src
					? rewriteImageSource(emailId, attribs.src, cidMap, assetByUrl)
					: null;
				const rewrittenSrcSet = attribs.srcset
					? rewriteSrcSet(emailId, attribs.srcset, cidMap, assetByUrl)
					: null;

				if (rewrittenSrc) sanitized.src = rewrittenSrc;
				else delete sanitized.src;

				if (rewrittenSrcSet) sanitized.srcset = rewrittenSrcSet;
				else delete sanitized.srcset;

				return { tagName, attribs: sanitized };
			},
		},
		exclusiveFilter: (frame) =>
			frame.tag === 'img' && !frame.attribs.src && !frame.attribs.srcset,
	});
}
