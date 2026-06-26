import { describe, expect, it } from 'vitest';
import {
	sanitizeEmailPreviewHtml,
	type RemoteContentPreviewAsset,
} from './RemoteContentPreviewSanitizer';

const archivedAsset: RemoteContentPreviewAsset = {
	id: 'asset-1',
	originalUrl: 'https://cdn.example.com/logo.png',
	finalUrl: 'https://cdn.example.com/logo.png?cache=1',
	status: 'archived',
	storagePath: 'remote-content/email-1/asset-1.png',
	contentType: 'image/png',
};

function sanitize(html: string): string {
	return sanitizeEmailPreviewHtml({
		emailId: 'email-1',
		html,
		cidMap: new Map([['inline-logo', 'data:image/png;base64,aW1hZ2U=']]),
		assets: [archivedAsset],
	});
}

describe('remote content preview sanitizer', () => {
	it('removes dangerous markup and rewrites archived remote images', () => {
		const html = sanitize(`
			<div onclick="alert(1)" style="color:red; background:url(https://evil.test/x); width:10px">
				<script>alert(1)</script>
				<img src="https://cdn.example.com/logo.png" onerror="alert(1)">
			</div>
		`);

		expect(html).toContain('/api/v1/archived-emails/email-1/remote-assets/asset-1');
		expect(html).toContain('style="color:red; width:10px"');
		expect(html).not.toContain('onclick');
		expect(html).not.toContain('onerror');
		expect(html).not.toContain('<script');
		expect(html).not.toContain('https://evil.test');
		expect(html).not.toContain('https://cdn.example.com/logo.png');
	});

	it('blocks unsafe links while hardening safe links', () => {
		const html = sanitize(`
			<a href="javascript:alert(1)">bad</a>
			<a href="https://example.com/report">good</a>
		`);

		expect(html).not.toContain('javascript:');
		expect(html).toContain('href="https://example.com/report"');
		expect(html).toContain('target="_blank"');
		expect(html).toContain('rel="noopener noreferrer"');
	});

	it('allows safe CID images and blocks unarchived or unsafe data images', () => {
		const html = sanitize(`
			<img src="cid:inline-logo">
			<img src="https://not-archived.example.com/pixel.png">
			<img src="data:image/svg+xml;base64,PHN2Zy8+">
		`);

		expect(html).toContain('data:image/png;base64,aW1hZ2U=');
		expect(html).not.toContain('not-archived.example.com');
		expect(html).not.toContain('image/svg+xml');
	});

	it('sanitizes srcset entries and drops unsafe descriptors', () => {
		const html = sanitize(`
			<img srcset="https://cdn.example.com/logo.png 1x javascript:alert(1),
				https://cdn.example.com/logo.png 300w">
		`);

		expect(html).toContain('/api/v1/archived-emails/email-1/remote-assets/asset-1 1x');
		expect(html).toContain('/api/v1/archived-emails/email-1/remote-assets/asset-1 300w');
		expect(html).not.toContain('javascript:alert');
	});
});
