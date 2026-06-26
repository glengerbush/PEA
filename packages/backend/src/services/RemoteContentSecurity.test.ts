import { describe, expect, it, vi } from 'vitest';
import {
	assertSafeRemoteUrl,
	BlockedRemoteContentError,
	detectImageContentType,
	isBlockedIpAddress,
	validateArchivableContent,
} from './RemoteContentSecurity';

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

describe('remote content address validation', () => {
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
