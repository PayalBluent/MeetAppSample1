/** Minimal typed pub/sub used by the mock backend to mirror Tauri's event system. */
export type Unlisten = () => void;

export class EventBus {
  private listeners = new Map<string, Set<(payload: unknown) => void>>();

  on(event: string, cb: (payload: unknown) => void): Unlisten {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(cb);
    return () => set!.delete(cb);
  }

  emit(event: string, payload: unknown): void {
    this.listeners.get(event)?.forEach((cb) => {
      try {
        cb(payload);
      } catch (err) {
        // Never let one listener break the emit loop.
        console.error(`[mock] listener for "${event}" threw`, err);
      }
    });
  }
}
