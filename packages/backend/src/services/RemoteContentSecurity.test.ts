import { describe, expect, it, vi } from 'vitest';
import {
	assertSafeRemoteUrl,
	BlockedRemoteContentError,
	createPinnedLookup,
	detectImageContentType,
	isBlockedIpAddress,
	isLikelyTrackingPixel,
	isLikelyTrackingUrl,
	MAX_STYLESHEET_BYTES,
	validateArchivableContent,
} from './RemoteContentSecurity';

const attrs = (entries: Record<string, string>): Map<string, string> =>
	new Map(Object.entries(entries));

const png = Buffer.from('89504e470d0a1a0a0000000d49484452', 'hex');
const jpeg = Buffer.from('ffd8ffe000104a464946', 'hex');
const gif = Buffer.from('GIF89a', 'ascii');
const webp = Buffer.from('524946460000000057454250', 'hex');
const avif = Buffer.from('00000018667479706176696600000000', 'hex');

describe('remote content image validation', () => {
	it.each([
		['png', png, 'image/png'],
		['jpeg', jpeg, 'image/jpeg'],
		['gif', gif, 'image/gif'],
		['webp', webp, 'image/webp'],
		['avif', avif, 'image/avif'],
	])('detects %s byte signatures', (_name, body, contentType) => {
		expect(detectImageContentType(body)).toBe(contentType);
		expect(validateArchivableContent(body, null)).toBe(contentType);
	});

	it('does not trust an image content-type header without image bytes', () => {
		const html = Buffer.from('<html><script>alert(1)</script></html>');

		expect(() => validateArchivableContent(html, 'image/png')).toThrow(
			BlockedRemoteContentError
		);
	});

	it('uses sniffed bytes instead of a misleading non-image content type', () => {
		expect(validateArchivableContent(png, 'application/octet-stream')).toBe('image/png');
	});
});

describe('tracking pixel detection', () => {
	it.each([
		['1x1 attributes', { width: '1', height: '1' }],
		['2x2 attributes', { width: '2', height: '2' }],
		['1px style dimensions', { style: 'width:1px;height:1px' }],
		['display:none', { style: 'display:none', width: '600', height: '400' }],
		['visibility:hidden', { style: 'visibility:hidden' }],
	])('flags %s as a tracking pixel', (_name, attributes) => {
		expect(isLikelyTrackingPixel(attrs(attributes))).toBe(true);
	});

	it.each([
		['a normally sized image', { width: '600', height: '300' }],
		['a small but visible icon', { width: '16', height: '16' }],
		['only one tiny dimension', { width: '1' }],
		['no dimension hints', {}],
	])('does not flag %s', (_name, attributes) => {
		expect(isLikelyTrackingPixel(attrs(attributes))).toBe(false);
	});

	it.each([
		'https://example.us1.list-manage.com/track/open.php?u=abc&id=def',
		'https://click.example.com/wf/open?upn=abc',
		'https://t.example.com/e/open?token=abc',
		'https://example.com/o.gif?mid=123',
		'https://track.example.com/open.aspx?id=9',
	])('flags known tracking URL %s', (url) => {
		expect(isLikelyTrackingUrl(url)).toBe(true);
	});

	it.each([
		'https://cdn.example.com/images/logo.png',
		'https://example.com/assets/hero-open-house.jpg',
		'https://example.com/photos/p.webp',
	])('does not flag legitimate image URL %s', (url) => {
		expect(isLikelyTrackingUrl(url)).toBe(false);
	});
});

describe('remote content address validation', () => {
	it('pins both single-address and all-address DNS lookups', () => {
		const pinnedAddress = { address: '93.184.216.34', family: 4 as const };
		const pinnedLookup = createPinnedLookup(pinnedAddress);
		const singleCallback = vi.fn();
		const allCallback = vi.fn();

		pinnedLookup('example.com', { all: false }, singleCallback);
		pinnedLookup('example.com', { all: true }, allCallback);

		expect(singleCallback).toHaveBeenCalledWith(null, pinnedAddress.address, 4);
		expect(allCallback).toHaveBeenCalledWith(null, [pinnedAddress]);
	});

	it.each([
		'0.0.0.0',
		'10.0.0.10',
		'127.0.0.1',
		'169.254.1.1',
		'172.16.0.1',
		'192.168.0.1',
		'100.64.0.1',
		'::1',
		'fc00::1',
		'fe80::1',
		'::ffff:127.0.0.1',
	])('blocks private/local address %s', (address) => {
		expect(isBlockedIpAddress(address)).toBe(true);
	});

	it('accepts a host when all resolved addresses are public', async () => {
		const lookup = vi.fn().mockResolvedValue([{ address: '93.184.216.34', family: 4 }]);

		await expect(
			assertSafeRemoteUrl(new URL('https://example.com/image.png'), lookup)
		).resolves.toEqual({
			address: '93.184.216.34',
			family: 4,
		});
		expect(lookup).toHaveBeenCalledWith('example.com');
	});

	it('blocks a host if any DNS answer is private', async () => {
		const lookup = vi.fn().mockResolvedValue([
			{ address: '93.184.216.34', family: 4 },
			{ address: '127.0.0.1', family: 4 },
		]);

		await expect(
			assertSafeRemoteUrl(new URL('https://example.com/image.png'), lookup)
		).rejects.toThrow(BlockedRemoteContentError);
	});

	it.each([
		'file:///tmp/payload.png',
		'https://user:pass@example.com/image.png',
		'http://localhost/image.png',
		'http://example.localhost/image.png',
		'http://example.com:8080/image.png',
		'http://127.0.0.1/image.png',
		'http://[::1]/image.png',
	])('blocks unsafe URL %s', async (value) => {
		await expect(assertSafeRemoteUrl(new URL(value), vi.fn())).rejects.toThrow(
			BlockedRemoteContentError
		);
	});
});

describe('stylesheet content validation', () => {
	it('accepts text/css declared content', () => {
		const css = Buffer.from('.a{color:red}\n.b{color:blue}', 'utf8');
		expect(validateArchivableContent(css, 'text/css; charset=utf-8')).toBe('text/css');
	});

	it('rejects non-css, non-image content even if non-empty', () => {
		const text = Buffer.from('hello world', 'utf8');
		expect(() => validateArchivableContent(text, 'text/plain')).toThrow(
			'Remote content type is not archivable'
		);
	});

	it('rejects a binary payload mislabelled as text/css', () => {
		const binary = Buffer.from([0x00, 0x01, 0x02, 0x03, 0x00, 0xff, 0xfe]);
		expect(() => validateArchivableContent(binary, 'text/css')).toThrow('not text');
	});

	it('rejects an oversized stylesheet', () => {
		const huge = Buffer.alloc(MAX_STYLESHEET_BYTES + 1, 0x61); // 'a'
		expect(() => validateArchivableContent(huge, 'text/css')).toThrow('too large');
	});

	it('does not treat css text declared as an image as archivable', () => {
		const css = Buffer.from('.a{color:red}', 'utf8');
		expect(() => validateArchivableContent(css, 'image/png')).toThrow(
			'Remote content type is not archivable'
		);
	});
});
