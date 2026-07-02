import type { Request, Response, NextFunction } from 'express';
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
// Authentication is removed in this fork (single-user desktop app).
//
// Every request is attributed to the single local user (the first user in the
// database, which owns all archived content). The identity is populated on
// `req.user` so ownership filters, audit logging, and foreign-key constraints
// continue to work unchanged.
// -----------------------------------------------------------------------------

let cachedLocalUser: AuthTokenPayload | null = null;

async function resolveLocalUser(): Promise<AuthTokenPayload> {
	if (cachedLocalUser) return cachedLocalUser;
	const user = await new UserService().getOrCreateLocalUser();
	cachedLocalUser = { sub: user.id, email: user.email };
	return cachedLocalUser;
}

/** Attaches the local user to the request (name kept to minimize churn). */
export const requireAuth = () => {
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
