import express, { Express, type Request, type Response, type NextFunction } from 'express';
import dotenv from 'dotenv';
import { AuthController } from './controllers/auth.controller';
import { IngestionController } from './controllers/ingestion.controller';
import { ArchivedEmailController } from './controllers/archived-email.controller';
import { StorageController } from './controllers/storage.controller';
import { SearchController } from './controllers/search.controller';
import { createAuthRouter } from './routes/auth.routes';
import { createIngestionRouter } from './routes/ingestion.routes';
import { createArchivedEmailRouter } from './routes/archived-email.routes';
import { createStorageRouter } from './routes/storage.routes';
import { createSearchRouter } from './routes/search.routes';
import { createDashboardRouter } from './routes/dashboard.routes';
import { createUploadRouter } from './routes/upload.routes';
import { createUserRouter } from './routes/user.routes';
import { createSettingsRouter } from './routes/settings.routes';
import { apiKeyRoutes } from './routes/api-key.routes';
import { createJobsRouter } from './routes/jobs.routes';
import { createContactsRouter } from './routes/contacts.routes';
import { AuthService } from '../services/AuthService';
import { UserService } from '../services/UserService';
import { StorageService } from '../services/StorageService';
import { SearchService } from '../services/SearchService';
import { SettingsService } from '../services/SettingsService';
import i18next from 'i18next';
import FsBackend from 'i18next-fs-backend';
import i18nextMiddleware from 'i18next-http-middleware';
import path from 'path';
import { logger } from '../config/logger';
import { rateLimiter } from './middleware/rateLimiter';
import { config } from '../config';
import { OpenArchiverFeature } from '@open-archiver/types';
// Define the "plugin" interface
export interface ArchiverModule {
	initialize: (app: Express, authService: AuthService) => Promise<void>;
	name: OpenArchiverFeature;
}

export let authService: AuthService;

export async function createServer(modules: ArchiverModule[] = []): Promise<Express> {
	// Load environment variables
	dotenv.config();

	// --- Environment Variable Validation ---
	const { JWT_SECRET, JWT_EXPIRES_IN } = process.env;

	if (!JWT_SECRET || !JWT_EXPIRES_IN) {
		throw new Error(
			'Missing required environment variables for the backend: JWT_SECRET, JWT_EXPIRES_IN.'
		);
	}

	// --- Dependency Injection Setup ---
	const userService = new UserService();
	authService = new AuthService(userService, JWT_SECRET, JWT_EXPIRES_IN);
	const authController = new AuthController(authService, userService);
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

	// Configure the Meilisearch index on startup
	logger.info({}, 'Configuring email index...');
	await searchService.configureEmailIndex();

	const app = express();

	// --- CORS (inlined: single static allowed origin + credentials) ---
	const corsOrigin = process.env.APP_URL || 'http://localhost:3000';
	app.use((req: Request, res: Response, next: NextFunction) => {
		res.header('Access-Control-Allow-Origin', corsOrigin);
		res.header('Access-Control-Allow-Credentials', 'true');
		res.vary('Origin');
		if (req.method === 'OPTIONS') {
			res.header('Access-Control-Allow-Methods', 'GET,HEAD,PUT,PATCH,POST,DELETE');
			const requested = req.header('Access-Control-Request-Headers');
			if (requested) res.header('Access-Control-Allow-Headers', requested);
			return res.sendStatus(204);
		}
		next();
	});

	// Trust the proxy to get the real IP address of the client.
	// This is important for audit logging and security.
	app.set('trust proxy', true);

	// --- Routes ---
	const authRouter = createAuthRouter(authController);
	const ingestionRouter = createIngestionRouter(ingestionController, authService);
	const archivedEmailRouter = createArchivedEmailRouter(archivedEmailController, authService);
	const storageRouter = createStorageRouter(storageController, authService);
	const searchRouter = createSearchRouter(searchController, authService);
	const dashboardRouter = createDashboardRouter(authService);
	const contactsRouter = createContactsRouter(authService);
	const uploadRouter = createUploadRouter(authService);
	const userRouter = createUserRouter(authService);
	const settingsRouter = createSettingsRouter(authService);
	const apiKeyRouter = apiKeyRoutes(authService);
	const jobsRouter = createJobsRouter(authService);

	// Middleware for all other routes
	app.use((req, res, next) => {
		// exclude certain API endpoints from the rate limiter, for example status, system settings
		const excludedPatterns = [/^\/v\d+\/auth\/status$/, /^\/v\d+\/settings\/system$/];
		for (const pattern of excludedPatterns) {
			if (pattern.test(req.path)) {
				return next();
			}
		}
		rateLimiter(req, res, next);
	});
	app.use(express.json({ limit: '25mb' }));
	app.use(express.urlencoded({ extended: true, limit: '25mb' }));

	// i18n middleware
	app.use(i18nextMiddleware.handle(i18next));

	app.use(`/${config.api.version}/auth`, authRouter);
	app.use(`/${config.api.version}/upload`, uploadRouter);
	app.use(`/${config.api.version}/ingestion-sources`, ingestionRouter);
	app.use(`/${config.api.version}/archived-emails`, archivedEmailRouter);
	app.use(`/${config.api.version}/storage`, storageRouter);
	app.use(`/${config.api.version}/search`, searchRouter);
	app.use(`/${config.api.version}/dashboard`, dashboardRouter);
	app.use(`/${config.api.version}/contacts`, contactsRouter);
	app.use(`/${config.api.version}/users`, userRouter);
	app.use(`/${config.api.version}/settings`, settingsRouter);
	app.use(`/${config.api.version}/api-keys`, apiKeyRouter);
	app.use(`/${config.api.version}/jobs`, jobsRouter);

	// Load all provided extension modules
	for (const module of modules) {
		await module.initialize(app, authService);
		logger.info(`🏢 Enterprise module loaded: ${module.name}`);
	}
	app.get('/', (req, res) => {
		res.send('Backend is running!!');
	});
	logger.info('✅ Core OSS modules loaded.');

	return app;
}
