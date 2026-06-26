/**
 * Represents a user account in the system.
 * This is the core user object that will be stored in the database.
 */
export interface User {
	id: string;
	first_name: string | null;
	last_name: string | null;
	email: string;
	createdAt: Date;
}

/**
 * Represents a user's session.
 * This is used to track a user's login status.
 */
export interface Session {
	id: string;
	userId: string;
	expiresAt: Date;
}

export interface ApiKey {
	id: string;
	name: string;
	key: string;
	expiresAt: string;
	createdAt: string;
}
