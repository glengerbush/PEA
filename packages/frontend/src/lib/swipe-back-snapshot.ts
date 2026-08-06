export interface SwipeBackSnapshot {
	dataUrl: string;
	url: string;
	width: number;
	height: number;
	theme: string;
}

const MAX_SNAPSHOTS = 3;
const snapshots = new Map<string, SwipeBackSnapshot>();
let captureVersion = 0;

function themeKey(): string {
	return document.documentElement.classList.contains('dark') ? 'dark' : 'light';
}

function releaseSnapshot(snapshot: SwipeBackSnapshot | undefined): void {
	if (snapshot?.dataUrl.startsWith('blob:')) URL.revokeObjectURL(snapshot.dataUrl);
}

async function captureNativeSnapshot(): Promise<string | null> {
	if (window.location.protocol !== 'pea:') return null;
	const response = await fetch('/api/v1/native/snapshot');
	// macOS keeps the DOM renderer until its native capture path is implemented.
	if (response.status === 404) return null;
	if (!response.ok) throw new Error(await response.text());
	return URL.createObjectURL(await response.blob());
}

async function captureDomSnapshot(
	surface: HTMLElement,
	width: number,
	height: number
): Promise<string> {
	const { default: html2canvas } = await import('html2canvas-pro');
	const canvas = await html2canvas(surface, {
		backgroundColor: getComputedStyle(document.body).backgroundColor,
		width,
		height,
		windowWidth: width,
		windowHeight: height,
		scrollX: 0,
		scrollY: -window.scrollY,
		scale: 1,
		logging: false,
		useCORS: true,
		ignoreElements: (element) => element.hasAttribute('data-swipe-snapshot-exclude'),
		onclone: (clonedDocument) => {
			const clonedSurface = clonedDocument.querySelector<HTMLElement>('[data-swipe-surface]');
			if (!clonedSurface) return;
			clonedSurface.style.transform = 'none';
			clonedSurface.style.transition = 'none';
			clonedSurface.style.boxShadow = 'none';
		},
	});
	return canvas.toDataURL('image/png');
}

export async function captureSwipeBackSnapshot(surface: HTMLElement, url: string): Promise<void> {
	if (window.innerWidth <= 0 || window.innerHeight <= 0) return;

	const version = ++captureVersion;
	const width = window.innerWidth;
	const height = window.innerHeight;
	const theme = themeKey();
	await document.fonts.ready;
	const dataUrl =
		(await captureNativeSnapshot()) ?? (await captureDomSnapshot(surface, width, height));

	// A newer route, resize, or theme change superseded this render.
	if (
		version !== captureVersion ||
		width !== window.innerWidth ||
		height !== window.innerHeight ||
		theme !== themeKey()
	) {
		releaseSnapshot({ dataUrl, url, width, height, theme });
		return;
	}

	releaseSnapshot(snapshots.get(url));
	snapshots.delete(url);
	snapshots.set(url, {
		dataUrl,
		url,
		width,
		height,
		theme,
	});

	while (snapshots.size > MAX_SNAPSHOTS) {
		const oldest = snapshots.keys().next().value;
		if (oldest === undefined) break;
		releaseSnapshot(snapshots.get(oldest));
		snapshots.delete(oldest);
	}
}

export function getSwipeBackSnapshot(url: string): SwipeBackSnapshot | null {
	const snapshot = snapshots.get(url);
	if (
		!snapshot ||
		snapshot.width !== window.innerWidth ||
		snapshot.height !== window.innerHeight ||
		snapshot.theme !== themeKey()
	) {
		return null;
	}

	// Refresh its LRU position.
	snapshots.delete(url);
	snapshots.set(url, snapshot);
	return snapshot;
}

export function invalidateSwipeBackSnapshots(): void {
	captureVersion += 1;
	for (const snapshot of snapshots.values()) releaseSnapshot(snapshot);
	snapshots.clear();
}
