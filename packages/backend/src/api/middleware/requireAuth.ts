import type { Request, Response, NextFunction } from 'express';
import type { AuthService } from '../../services/AuthService';
import type { AuthTokenPayload } from '@open-archiver/types';
import 'dotenv/config';
import { ApiKeyService } from '../../services/ApiKeyService';
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

function readCookieValue(cookieHeader: string | undefined, name: string): string | null {
	if (!cookieHeader) return null;

	for (const cookie of cookieHeader.split(';')) {
		const [rawKey, ...rawValue] = cookie.trim().split('=');
		if (rawKey === name) {
			return decodeURIComponent(rawValue.join('='));
		}
	}

	return null;
}

export const requireAuth = (authService: AuthService) => {
	return async (req: Request, res: Response, next: NextFunction) => {
		const authHeader = req.headers.authorization;
		const apiKeyHeader = req.headers['x-api-key'];

		if (apiKeyHeader) {
			const userId = await ApiKeyService.validateKey(apiKeyHeader as string);
			if (!userId) {
				return res.status(401).json({ message: 'Unauthorized: Invalid API key' });
			}
			const user = await new UserService().findById(userId);
			if (!user) {
				return res.status(401).json({ message: 'Unauthorized: Invalid user' });
			}
			req.user = {
				sub: user.id,
				email: user.email,
			};
			return next();
		}

		const cookieToken = readCookieValue(req.headers.cookie, 'accessToken');
		if ((!authHeader || !authHeader.startsWith('Bearer ')) && !cookieToken) {
			return res.status(401).json({ message: 'Unauthorized: No token provided' });
		}
		const token = authHeader?.startsWith('Bearer ') ? authHeader.split(' ')[1] : cookieToken;
		try {
			if (!token) {
				return res.status(401).json({ message: 'Unauthorized: No token provided' });
			}
			const payload = await authService.verifyToken(token);
			if (!payload) {
				return res.status(401).json({ message: 'Unauthorized: Invalid token' });
			}
			req.user = payload;
			next();
		} catch (error) {
			console.error('Authentication error:', error);
			return res
				.status(500)
				.json({ message: 'An internal server error occurred during authentication' });
		}
	};
};
