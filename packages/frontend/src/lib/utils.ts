import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
}

// --- Class-variance helper ----------------------------------------------------
// Covers the subset the UI wrappers use: a base string plus variant→class maps
// and default selections. Conflicts resolve through cn()'s twMerge, so callers
// can still pass an overriding `class`.
type VariantShape = Record<string, Record<string, string>>;

type TVConfig<V extends VariantShape> = {
	base?: string;
	variants?: V;
	defaultVariants?: { [K in keyof V]?: keyof V[K] };
};

type TVProps<V extends VariantShape> = { [K in keyof V]?: keyof V[K] } & {
	class?: ClassValue;
	className?: ClassValue;
};

export function tv<V extends VariantShape>(config: TVConfig<V>) {
	return (props: TVProps<V> = {}) => {
		const classes: ClassValue[] = [config.base];
		if (config.variants) {
			for (const key in config.variants) {
				const chosen = props[key] ?? config.defaultVariants?.[key];
				if (chosen != null) classes.push(config.variants[key][chosen as string]);
			}
		}
		return cn(...classes, props.class, props.className);
	};
}

/** The variant props a class-variance helper's result accepts (minus class/className). */
export type VariantProps<T extends (...args: never[]) => string> = Omit<
	NonNullable<Parameters<T>[0]>,
	'class' | 'className'
>;

export function formatBytes(bytes: number, decimals = 2) {
	if (bytes === 0) return '0 Bytes';

	const k = 1024;
	const dm = decimals < 0 ? 0 : decimals;
	const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'];

	const i = Math.floor(Math.log(bytes) / Math.log(k));

	return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WithoutChild<T> = T extends { child?: any } ? Omit<T, 'child'> : T;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WithoutChildren<T> = T extends { children?: any } ? Omit<T, 'children'> : T;
export type WithoutChildrenOrChild<T> = WithoutChildren<WithoutChild<T>>;
export type WithElementRef<T, U extends HTMLElement = HTMLElement> = T & { ref?: U | null };
