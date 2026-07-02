import { defineConfig } from 'vitepress';

export default defineConfig({
	head: [['link', { rel: 'icon', href: '/logo-sq.svg' }]],
	title: 'Open Archiver Docs',
	description: 'Documentation for the Open Archiver desktop app (fork).',
	themeConfig: {
		search: {
			provider: 'local',
		},
		logo: {
			src: '/logo-sq.svg',
		},
		nav: [
			{ text: 'Home', link: '/' },
			{ text: 'Github', link: 'https://github.com/glengerbush/OpenArchiver' },
		],
		sidebar: [
			{
				text: 'Guide',
				items: [{ text: 'Get Started', link: '/' }],
			},
			{
				text: 'API Reference',
				items: [
					{ text: 'Overview', link: '/api/' },
					{ text: 'Archived Email', link: '/api/archived-email' },
					{ text: 'Dashboard', link: '/api/dashboard' },
					{ text: 'Ingestion', link: '/api/ingestion' },
					{ text: 'Search', link: '/api/search' },
					{ text: 'Storage', link: '/api/storage' },
					{ text: 'Upload', link: '/api/upload' },
					{ text: 'Jobs', link: '/api/jobs' },
					{ text: 'Users', link: '/api/users' },
					{ text: 'Settings', link: '/api/settings' },
				],
			},
		],
	},
});
