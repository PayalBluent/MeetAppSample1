import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen, type Unlisten } from "@/lib/tauri";
import { qk } from "@/lib/query";
import type { Meeting, RecorderStatus } from "@/types";

/**
 * Bridges backend events into the TanStack Query cache. Mount once, high in the
 * tree. Works identically against the real Tauri event stream and the mock bus.
 */
export function useBackendEvents() {
  const qc = useQueryClient();

  useEffect(() => {
    const unlisteners: Unlisten[] = [];
    let cancelled = false;

    const register = async () => {
      const subs = await Promise.all([
        listen("recorder://status", (status: RecorderStatus) => {
          qc.setQueryData(qk.recorderStatus, status);
        }),

        listen("meeting://detected", () => {
          qc.invalidateQueries({ queryKey: qk.detected });
        }),

        listen("meeting://ended", () => {
          qc.invalidateQueries({ queryKey: qk.detected });
        }),

        listen("meeting://updated", (meeting: Meeting) => {
          qc.setQueryData(qk.meeting(meeting.id), meeting);
          qc.invalidateQueries({ queryKey: qk.meetings });
        }),

        listen("recorder://transcript", ({ meetingId, segment }) => {
          qc.setQueryData<Meeting | null | undefined>(
            qk.meeting(meetingId),
            (prev) =>
              prev
                ? { ...prev, transcript: [...prev.transcript, segment] }
                : prev,
          );
        }),
      ]);

      if (cancelled) subs.forEach((u) => u());
      else unlisteners.push(...subs);
    };

    void register();
    return () => {
      cancelled = true;
      unlisteners.forEach((u) => u());
    };
  }, [qc]);
}
