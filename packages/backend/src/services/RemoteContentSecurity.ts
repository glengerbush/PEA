import { lookup } from 'dns/promises';
import net from 'net';

export type SafeResolvedAddress = { address: string; family: 4 | 6 };
type LookupAddress = { address: string; family: number };
type RemoteContentLookup = (hostname: string) => Promise<LookupAddress[]>;

export const SAFE_IMAGE_TYPES = new Set([
	'image/png',
	'image/jpeg',
	'image/gif',
	'image/webp',
	'image/avif',
]);

export class BlockedRemoteContentError extends Error {}

export function normalizeContentType(value: string | null): string | null {
	if (!value) return null;
	const contentType = value.split(';')[0].trim().toLowerCase();
	if (contentType === 'image/jpg') return 'image/jpeg';
	return contentType || null;
}

export function isSafePreviewContentType(contentType: string | null): boolean {
	return contentType ? SAFE_IMAGE_TYPES.has(contentType) : false;
}

export function getHeaderValue(value: string | string[] | undefined): string | null {
	if (Array.isArray(value)) return value[0] || null;
	return value || null;
}

export function isDefaultRemotePort(url: URL): boolean {
	const port = url.port ? Number(url.port) : url.protocol === 'https:' ? 443 : 80;
	if (!Number.isInteger(port)) return false;
	if (url.protocol === 'http:') return port === 80;
	if (url.protocol === 'https:') return port === 443;
	return false;
}

export function detectImageContentType(body: Buffer): string | null {
	if (
		body.length >= 8 &&
		body[0] === 0x89 &&
		body[1] === 0x50 &&
		body[2] === 0x4e &&
		body[3] === 0x47 &&
		body[4] === 0x0d &&
		body[5] === 0x0a &&
		body[6] === 0x1a &&
		body[7] === 0x0a
	) {
		return 'image/png';
	}

	if (body.length >= 3 && body[0] === 0xff && body[1] === 0xd8 && body[2] === 0xff) {
		return 'image/jpeg';
	}

	if (
		body.length >= 6 &&
		(body.subarray(0, 6).toString('ascii') === 'GIF87a' ||
			body.subarray(0, 6).toString('ascii') === 'GIF89a')
	) {
		return 'image/gif';
	}

	if (
		body.length >= 12 &&
		body.subarray(0, 4).toString('ascii') === 'RIFF' &&
		body.subarray(8, 12).toString('ascii') === 'WEBP'
	) {
		return 'image/webp';
	}

	if (body.length >= 16 && body.subarray(4, 8).toString('ascii') === 'ftyp') {
		const brands = body.subarray(8, Math.min(body.length, 64)).toString('ascii');
		if (brands.includes('avif') || brands.includes('avis')) {
			return 'image/avif';
		}
	}

	return null;
}

export function validateArchivableContent(body: Buffer, contentTypeHeader: string | null): string {
	if (body.length === 0) {
		throw new BlockedRemoteContentError('Remote content is empty');
	}

	const sniffedContentType = detectImageContentType(body);
	if (!sniffedContentType) {
		throw new BlockedRemoteContentError('Remote content type is not archivable');
	}

	return sniffedContentType;
}

function isPrivateIPv4(address: string): boolean {
	const parts = address.split('.').map((part) => Number(part));
	if (
		parts.length !== 4 ||
		parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)
	) {
		return true;
	}

	const [a, b] = parts;
	return (
		a === 0 ||
		a === 10 ||
		a === 127 ||
		(a === 169 && b === 254) ||
		(a === 172 && b >= 16 && b <= 31) ||
		(a === 192 && b === 168) ||
		(a === 100 && b >= 64 && b <= 127) ||
		(a === 192 && b === 0) ||
		(a === 192 && b === 2) ||
		(a === 198 && (b === 18 || b === 19)) ||
		(a === 198 && b === 51) ||
		(a === 203 && b === 0) ||
		a >= 224
	);
}

function isPrivateIPv6(address: string): boolean {
	const normalized = address.toLowerCase();
	if (
		normalized === '::' ||
		normalized === '::1' ||
		normalized.startsWith('fc') ||
		normalized.startsWith('fd') ||
		/^fe[89ab]/.test(normalized) ||
		normalized.startsWith('ff')
	) {
		return true;
	}

	if (normalized.startsWith('::ffff:')) {
		return isPrivateIPv4(normalized.slice('::ffff:'.length));
	}

	return false;
}

export function isBlockedIpAddress(address: string): boolean {
	const family = net.isIP(address);
	if (family === 4) return isPrivateIPv4(address);
	if (family === 6) return isPrivateIPv6(address);
	return true;
}

async function lookupRemoteHostname(hostname: string): Promise<LookupAddress[]> {
	return lookup(hostname, { all: true, verbatim: true });
}

export async function assertSafeRemoteUrl(
	url: URL,
	resolveHostname: RemoteContentLookup = lookupRemoteHostname
): Promise<SafeResolvedAddress> {
	if (url.protocol !== 'http:' && url.protocol !== 'https:') {
		throw new BlockedRemoteContentError('Unsupported remote content protocol');
	}
	if (url.username || url.password) {
		throw new BlockedRemoteContentError('Credentialed remote content URLs are blocked');
	}
	if (!isDefaultRemotePort(url)) {
		throw new BlockedRemoteContentError('Non-standard remote content ports are blocked');
	}

	const hostname = url.hostname.toLowerCase();
	if (hostname === 'localhost' || hostname.endsWith('.localhost')) {
		throw new BlockedRemoteContentError('Localhost remote content is blocked');
	}

	const lookupHostname = hostname.replace(/^\[|\]$/g, '');
	const literalIpFamily = net.isIP(lookupHostname);
	if (literalIpFamily) {
		if (isBlockedIpAddress(lookupHostname)) {
			throw new BlockedRemoteContentError('Private or local network addresses are blocked');
		}
		return { address: lookupHostname, family: literalIpFamily as 4 | 6 };
	}

	const addresses = await resolveHostname(lookupHostname);
	if (addresses.length === 0) {
		throw new BlockedRemoteContentError('Remote content host did not resolve');
	}
	if (addresses.some((address) => isBlockedIpAddress(address.address))) {
		throw new BlockedRemoteContentError('Private or local network addresses are blocked');
	}
	const selectedAddress = addresses.find(
		(address): address is SafeResolvedAddress => address.family === 4 || address.family === 6
	);
	if (!selectedAddress) {
		throw new BlockedRemoteContentError('Remote content host did not resolve');
	}
	return selectedAddress;
}
