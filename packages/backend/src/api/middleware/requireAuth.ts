import type { Request, Response, NextFunction } from 'express';
import type { AuthService } from '../../services/AuthService';
import type { AuthTokenPayload } from '@open-archiver/types';
import 'dotenv/config';
import { UserService } from '../../services/UserService';

// By using module augmentation, we can add our custom 'user' property
// to the Express Request interface in a type-safe way.
declare global {
	namespace Express {
		export interface Request {
			user?: AuthTokenPayload;
		}
	}
}

// -----------------------------------------------------------------------------
// LOCAL MODE: authentication is disabled.
//
// The login requirement has been removed for local use. Rather than verifying a
// JWT or API key, every request is treated as the single local user (the first
// user in the database, which owns all archived content). The user identity is
// still populated on `req.user` so ownership filters, audit logging, and
// foreign-key constraints continue to work unchanged.
//
// To restore real authentication later, revert this file to its token-verifying
// implementation — no other backend files need to change.
// -----------------------------------------------------------------------------

let cachedLocalUser: AuthTokenPayload | null = null;

async function resolveLocalUser(): Promise<AuthTokenPayload> {
	if (cachedLocalUser) return cachedLocalUser;
	const user = await new UserService().getOrCreateLocalUser();
	cachedLocalUser = { sub: user.id, email: user.email };
	return cachedLocalUser;
}

export const requireAuth = (_authService: AuthService) => {
	return async (req: Request, res: Response, next: NextFunction) => {
		try {
			req.user = await resolveLocalUser();
			next();
		} catch (error) {
			console.error('Failed to resolve local user:', error);
			return res
				.status(500)
				.json({ message: 'An internal server error occurred resolving the local user' });
		}
	};
};
