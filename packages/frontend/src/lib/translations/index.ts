import { readable } from 'svelte/store';
import en from './en.json';

// Single-user local app — English only, no locale switching. A small dot-path
// lookup backing `$t('app.x.y')` and `$t('app.x.y', { count })`.

type Params = Record<string, string | number>;

function lookup(key: string): string {
	let node: unknown = en;
	for (const part of key.split('.')) {
		if (node && typeof node === 'object' && part in (node as Record<string, unknown>)) {
			node = (node as Record<string, unknown>)[part];
		} else {
			return key; // missing key falls back to the key itself
		}
	}
	return typeof node === 'string' ? node : key;
}

function translate(key: string, params?: Params): string {
	const text = lookup(key);
	if (!params) return text;
	return text.replace(/\{\{\s*(\w+)\s*\}\}/g, (_, name: string) =>
		name in params ? String(params[name]) : `{{${name}}}`
	);
}

/** Translator exposed as a store so `$t(...)` keeps working. Value is static. */
export const t = readable(translate);
