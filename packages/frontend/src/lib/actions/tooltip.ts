/**
 * Minimal hover/focus tooltip action. Shows a themed popover after a short
 * delay, positioned below (or above, if it would overflow) the target, and
 * cleans up on leave/blur/destroy. Content is plain text.
 */
export function tooltip(node: HTMLElement, content: string) {
	let tip: HTMLDivElement | null = null;
	let timer: ReturnType<typeof setTimeout> | null = null;
	let current = content;

	function place() {
		if (!tip) return;
		const anchor = node.getBoundingClientRect();
		const box = tip.getBoundingClientRect();
		let left = anchor.left + anchor.width / 2 - box.width / 2;
		left = Math.max(8, Math.min(left, window.innerWidth - box.width - 8));
		let top = anchor.bottom + 6;
		if (top + box.height > window.innerHeight - 8) top = anchor.top - box.height - 6;
		tip.style.left = `${left}px`;
		tip.style.top = `${top}px`;
	}

	function show() {
		if (tip || !current) return;
		tip = document.createElement('div');
		tip.textContent = current;
		tip.setAttribute('role', 'tooltip');
		tip.className =
			'bg-popover text-popover-foreground pointer-events-none fixed z-50 max-w-xs rounded-md border px-2 py-1 text-xs shadow-md';
		document.body.appendChild(tip);
		place();
	}

	function scheduleShow() {
		if (!timer) timer = setTimeout(show, 500);
	}

	function hide() {
		if (timer) {
			clearTimeout(timer);
			timer = null;
		}
		tip?.remove();
		tip = null;
	}

	node.addEventListener('mouseenter', scheduleShow);
	node.addEventListener('mouseleave', hide);
	node.addEventListener('focusin', show);
	node.addEventListener('focusout', hide);

	return {
		update(next: string) {
			current = next;
			if (tip) {
				tip.textContent = next;
				place();
			}
		},
		destroy() {
			hide();
			node.removeEventListener('mouseenter', scheduleShow);
			node.removeEventListener('mouseleave', hide);
			node.removeEventListener('focusin', show);
			node.removeEventListener('focusout', hide);
		},
	};
}
