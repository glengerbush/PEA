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

function sanitize(html: string, cssByUrl?: Map<string, string>): string {
	return sanitizeEmailPreviewHtml({
		emailId: 'email-1',
		html,
		cidMap: new Map([['inline-logo', 'data:image/png;base64,aW1hZ2U=']]),
		assets: [archivedAsset],
		cssByUrl,
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
		// The inline style is preserved (color/width survive) but the remote
		// background URL is neutralized rather than fetched.
		expect(html).toContain('color:red');
		expect(html).toContain('width:10px');
		expect(html).toContain("background:url('data:,')");
		expect(html).not.toContain('onclick');
		expect(html).not.toContain('onerror');
		expect(html).not.toContain('<script');
		expect(html).not.toContain('https://evil.test');
		expect(html).not.toContain('https://cdn.example.com/logo.png');
	});

	it('inlines an archived external stylesheet as sanitized CSS, neutralizing malicious content', () => {
		const maliciousCss = `
			@import url(https://evil.test/more.css);
			.x { color: blue; behavior: url(#default#bad); width: expression(alert(1)); }
			/* breakout attempt: */ </style><script>alert(1)</script>
			.bg { background: url(https://cdn.example.com/logo.png); }
			.rel { background: url(./logo.png); }
		`;
		const html = sanitize(
			`<head><link rel="stylesheet" href="https://cdn.example.com/theme.css"></head><body><div class="x bg">Hi</div></body>`,
			new Map([['https://cdn.example.com/theme.css', maliciousCss]])
		);

		// The stylesheet is inlined (so it actually applies) but fully sanitized.
		expect(html).toContain('<style>');
		expect(html).toContain('color: blue');
		expect(html).not.toContain('@import');
		expect(html).not.toContain('<script');
		expect(html).not.toContain('</style><script'); // no breakout
		expect(html).not.toContain('expression(');
		expect(html).not.toContain('behavior:');
		expect(html).not.toContain('evil.test');
		// Both absolute and relative (resolved against the stylesheet URL) url()s
		// that map to the archived asset are rewritten to the local copy.
		expect(html).toContain("url('/api/v1/archived-emails/email-1/remote-assets/asset-1')");
	});

	it('does not inline a <link> whose CSS was not archived', () => {
		const html = sanitize(
			`<link rel="stylesheet" href="https://cdn.example.com/missing.css"><div>Hi</div>`,
			new Map()
		);
		expect(html).not.toContain('cdn.example.com/missing.css');
		expect(html).not.toContain('<link');
	});

	it('preserves <style> blocks, rewriting archived url() and neutralizing the rest', () => {
		const html = sanitize(`
			<style>
				@import url(https://evil.test/import.css);
				.hero { background-image: url(https://cdn.example.com/logo.png); color: #333; }
				.x { background: url(https://evil.test/tracker.gif); behavior: url(#default#bad); }
			</style>
			<div class="hero">Hi</div>
		`);

		expect(html).toContain('<style>');
		expect(html).toContain('color: #333');
		// archived asset url() rewritten to the local asset
		expect(html).toContain("url('/api/v1/archived-emails/email-1/remote-assets/asset-1')");
		// remote/import/behavior all neutralized
		expect(html).not.toContain('@import');
		expect(html).not.toContain('evil.test');
		expect(html).not.toContain('behavior:');
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
