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

type CssUrlRewriter = (rawUrl: string) => string | null;

/**
 * Rewrites every `url(...)` in a CSS string to its archived local asset, so the
 * rendered preview never reaches out to the network. URLs that weren't archived
 * (or aren't images) become an empty `data:` URL — the strict preview CSP would
 * block a remote fetch anyway, this just avoids a broken request.
 */
function rewriteCssUrls(css: string, rewriteUrl: CssUrlRewriter): string {
	return css.replace(/url\(\s*(['"]?)([^'")]*)\1\s*\)/gi, (_full, _quote, rawUrl: string) => {
		const rewritten = rewriteUrl(decodeHtmlAttribute(rawUrl.trim()));
		return `url('${rewritten ?? 'data:,'}')`;
	});
}

/**
 * Sanitizes the contents of a `<style>` block for inclusion in the no-script,
 * strict-CSP preview iframe: removes remote stylesheet imports, legacy
 * script-via-CSS vectors, and tag-breakout characters, and rewrites `url()` to
 * archived assets.
 */
function sanitizeCssText(css: string, rewriteUrl: CssUrlRewriter): string {
	const cleaned = css
		.replace(/\/\*[\s\S]*?\*\//g, '') // comments (could hide tricks)
		.replace(/@import\b[^;]*;?/gi, '') // remote stylesheet imports
		.replace(/expression\s*\([^)]*\)/gi, '') // legacy IE script-in-CSS
		.replace(/(?:behavior|-moz-binding)\s*:[^;}]*/gi, '') // script via CSS
		.replace(/javascript:/gi, '')
		.replace(/[<>]/g, ''); // prevent </style> breakout / tag injection
	return rewriteCssUrls(cleaned, rewriteUrl).trim();
}

/**
 * Sanitizes an inline `style="..."` value. Strips the dangerous constructs and
 * rewrites `url(...)` to archived assets (previously url() declarations were
 * dropped entirely, losing inline background images).
 */
function sanitizeStyle(value: string, rewriteUrl: CssUrlRewriter): string {
	const cleaned = value
		.replace(/@import\b[^;]*;?/gi, '')
		.replace(/expression\s*\([^)]*\)/gi, '')
		.replace(/(?:behavior|-moz-binding)\s*:[^;]*/gi, '')
		.replace(/javascript:/gi, '');

	return cleaned
		.split(';')
		.map((declaration) => declaration.trim())
		.filter(Boolean)
		.map((declaration) => rewriteCssUrls(declaration, rewriteUrl))
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

function sanitizeCommonAttributes(attribs: Attributes, rewriteUrl: CssUrlRewriter): Attributes {
	const sanitized: Attributes = {};
	for (const [rawName, value] of Object.entries(attribs)) {
		const name = rawName.toLowerCase();
		if (name.startsWith('on') || BLOCKED_ATTRIBUTES.has(name)) continue;

		if (name === 'style') {
			const safeStyle = sanitizeStyle(value, rewriteUrl);
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

/** Reads a named attribute out of a single start-tag string. */
function getTagAttribute(tag: string, name: string): string | null {
	const match = new RegExp(`\\b${name}\\s*=\\s*("([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i').exec(tag);
	return match ? (match[2] ?? match[3] ?? match[4] ?? null) : null;
}

export function sanitizeEmailPreviewHtml({
	emailId,
	html,
	cidMap,
	assets,
	cssByUrl = new Map(),
}: {
	emailId: string;
	html: string;
	cidMap: Map<string, string>;
	assets: RemoteContentPreviewAsset[];
	/** Archived stylesheet contents keyed by their (original/final) URL, for
	 *  inlining `<link rel="stylesheet">` references. */
	cssByUrl?: Map<string, string>;
}): string {
	const assetByUrl = new Map<string, RemoteContentPreviewAsset>();
	for (const asset of assets) {
		assetByUrl.set(asset.originalUrl, asset);
		if (asset.finalUrl) assetByUrl.set(asset.finalUrl, asset);
	}

	const rewriteUrl: CssUrlRewriter = (rawUrl) =>
		rewriteImageSource(emailId, rawUrl, cidMap, assetByUrl);

	const styleBlocks: string[] = [];

	// Inline archived external stylesheets (<link rel="stylesheet">) as sanitized
	// <style> blocks. The <link> tags themselves are dropped by sanitize-html
	// (and the CSP forbids loading external sheets), so this is the only way the
	// email's linked CSS can take effect — and only after full sanitization.
	if (cssByUrl.size > 0) {
		const linkPattern = /<link\b[^>]*>/gi;
		let linkMatch: RegExpExecArray | null;
		while ((linkMatch = linkPattern.exec(html)) !== null) {
			const tag = linkMatch[0];
			const rel = (getTagAttribute(tag, 'rel') || '').toLowerCase();
			if (!rel.split(/\s+/).includes('stylesheet')) continue;
			const hrefRaw = getTagAttribute(tag, 'href');
			const href = hrefRaw ? toRemoteUrl(decodeHtmlAttribute(hrefRaw)) : null;
			const css = href ? cssByUrl.get(href) : undefined;
			if (css && href) {
				// Resolve the stylesheet's own (possibly relative) url() references
				// against its URL before mapping them to archived assets.
				const sheetRewrite: CssUrlRewriter = (rawUrl) => {
					let absolute = rawUrl;
					try {
						absolute = new URL(rawUrl, href).href;
					} catch {
						/* keep as-is; rewriteImageSource will reject non-URLs */
					}
					return rewriteImageSource(emailId, absolute, cidMap, assetByUrl);
				};
				const safe = sanitizeCssText(css, sheetRewrite);
				if (safe) styleBlocks.push(safe);
			}
		}
	}

	// Preserve the email's own <style> blocks (class-based / responsive styling)
	// instead of dropping them. We sanitize the CSS and pull the blocks out before
	// running sanitize-html (which would otherwise discard them), then re-emit them
	// as a single leading <style>. Rendering is safe: the preview iframe has no
	// scripts and a strict CSP, and the CSS is stripped of remote fetches.
	const htmlWithoutStyleTags = html.replace(
		/<style\b[^>]*>([\s\S]*?)<\/style\s*>/gi,
		(_full, css: string) => {
			const safe = sanitizeCssText(css, rewriteUrl);
			if (safe) styleBlocks.push(safe);
			return '';
		}
	);

	const sanitizedBody = sanitizeHtml(htmlWithoutStyleTags, {
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
				attribs: sanitizeCommonAttributes(attribs, rewriteUrl),
			}),
			a: (tagName, attribs) => {
				const sanitized = sanitizeCommonAttributes(attribs, rewriteUrl);
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
				const sanitized = sanitizeCommonAttributes(attribs, rewriteUrl);
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

	const styleTag = styleBlocks.length > 0 ? `<style>${styleBlocks.join('\n')}</style>` : '';
	return `${styleTag}${sanitizedBody}`;
}
