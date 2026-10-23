import { apiBaseUrl } from '#lib/config.js';
import type { Event } from './client';

/**
 * Every event type `GET /events` sends. The API sets each event's SSE `event:` field to its
 * type, so `onmessage` never fires and each type needs its own listener.
 */
export const EVENT_TYPES = [
	'app.connected',
	'app.lagged',
	'job.queued',
	'job.started',
	'job.retrying',
	'job.deferred',
	'job.succeeded',
	'job.failed',
	'queue.paused',
	'queue.resumed',
	'application.created',
	'application.updated',
	'application.deleted',
	'run.updated',
	'kill_switch.updated',
	'goal.updated',
	'data.deleted'
] as const satisfies readonly Event['type'][];

export interface EventHandlers {
	/** Called with each parsed event. */
	event: (event: Event) => void;
	/** Called when the stream errors. The browser keeps reconnecting on its own. */
	error: () => void;
}

/** The parts of `EventSource` this module uses, so tests can pass a fake. */
export interface EventSourceLike {
	addEventListener(type: string, listener: (event: MessageEvent) => void): void;
	onerror: ((event: globalThis.Event) => void) | null;
	close(): void;
}

export interface ConnectEventsOptions {
	url?: string;
	EventSource?: new (url: string) => EventSourceLike;
}

/** Opens the API's event stream and returns a function that closes it. */
export function connectEvents(
	handlers: EventHandlers,
	{ url = `${apiBaseUrl()}/events`, EventSource = globalThis.EventSource }: ConnectEventsOptions = {}
): () => void {
	const source = new EventSource(url);
	for (const type of EVENT_TYPES) {
		source.addEventListener(type, (message) => {
			let event: Event;
			try {
				event = JSON.parse(message.data) as Event;
			} catch {
				return;
			}
			handlers.event(event);
		});
	}
	source.onerror = () => handlers.error();
	return () => source.close();
}
