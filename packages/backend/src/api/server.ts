import express, { Express, type Request, type Response, type NextFunction } from 'express';
import dotenv from 'dotenv';
import { IngestionController } from './controllers/ingestion.controller';
import { ArchivedEmailController } from './controllers/archived-email.controller';
import { StorageController } from './controllers/storage.controller';
import { SearchController } from './controllers/search.controller';
import { createIngestionRouter } from './routes/ingestion.routes';
import { createArchivedEmailRouter } from './routes/archived-email.routes';
import { createStorageRouter } from './routes/storage.routes';
import { createSearchRouter } from './routes/search.routes';
import { createDashboardRouter } from './routes/dashboard.routes';
import { createUploadRouter } from './routes/upload.routes';
import { createUserRouter } from './routes/user.routes';
import { createSettingsRouter } from './routes/settings.routes';
import { createJobsRouter } from './routes/jobs.routes';
import { createContactsRouter } from './routes/contacts.routes';
import { StorageService } from '../services/StorageService';
import { SearchService } from '../services/SearchService';
import { SettingsService } from '../services/SettingsService';
import i18next from 'i18next';
import FsBackend from 'i18next-fs-backend';
import i18nextMiddleware from 'i18next-http-middleware';
import path from 'path';
import { logger } from '../config/logger';
import { withRetry } from '../helpers/retry';
import { rateLimiter } from './middleware/rateLimiter';
import { config } from '../config';
import { OpenArchiverFeature } from '@open-archiver/types';
// Define the "plugin" interface
export interface ArchiverModule {
	initialize: (app: Express) => Promise<void>;
	name: OpenArchiverFeature;
}

export async function createServer(modules: ArchiverModule[] = []): Promise<Express> {
	// Load environment variables
	dotenv.config();

	// --- Dependency Injection Setup ---
	const ingestionController = new IngestionController();
	const archivedEmailController = new ArchivedEmailController();
	const storageService = new StorageService();
	const storageController = new StorageController(storageService);
	const searchService = new SearchService();
	const searchController = new SearchController();
	const settingsService = new SettingsService();

	// --- i18next Initialization ---
	const initializeI18next = async () => {
		const systemSettings = await settingsService.getSystemSettings();
		const defaultLanguage = systemSettings?.language || 'en';
		logger.info({ language: defaultLanguage }, 'Default language');
		await i18next.use(FsBackend).init({
			lng: defaultLanguage,
			fallbackLng: defaultLanguage,
			ns: ['translation'],
			defaultNS: 'translation',
			backend: {
				loadPath: path.resolve(__dirname, '../locales/{{lng}}/{{ns}}.json'),
			},
		});
	};

	// Initialize i18next
	await initializeI18next();
	logger.info({}, 'i18next initialized');

	// Configure the Meilisearch index on startup. Retried so a boot that races
	// Meilisearch coming up (compose start, embedded child) doesn't crash.
	logger.info({}, 'Configuring email index...');
	await withRetry(() => searchService.configureEmailIndex(), {
		label: 'Meilisearch index configuration',
	});

	const app = express();

	// No CORS middleware: UI and API are served same-origin by this process.

	// Trust the proxy to get the real IP address of the client.
	// This is important for audit logging and security.
	app.set('trust proxy', true);

	// --- Routes ---
	const ingestionRouter = createIngestionRouter(ingestionController);
	const archivedEmailRouter = createArchivedEmailRouter(archivedEmailController);
	const storageRouter = createStorageRouter(storageController);
	const searchRouter = createSearchRouter(searchController);
	const dashboardRouter = createDashboardRouter();
	const contactsRouter = createContactsRouter();
	const uploadRouter = createUploadRouter();
	const userRouter = createUserRouter();
	const settingsRouter = createSettingsRouter();
	const jobsRouter = createJobsRouter();

	// The API answers on /v1 (SvelteKit's SSR proxy target) and /api/v1 (what the
	// browser calls, served directly by this process in single-process mode).
	// All API middleware is scoped to these bases so it never touches the
	// SvelteKit handler mounted after the routers (e.g. json() must not consume
	// request bodies destined for SvelteKit routes).
	const apiBases = [`/${config.api.version}`, `/api/${config.api.version}`];

	app.use(apiBases, (req, res, next) => {
		// req.path is relative to the mount base here (e.g. /settings/system).
		// exclude certain API endpoints from the rate limiter
		if (/^\/settings\/system$/.test(req.path)) {
			return next();
		}
		rateLimiter(req, res, next);
	});
	app.use(apiBases, express.json({ limit: '25mb' }));
	app.use(apiBases, express.urlencoded({ extended: true, limit: '25mb' }));

	// i18n middleware
	app.use(apiBases, i18nextMiddleware.handle(i18next));

	const mount = (route: string, router: express.Router) => {
		for (const base of apiBases) {
			app.use(`${base}${route}`, router);
		}
	};

	mount('/upload', uploadRouter);
	mount('/ingestion-sources', ingestionRouter);
	mount('/archived-emails', archivedEmailRouter);
	mount('/storage', storageRouter);
	mount('/search', searchRouter);
	mount('/dashboard', dashboardRouter);
	mount('/contacts', contactsRouter);
	mount('/users', userRouter);
	mount('/settings', settingsRouter);
	mount('/jobs', jobsRouter);

	// Load all provided extension modules
	for (const module of modules) {
		await module.initialize(app);
		logger.info(`🏢 Enterprise module loaded: ${module.name}`);
	}
	// Liveness probe (also polled by the future desktop shell's splash screen).
	// The frontend handler mounted after the routers serves `/` itself.
	app.get('/healthz', (req, res) => {
		res.json({ status: 'ok' });
	});
	logger.info('✅ Core OSS modules loaded.');

	return app;
}
