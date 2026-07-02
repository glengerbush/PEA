const BASE_URL = '/api/v1'; // Using a relative URL for proxying

/**
 * A custom fetch wrapper for the client-side API (single local user — no auth).
 * @param url The URL to fetch, relative to the API base.
 * @param options The standard Fetch API options.
 * @returns A Promise that resolves to the Fetch Response.
 */
export const api = async (url: string, options: RequestInit = {}): Promise<Response> => {
	const defaultHeaders: HeadersInit = {};

	if (!(options.body instanceof FormData)) {
		defaultHeaders['Content-Type'] = 'application/json';
	}

	const mergedOptions: RequestInit = {
		...options,
		headers: {
			...defaultHeaders,
			...options.headers,
		},
	};

	return fetch(`${BASE_URL}${url}`, mergedOptions);
};
