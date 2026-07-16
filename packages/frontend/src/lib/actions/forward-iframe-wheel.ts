import type { SwipeWheelInput } from '$lib/stores/swipe-back.svelte';

type WheelHandler = ((event: SwipeWheelInput) => void) | undefined;

/** Forward wheel input out of an iframe, where it cannot bubble to the page. */
export function forwardIframeWheel(node: HTMLIFrameElement, handler: WheelHandler) {
	let currentHandler = handler;
	let frameWindow: Window | null = null;

	function forward(event: WheelEvent) {
		currentHandler?.({
			deltaX: event.deltaX,
			deltaY: event.deltaY,
			deltaMode: event.deltaMode,
			preventDefault: () => event.preventDefault(),
		});
	}

	function detach() {
		frameWindow?.removeEventListener('wheel', forward as EventListener);
		frameWindow = null;
	}

	function attach() {
		detach();
		try {
			frameWindow = node.contentWindow;
			frameWindow?.addEventListener('wheel', forward as EventListener, { passive: false });
		} catch {
			// Some browser-native viewers do not expose their inner window. The
			// outer page listener still handles input everywhere else.
			frameWindow = null;
		}
	}

	node.addEventListener('load', attach);
	attach();

	return {
		update(nextHandler: WheelHandler) {
			currentHandler = nextHandler;
			attach();
		},
		destroy() {
			node.removeEventListener('load', attach);
			detach();
		},
	};
}
