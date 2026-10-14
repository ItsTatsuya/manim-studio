"""Server-sent events: worker threads publish, every open window receives."""

from __future__ import annotations

import asyncio
import json
import threading

HEARTBEAT_SECONDS = 15
QUEUE_LIMIT = 2000
_CLOSED = object()


class EventBus:
    def __init__(self):
        self.lock = threading.Lock()
        self.subscribers: set[tuple[asyncio.AbstractEventLoop, asyncio.Queue]] = set()
        self.closed = False

    def publish(self, kind: str, payload: dict):
        message = json.dumps({"type": kind, **payload}, ensure_ascii=False)
        with self.lock:
            subscribers = list(self.subscribers)
        for loop, queue in subscribers:
            try:
                loop.call_soon_threadsafe(self._offer, loop, queue, message)
            except RuntimeError:  # The subscriber's event loop has closed.
                self._drop(loop, queue)

    def _offer(self, loop, queue, message):
        try:
            queue.put_nowait(message)
        except asyncio.QueueFull:
            # A stalled window resynchronizes by reconnecting.
            self._drop(loop, queue)
            queue.get_nowait()
            queue.put_nowait(_CLOSED)

    def _drop(self, loop, queue):
        with self.lock:
            self.subscribers.discard((loop, queue))

    def open(self):
        with self.lock:
            self.closed = False

    def close(self):
        """End every stream so the server can shut down promptly."""
        with self.lock:
            self.closed = True
            subscribers = list(self.subscribers)
            self.subscribers.clear()
        for loop, queue in subscribers:
            try:
                loop.call_soon_threadsafe(queue.put_nowait, _CLOSED)
            except (RuntimeError, asyncio.QueueFull):
                pass

    async def stream(self, hello: dict):
        loop = asyncio.get_running_loop()
        queue: asyncio.Queue = asyncio.Queue(QUEUE_LIMIT)
        with self.lock:
            if self.closed:
                return
            self.subscribers.add((loop, queue))
        try:
            yield f"data: {json.dumps({'type': 'hello', **hello})}\n\n"
            while True:
                try:
                    message = await asyncio.wait_for(queue.get(), HEARTBEAT_SECONDS)
                except TimeoutError:
                    yield ": keep-alive\n\n"
                    continue
                if message is _CLOSED:
                    return
                yield f"data: {message}\n\n"
        finally:
            self._drop(loop, queue)


BUS = EventBus()


def publish(kind: str, payload: dict):
    BUS.publish(kind, payload)
