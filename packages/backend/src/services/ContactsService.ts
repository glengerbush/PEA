import { sql } from 'drizzle-orm';
import { db } from '../database';
import { contacts } from '../database/schema';
import type {
	ContactImportFormat,
	ContactMap,
	ImportContactsResult,
} from '@open-archiver/types';
import { logger } from '../config/logger';

interface ParsedContact {
	email: string;
	displayName: string;
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function normalizeEmail(value: string): string {
	return value.trim().toLowerCase().replace(/^mailto:/, '');
}

/** Split a single CSV line, honoring double-quoted fields. */
function splitCsvLine(line: string): string[] {
	const out: string[] = [];
	let cur = '';
	let inQuotes = false;
	for (let i = 0; i < line.length; i++) {
		const ch = line[i];
		if (inQuotes) {
			if (ch === '"') {
				if (line[i + 1] === '"') {
					cur += '"';
					i++;
				} else {
					inQuotes = false;
				}
			} else {
				cur += ch;
			}
		} else if (ch === '"') {
			inQuotes = true;
		} else if (ch === ',') {
			out.push(cur);
			cur = '';
		} else {
			cur += ch;
		}
	}
	out.push(cur);
	return out.map((c) => c.trim());
}

export class ContactsService {
	/**
	 * Parse a CSV. Detects an email column (header contains "email") and a name
	 * column (a "name"/"display name"/"full name" header, or first + last name).
	 * Falls back to scanning every cell for an email-looking value.
	 */
	static parseCsv(content: string): ParsedContact[] {
		const lines = content.split(/\r?\n/).filter((l) => l.trim() !== '');
		if (lines.length === 0) return [];

		const header = splitCsvLine(lines[0]).map((h) => h.toLowerCase());
		const findCol = (...needles: string[]) =>
			header.findIndex((h) => needles.some((n) => h === n || h.includes(n)));

		const emailIdx = findCol('e-mail address', 'email address', 'email', 'e-mail');
		const nameIdx = findCol('display name', 'full name', 'name');
		const firstIdx = findCol('first name', 'given name', 'first');
		const lastIdx = findCol('last name', 'family name', 'surname', 'last');

		const results: ParsedContact[] = [];
		for (let i = 1; i < lines.length; i++) {
			const cells = splitCsvLine(lines[i]);
			let email = emailIdx >= 0 ? cells[emailIdx] : '';
			if (!email || !EMAIL_RE.test(normalizeEmail(email))) {
				// fall back to any email-looking cell
				email = cells.find((c) => EMAIL_RE.test(normalizeEmail(c))) || '';
			}
			email = normalizeEmail(email);
			if (!EMAIL_RE.test(email)) continue;

			let name = '';
			if (nameIdx >= 0 && cells[nameIdx]) {
				name = cells[nameIdx].trim();
			} else {
				const parts = [
					firstIdx >= 0 ? cells[firstIdx] : '',
					lastIdx >= 0 ? cells[lastIdx] : '',
				]
					.map((p) => (p || '').trim())
					.filter(Boolean);
				name = parts.join(' ');
			}
			results.push({ email, displayName: name || email });
		}
		return results;
	}

	/** Parse a vCard (.vcf) file — one contact per VCARD block, using FN + EMAIL. */
	static parseVcf(content: string): ParsedContact[] {
		const results: ParsedContact[] = [];
		const blocks = content.split(/BEGIN:VCARD/i).slice(1);
		for (const block of blocks) {
			const lines = block.split(/\r?\n/);
			let fn = '';
			let n = '';
			const emails: string[] = [];
			for (const raw of lines) {
				const line = raw.trim();
				if (/^FN[:;]/i.test(line)) {
					fn = line.replace(/^FN[^:]*:/i, '').trim();
				} else if (/^N[:;]/i.test(line)) {
					// N:Last;First;Middle;Prefix;Suffix
					const val = line.replace(/^N[^:]*:/i, '');
					const [last = '', first = ''] = val.split(';');
					n = [first, last].map((p) => p.trim()).filter(Boolean).join(' ');
				} else if (/^EMAIL[:;]/i.test(line)) {
					const val = normalizeEmail(line.replace(/^EMAIL[^:]*:/i, ''));
					if (EMAIL_RE.test(val)) emails.push(val);
				}
			}
			const name = fn || n;
			for (const email of emails) {
				results.push({ email, displayName: name || email });
			}
		}
		return results;
	}

	static async importContacts(
		format: ContactImportFormat,
		content: string
	): Promise<ImportContactsResult> {
		const parsedRaw = format === 'vcf' ? this.parseVcf(content) : this.parseCsv(content);

		// De-dupe within the file by email (last name wins).
		const byEmail = new Map<string, ParsedContact>();
		for (const c of parsedRaw) {
			if (c.email) byEmail.set(c.email, c);
		}
		const parsed = Array.from(byEmail.values());

		let imported = 0;
		let updated = 0;
		for (const c of parsed) {
			try {
				const result = await db
					.insert(contacts)
					.values({ email: c.email, displayName: c.displayName, source: format })
					.onConflictDoUpdate({
						target: contacts.email,
						set: { displayName: c.displayName, source: format, updatedAt: new Date() },
					})
					.returning({ createdAt: contacts.createdAt, updatedAt: contacts.updatedAt });
				const row = result[0];
				if (row && row.createdAt.getTime() === row.updatedAt.getTime()) imported += 1;
				else updated += 1;
			} catch (error) {
				logger.warn(
					{ email: c.email, error: error instanceof Error ? error.message : String(error) },
					'Failed to import contact'
				);
			}
		}

		return {
			parsed: parsedRaw.length,
			imported,
			updated,
			skipped: parsedRaw.length - parsed.length,
		};
	}

	/** Returns a lowercased-email → display-name map for resolving names in the UI. */
	static async getContactMap(): Promise<ContactMap> {
		const rows = await db
			.select({ email: contacts.email, displayName: contacts.displayName })
			.from(contacts);
		const map: ContactMap = {};
		for (const r of rows) {
			if (r.email && r.displayName) map[r.email] = r.displayName;
		}
		return map;
	}
}
