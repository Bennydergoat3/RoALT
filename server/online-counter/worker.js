// RoALT's online counter. Each running RoALT (with "Show how many people use RoALT" on) POSTs
// {"id": "<uuid made once per launch>"} to /ping every 2 minutes and gets {"online": N} back:
// how many distinct ids pinged in the last 5 minutes. Nothing is stored on disk.
import { DurableObject } from "cloudflare:workers";

/** An id not seen for this long stops counting. */
const TTL_MS = 5 * 60_000;
/** Most ids held at once, so junk pings can't grow the map without end. */
const MAX_IDS = 50_000;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method !== "POST" || url.pathname !== "/ping") return new Response("Not found", { status: 404 });
    let id;
    try {
      ({ id } = await request.json());
    } catch {
      return new Response("Bad request", { status: 400 });
    }
    if (typeof id !== "string" || !UUID.test(id)) return new Response("Bad request", { status: 400 });
    // One instance for everyone, so all pings land in the same map.
    const counter = env.COUNTER.get(env.COUNTER.idFromName("global"));
    return Response.json({ online: await counter.ping(id) });
  },
};

// shortcut: the ids live in memory only, so an eviction resets the count (it rebuilds within one
// ping interval), and anyone can inflate it with made-up ids. Add a per-IP cap if that gets abused.
export class Counter extends DurableObject {
  seen = new Map();

  ping(id) {
    const now = Date.now();
    // Re-insert so the map stays ordered oldest first; then expired ids are all at the front.
    this.seen.delete(id);
    this.seen.set(id, now);
    for (const [old, at] of this.seen) {
      if (now - at <= TTL_MS && this.seen.size <= MAX_IDS) break;
      this.seen.delete(old);
    }
    return this.seen.size;
  }
}
