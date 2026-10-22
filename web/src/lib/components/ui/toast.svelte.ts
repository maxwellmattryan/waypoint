import type { ToastTone } from './styles';

export interface ToastOptions {
	description?: string;
	/** Default `neutral`. */
	tone?: ToastTone;
	/** Milliseconds before it dismisses itself. Default `5000`; `0` stays until dismissed. */
	duration?: number;
}

export interface ToastItem {
	id: number;
	title: string;
	description?: string;
	tone: ToastTone;
	duration: number;
}

/** The app-wide toast queue. `<Toaster />`, mounted once in the root layout, renders it. */
class Toasts {
	/** The visible toasts, oldest first. */
	items = $state<ToastItem[]>([]);

	#nextId = 1;
	#timers = new Map<number, ReturnType<typeof setTimeout>>();

	/** Shows a toast and returns its id. */
	show = ({
		title,
		description,
		tone = 'neutral',
		duration = 5000
	}: ToastOptions & { title: string }): number => {
		const id = this.#nextId++;
		this.items.push({ id, title, description, tone, duration });
		if (duration > 0) {
			this.#timers.set(
				id,
				setTimeout(() => this.dismiss(id), duration)
			);
		}
		return id;
	};

	success = (title: string, opts: Omit<ToastOptions, 'tone'> = {}): number =>
		this.show({ ...opts, title, tone: 'success' });

	error = (title: string, opts: Omit<ToastOptions, 'tone'> = {}): number =>
		this.show({ ...opts, title, tone: 'danger' });

	dismiss = (id: number): void => {
		clearTimeout(this.#timers.get(id));
		this.#timers.delete(id);
		this.items = this.items.filter((item) => item.id !== id);
	};

	/** Removes every toast. For tests. */
	clear = (): void => {
		for (const timer of this.#timers.values()) clearTimeout(timer);
		this.#timers.clear();
		this.items = [];
	};
}

export const toast = new Toasts();
