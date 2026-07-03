/** Any SvelteKit load event (universal or server) — only fetch is needed. */
interface FetchEvent {
	fetch: typeof fetch;
}

const BASE_URL = '/api/v1'; // Using a relative URL for proxying

/**
 * A custom fetch wrapper for the load-function API (single local user — no auth).
 * @param url The URL to fetch, relative to the API base.
 * @param event The SvelteKit request event.
 * @param options The standard Fetch API options.
 * @returns A Promise that resolves to the Fetch Response.
 */
export const api = async (
	url: string,
	event: FetchEvent,
	options: RequestInit = {}
): Promise<Response> => {
	const defaultHeaders: HeadersInit = {
		'Content-Type': 'application/json',
	};

	const mergedOptions: RequestInit = {
		...options,
		headers: {
			...defaultHeaders,
			...options.headers,
		},
	};

	return event.fetch(`${BASE_URL}${url}`, mergedOptions);
};
