/**
 * The request identity attached to every API request. Authentication is
 * removed in this fork — every request resolves to the single local user —
 * but ownership filters, audit attribution, and FK integrity still key off
 * this shape.
 */
export interface AuthTokenPayload {
	[claim: string]: unknown;
	/**
	 * The user's unique identifier.
	 */
	sub: string;
	/**
	 * The user's email address.
	 */
	email: string;
}
