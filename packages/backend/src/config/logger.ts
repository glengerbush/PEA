import pino from 'pino';

// The pino-pretty transport resolves its module by name at runtime in a worker
// thread — that breaks inside the bundled desktop build (single-file esbuild
// output, no node_modules). OA_BUNDLED is set by the desktop shell; bundled
// builds log plain JSON to stdout instead.
const usePretty = process.env.OA_BUNDLED !== '1';

export const logger = pino({
	level: process.env.LOG_LEVEL || 'info',
	redact: ['password'],
	...(usePretty
		? {
				transport: {
					target: 'pino-pretty',
					options: {
						colorize: true,
					},
				},
			}
		: {}),
});
