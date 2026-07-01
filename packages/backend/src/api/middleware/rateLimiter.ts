import { rateLimit, ipKeyGenerator } from 'express-rate-limit';
import { config } from '../../config';

const windowInMinutes = Math.ceil(config.api.rateLimit.windowMs / 60000);

/**
 * Loopback and RFC1918 / unique-local addresses. In this personal, single-user
 * deployment the frontend (SvelteKit SSR) and backend talk over the Docker
 * network, so every API call shares one source IP — a per-IP limit would
 * throttle normal browsing. Skip rate limiting for local/private traffic;
 * a genuinely public client IP is still limited.
 */
function isLocalOrPrivateAddress(ip: string): boolean {
	if (!ip) return false;
	const v = ip.replace(/^::ffff:/, '').toLowerCase();
	if (v === '::1' || v === '127.0.0.1') return true;
	if (v.startsWith('127.') || v.startsWith('10.') || v.startsWith('192.168.')) return true;
	if (/^172\.(1[6-9]|2[0-9]|3[01])\./.test(v)) return true;
	if (v.startsWith('fc') || v.startsWith('fd')) return true; // IPv6 unique-local
	return false;
}

export const rateLimiter = rateLimit({
	windowMs: config.api.rateLimit.windowMs,
	max: config.api.rateLimit.max,
	skip: (req) => isLocalOrPrivateAddress(req.ip || ''),
	keyGenerator: (req, res) => {
		// Use the real IP address of the client, even if it's behind a proxy.
		// `app.set('trust proxy', true)` in `server.ts`.
		return ipKeyGenerator(req.ip || 'unknown');
	},
	message: {
		status: 429,
		message: `Too many requests from this IP, please try again after ${windowInMinutes} minutes`,
	},
	statusCode: 429,
	standardHeaders: true,
	legacyHeaders: false,
});
